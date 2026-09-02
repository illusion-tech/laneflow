//! 标识 v1（Identity v1）的编译器侧规范编码与唯一性登记。
//!
//! 本模块消费 `laneflow-static-contract` 冻结的登记元数据，但独立实现 envelope、字段
//! 校验和 BLAKE3-128 派生。编码器不接受“可选”或未登记字段：调用方必须一次提供实体
//! 种类要求的完整标签序列。完整规范前像仅在编译阶段的临时登记表中保留，用于区分
//! 重复身份与摘要碰撞；HIR/MIR 只携带有类型的 16 字节稳定标识。
//!
//! 后继独立制品验证器不得复用本模块。已知向量测试中的预言机也使用另一套字节组装
//! 代码，以免编译器编码错误同时污染期望值。

use std::collections::HashMap;

use laneflow_static_contract::{
    EntityKind, FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES, FieldEncoding, FieldTag,
    IDENTITY_ENCODING_VERSION, IDENTITY_MAGIC, STABLE_ID_DOMAIN_PREFIX, StableId128,
};

use crate::limits::CompileLimits;

#[cfg(test)]
use crate::SourceSpan;
use crate::source::external_token_violation;
use crate::{SourceLocation, SourceTextViolation};

/// 标识字段集合不能形成规范 Identity v1 前像的精确原因。
///
/// `position` 是实体字段序列中的零基位置，`tag`、`expected` 与 `actual` 均为登记表的
/// 原始 `u16` 代码。该类型进入结构化诊断；调用方不应依赖其显示文本。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum CanonicalIdentityViolation {
    /// 字段项数与实体种类登记的必需项数不同。
    FieldCountMismatch { expected: u16, actual: u64 },
    /// 字段位置出现未知或保留标签。
    UnknownFieldTag { position: u16, tag: u16 },
    /// 字段标签已登记，但不位于实体种类要求的当前位置。
    UnexpectedFieldTag {
        position: u16,
        expected: u16,
        actual: u16,
    },
    /// ASCII 字段违反外部标识 token 约束。
    InvalidAsciiField {
        tag: u16,
        violation: SourceTextViolation,
    },
    /// StableId128 字段不是恰好 16 字节。
    InvalidStableIdLength { tag: u16, actual: u64 },
    /// 单字段长度不能写入 Identity v1 的 `u32_le(byte_length)`。
    FieldByteLengthOverflow { tag: u16, actual: u64 },
    /// 完整规范身份字节数不能由当前目标平台的 `usize` 表示。
    CanonicalByteLengthOverflow { actual: u64 },
    /// 种类是登记表保留空位，不得编码。
    UnconstructibleKind { kind: u16 },
}

/// 编译器内部尚未编码的一个 Identity v1 字段。
#[derive(Clone, Copy)]
pub(crate) struct IdentityFieldInput<'a> {
    tag: u16,
    bytes: &'a [u8],
}

impl<'a> IdentityFieldInput<'a> {
    /// 使用已登记标签构造字段；规范顺序仍由编码器对照实体登记表验证。
    pub(crate) const fn new(tag: FieldTag, bytes: &'a [u8]) -> Self {
        Self {
            tag: tag.code(),
            bytes,
        }
    }

    #[cfg(test)]
    const fn from_raw(tag: u16, bytes: &'a [u8]) -> Self {
        Self { tag, bytes }
    }
}

/// 已编码的完整规范身份及其派生稳定标识。
#[derive(Debug)]
pub(crate) struct EncodedCanonicalIdentity {
    kind: EntityKind,
    canonical_bytes: Box<[u8]>,
    stable_id: StableId128,
}

impl EncodedCanonicalIdentity {
    /// 返回 envelope 中登记的实体种类。
    pub(crate) const fn kind(&self) -> EntityKind {
        self.kind
    }

    /// 返回不含 BLAKE3 域分离前缀的完整规范前像。
    pub(crate) fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }

    /// 返回规范前像经 BLAKE3 派生后的前 16 字节。
    pub(crate) const fn stable_id(&self) -> StableId128 {
        self.stable_id
    }
}

/// 为「`AuthoringNamespaceId` + 单一 ASCII 本地键」实体种类派生 Identity v1 稳定标识。
///
/// prepare 绑定用它把编制字符串对上已安装 `SharedIdentityIndex`。字段顺序与种类登记表
/// 一致，由本函数按 `EntityKind::required_tags` 组装。独立制品验证器不得把本函数当作
/// 第二套已知向量预言机。
pub fn derive_canonical_stable_id_v1(
    kind: EntityKind,
    authoring_namespace_id: &str,
    local_key: &str,
    limits: &CompileLimits,
) -> Result<StableId128, CanonicalIdentityViolation> {
    if !kind.is_constructible() {
        return Err(CanonicalIdentityViolation::UnconstructibleKind { kind: kind.code() });
    }
    let tags = kind.required_tags();
    if tags.len() != 2 {
        return Err(CanonicalIdentityViolation::FieldCountMismatch {
            expected: u16::try_from(tags.len()).expect("Identity v1 field count must fit u16"),
            actual: 2,
        });
    }
    if tags[0] != FieldTag::AuthoringNamespaceId {
        return Err(CanonicalIdentityViolation::UnexpectedFieldTag {
            position: 0,
            expected: FieldTag::AuthoringNamespaceId.code(),
            actual: tags[0].code(),
        });
    }
    let fields = [
        IdentityFieldInput::new(tags[0], authoring_namespace_id.as_bytes()),
        IdentityFieldInput::new(tags[1], local_key.as_bytes()),
    ];
    Ok(encode_canonical_identity(kind, &fields, limits.identity_ascii_bytes_limit())?.stable_id())
}

/// 编码并派生一个完整 Identity v1 身份。
///
/// 哈希器和输出缓冲区在同一次顺序写入中更新，避免为
/// `domain_prefix || canonical_bytes` 再构造一份拼接缓冲区。返回前会验证字段集合与登记
/// 表完全一致，因此成功值可以安全进入唯一性登记表。不可构造种类失败关闭。
pub(crate) fn encode_canonical_identity(
    kind: EntityKind,
    fields: &[IdentityFieldInput<'_>],
    max_single_string_bytes: u64,
) -> Result<EncodedCanonicalIdentity, CanonicalIdentityViolation> {
    if !kind.is_constructible() {
        return Err(CanonicalIdentityViolation::UnconstructibleKind { kind: kind.code() });
    }
    let required_tags = kind.required_tags();
    if fields.len() != required_tags.len() {
        return Err(CanonicalIdentityViolation::FieldCountMismatch {
            expected: u16::try_from(required_tags.len())
                .expect("Identity v1 field count must fit u16"),
            actual: u64::try_from(fields.len()).unwrap_or(u64::MAX),
        });
    }

    let max_identity_ascii_bytes =
        max_single_string_bytes.min(FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES);
    let mut encoded_length = 10_u64;
    for (position, (field, expected)) in fields.iter().zip(required_tags).enumerate() {
        let position = u16::try_from(position).expect("Identity v1 field position must fit u16");
        let Some(actual_tag) = FieldTag::from_code(field.tag) else {
            return Err(CanonicalIdentityViolation::UnknownFieldTag {
                position,
                tag: field.tag,
            });
        };
        if actual_tag != *expected {
            return Err(CanonicalIdentityViolation::UnexpectedFieldTag {
                position,
                expected: expected.code(),
                actual: field.tag,
            });
        }

        let field_length = u64::try_from(field.bytes.len()).unwrap_or(u64::MAX);
        if field_length > u64::from(u32::MAX) {
            return Err(CanonicalIdentityViolation::FieldByteLengthOverflow {
                tag: field.tag,
                actual: field_length,
            });
        }
        match actual_tag.encoding() {
            FieldEncoding::Ascii => {
                let violation = field
                    .bytes
                    .iter()
                    .position(|byte| !byte.is_ascii())
                    .map(|byte_index| SourceTextViolation::NonAscii {
                        byte_index: u64::try_from(byte_index).unwrap_or(u64::MAX),
                    })
                    .or_else(|| {
                        // 全 ASCII 字节必然是合法 UTF-8；文本规则仍由来源层的同一 SSOT
                        // 执行，防止身份编码器与前端对 token 合法集合产生分歧。
                        let text = std::str::from_utf8(field.bytes)
                            .expect("ASCII bytes must always be valid UTF-8");
                        external_token_violation(text, max_identity_ascii_bytes)
                    });
                if let Some(violation) = violation {
                    return Err(CanonicalIdentityViolation::InvalidAsciiField {
                        tag: field.tag,
                        violation,
                    });
                }
            }
            FieldEncoding::StableId128 if field.bytes.len() != 16 => {
                return Err(CanonicalIdentityViolation::InvalidStableIdLength {
                    tag: field.tag,
                    actual: field_length,
                });
            }
            FieldEncoding::StableId128 => {}
        }
        encoded_length = encoded_length
            .saturating_add(6)
            .saturating_add(field_length);
    }

    let capacity = usize::try_from(encoded_length).map_err(|_| {
        CanonicalIdentityViolation::CanonicalByteLengthOverflow {
            actual: encoded_length,
        }
    })?;
    let mut canonical_bytes = Vec::with_capacity(capacity);
    let mut hasher = blake3::Hasher::new();
    hasher.update(STABLE_ID_DOMAIN_PREFIX);

    append(&mut canonical_bytes, &mut hasher, &IDENTITY_MAGIC);
    append(
        &mut canonical_bytes,
        &mut hasher,
        &IDENTITY_ENCODING_VERSION.to_le_bytes(),
    );
    append(
        &mut canonical_bytes,
        &mut hasher,
        &kind.code().to_le_bytes(),
    );
    append(
        &mut canonical_bytes,
        &mut hasher,
        &u16::try_from(fields.len())
            .expect("validated Identity v1 field count must fit u16")
            .to_le_bytes(),
    );
    for field in fields {
        append(&mut canonical_bytes, &mut hasher, &field.tag.to_le_bytes());
        append(
            &mut canonical_bytes,
            &mut hasher,
            &u32::try_from(field.bytes.len())
                .expect("validated Identity v1 field length must fit u32")
                .to_le_bytes(),
        );
        append(&mut canonical_bytes, &mut hasher, field.bytes);
    }
    debug_assert_eq!(canonical_bytes.len(), capacity);

    let digest = hasher.finalize();
    let mut stable_id = [0_u8; 16];
    stable_id.copy_from_slice(&digest.as_bytes()[..16]);
    Ok(EncodedCanonicalIdentity {
        kind,
        canonical_bytes: canonical_bytes.into_boxed_slice(),
        stable_id: StableId128::from_bytes(stable_id),
    })
}

fn append(output: &mut Vec<u8>, hasher: &mut blake3::Hasher, bytes: &[u8]) {
    output.extend_from_slice(bytes);
    hasher.update(bytes);
}

/// 临时身份登记项；完整前像使摘要相等时仍能作权威比较。
pub(crate) struct RegisteredCanonicalIdentity {
    canonical_bytes: Box<[u8]>,
    owning_span: SourceLocation,
}

/// 单次编译内的稳定标识唯一性登记表。
///
/// `HashMap` 仅用于按摘要查找，不参与规范输出遍历。登记表在身份闭合后释放，不能进入
/// HIR/MIR 或制品；摘要冲突不得通过盐、后缀或重编号恢复。
#[derive(Default)]
pub(crate) struct IdentityRegistry {
    by_stable_id: HashMap<StableId128, RegisteredCanonicalIdentity>,
}

#[derive(Debug)]
pub(crate) enum IdentityRegistrationError {
    Duplicate { existing_span: SourceLocation },
    DigestCollision { existing_span: SourceLocation },
}

impl IdentityRegistry {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            by_stable_id: HashMap::with_capacity(capacity),
        }
    }

    /// 登记完整身份；成功后同一摘要的后续项必须与此处保存的前像比较。
    pub(crate) fn register(
        &mut self,
        identity: &EncodedCanonicalIdentity,
        owning_span: &SourceLocation,
    ) -> Result<(), IdentityRegistrationError> {
        self.register_prederived(identity.stable_id, identity.canonical_bytes(), owning_span)
    }

    fn register_prederived(
        &mut self,
        stable_id: StableId128,
        canonical_bytes: &[u8],
        owning_span: &SourceLocation,
    ) -> Result<(), IdentityRegistrationError> {
        if let Some(existing) = self.by_stable_id.get(&stable_id) {
            return if existing.canonical_bytes.as_ref() == canonical_bytes {
                Err(IdentityRegistrationError::Duplicate {
                    existing_span: existing.owning_span.clone(),
                })
            } else {
                Err(IdentityRegistrationError::DigestCollision {
                    existing_span: existing.owning_span.clone(),
                })
            };
        }
        self.by_stable_id.insert(
            stable_id,
            RegisteredCanonicalIdentity {
                canonical_bytes: canonical_bytes.into(),
                owning_span: owning_span.clone(),
            },
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use laneflow_static_contract::{FieldTag, LaneEdgeId};

    const STRING_LIMIT: u64 = 53;

    fn lane_edge_fields<'a>(namespace: &'a [u8], key: &'a [u8]) -> [IdentityFieldInput<'a>; 2] {
        [
            IdentityFieldInput::new(FieldTag::AuthoringNamespaceId, namespace),
            IdentityFieldInput::new(FieldTag::LaneEdgeKey, key),
        ]
    }

    fn known_vector_fields(kind: EntityKind) -> Vec<(u16, Vec<u8>)> {
        kind.required_tags()
            .iter()
            .map(|tag| {
                let bytes = match tag.encoding() {
                    FieldEncoding::Ascii if *tag == FieldTag::AuthoringNamespaceId => {
                        b"vector/v1".to_vec()
                    }
                    FieldEncoding::Ascii => format!("field-{}", tag.code()).into_bytes(),
                    FieldEncoding::StableId128 => vec![u8::try_from(tag.code()).unwrap(); 16],
                };
                (tag.code(), bytes)
            })
            .collect()
    }

    fn independent_oracle(kind_code: u16, fields: &[(u16, Vec<u8>)]) -> (Vec<u8>, [u8; 16]) {
        // 预言机有意不调用生产 `append`/`encode_canonical_identity`，也不读取生产
        // envelope 常量；只有 BLAKE3 primitive 与登记表元数据允许共享。
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"LFID");
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&kind_code.to_le_bytes());
        bytes.extend_from_slice(&u16::try_from(fields.len()).unwrap().to_le_bytes());
        for (tag, value) in fields {
            bytes.extend_from_slice(&tag.to_le_bytes());
            bytes.extend_from_slice(&u32::try_from(value.len()).unwrap().to_le_bytes());
            bytes.extend_from_slice(value);
        }
        let mut hash_input = b"laneflow.stable-id.v1\0".to_vec();
        hash_input.extend_from_slice(&bytes);
        let digest = blake3::hash(&hash_input);
        let mut stable_id = [0_u8; 16];
        stable_id.copy_from_slice(&digest.as_bytes()[..16]);
        (bytes, stable_id)
    }

    fn hexadecimal(bytes: &[u8]) -> String {
        use std::fmt::Write as _;

        let mut output = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            write!(&mut output, "{byte:02x}").unwrap();
        }
        output
    }

    /// Registry revision 4 全部可构造种类的规范字节与 BLAKE3-128 已知向量。
    ///
    /// 字段生成规则和文件列语义记录在向量文件头；该文件是独立预言机可复用的期望
    /// 事实，不能在测试运行时从生产编码器重写。
    const KNOWN_VECTORS: &str = include_str!("../tests/identity-v1-known-vectors.txt");

    #[test]
    fn all_registry_kinds_match_frozen_independent_vectors() {
        let vectors = KNOWN_VECTORS
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| {
                let mut columns = line.split_ascii_whitespace();
                let kind_code = columns.next().unwrap().parse::<u16>().unwrap();
                let canonical_hex = columns.next().unwrap();
                let stable_id_hex = columns.next().unwrap();
                assert!(columns.next().is_none(), "unexpected vector column: {line}");
                (kind_code, canonical_hex, stable_id_hex)
            })
            .collect::<Vec<_>>();
        assert_eq!(vectors.len(), EntityKind::ALL.len());

        for ((kind_code, expected_bytes, expected_id), registered_kind) in
            vectors.into_iter().zip(EntityKind::ALL)
        {
            let Some(kind) = EntityKind::from_code(kind_code) else {
                assert!(
                    !registered_kind.is_constructible(),
                    "known vector kind must be registered"
                );
                assert_eq!(kind_code, registered_kind.code());
                assert_eq!(expected_bytes, "-");
                assert_eq!(expected_id, "-");
                assert_eq!(
                    encode_canonical_identity(registered_kind, &[], STRING_LIMIT).unwrap_err(),
                    CanonicalIdentityViolation::UnconstructibleKind { kind: kind_code }
                );
                continue;
            };
            assert_eq!(kind, registered_kind);
            let owned_fields = known_vector_fields(kind);
            let compiler_fields = owned_fields
                .iter()
                .map(|(tag, bytes)| IdentityFieldInput::from_raw(*tag, bytes))
                .collect::<Vec<_>>();
            let compiler = encode_canonical_identity(kind, &compiler_fields, STRING_LIMIT).unwrap();
            let (oracle_bytes, oracle_id) = independent_oracle(kind.code(), &owned_fields);

            assert_eq!(hexadecimal(&oracle_bytes), expected_bytes, "{kind:?} bytes");
            assert_eq!(hexadecimal(&oracle_id), expected_id, "{kind:?} id");
            assert_eq!(compiler.canonical_bytes(), oracle_bytes, "{kind:?}");
            assert_eq!(compiler.stable_id().as_bytes(), &oracle_id, "{kind:?}");
        }
    }

    #[test]
    fn policy_identity_uses_namespace_and_policy_key() {
        let limits = CompileLimits::single_network_1m_v2();
        let derive = |namespace, key| {
            derive_canonical_stable_id_v1(EntityKind::RightOfWayPolicySet, namespace, key, &limits)
                .unwrap()
        };
        let policy = derive("vector/v1", "field-35");
        let expected = KNOWN_VECTORS
            .lines()
            .find(|line| line.starts_with("24 "))
            .unwrap()
            .split_ascii_whitespace()
            .nth(2)
            .unwrap();
        assert_eq!(hexadecimal(policy.as_bytes()), expected);
        assert_ne!(derive("vector/other", "field-35"), policy);
        assert_ne!(derive("vector/v1", "other-key"), policy);
        assert_ne!(
            derive_canonical_stable_id_v1(EntityKind::LaneEdge, "vector/v1", "field-35", &limits)
                .unwrap(),
            policy
        );
        assert!(matches!(
            derive_canonical_stable_id_v1(
                EntityKind::RightOfWayPolicySet,
                "vector/v1",
                &"a".repeat(54),
                &limits,
            ),
            Err(CanonicalIdentityViolation::InvalidAsciiField {
                tag: 35,
                violation: SourceTextViolation::TooLong {
                    limit: 53,
                    observed: 54
                },
            })
        ));
    }

    #[test]
    fn public_namespaced_key_derivation_matches_encoder() {
        let limits = CompileLimits::p100_initial_v1();
        let encoded = encode_canonical_identity(
            EntityKind::LaneEdge,
            &lane_edge_fields(b"city/vector", b"edge-a"),
            limits.max_single_string_bytes(),
        )
        .unwrap();
        let derived =
            derive_canonical_stable_id_v1(EntityKind::LaneEdge, "city/vector", "edge-a", &limits)
                .unwrap();
        assert_eq!(derived, encoded.stable_id());
        assert!(
            derive_canonical_stable_id_v1(EntityKind::Movement, "city/vector", "m", &limits)
                .is_err()
        );
    }

    #[test]
    fn larger_profile_string_limit_cannot_widen_identity_ascii_fields() {
        let at_bound = "a".repeat(53);
        let over_bound = "a".repeat(54);
        let limits = CompileLimits::single_network_1m_v2();

        assert!(
            derive_canonical_stable_id_v1(EntityKind::LaneEdge, &at_bound, &at_bound, &limits,)
                .is_ok()
        );
        assert_eq!(
            derive_canonical_stable_id_v1(EntityKind::LaneEdge, &at_bound, &over_bound, &limits,)
                .unwrap_err(),
            CanonicalIdentityViolation::InvalidAsciiField {
                tag: FieldTag::LaneEdgeKey.code(),
                violation: SourceTextViolation::TooLong {
                    limit: 53,
                    observed: 54,
                },
            }
        );
        assert!(matches!(
            encode_canonical_identity(
                EntityKind::LaneEdge,
                &lane_edge_fields(at_bound.as_bytes(), over_bound.as_bytes()),
                4_096,
            ),
            Err(CanonicalIdentityViolation::InvalidAsciiField {
                violation: SourceTextViolation::TooLong {
                    limit: 53,
                    observed: 54,
                },
                ..
            })
        ));
    }

    #[test]
    fn lane_edge_encoding_uses_exact_v1_envelope_and_domain_separation() {
        let identity = encode_canonical_identity(
            EntityKind::LaneEdge,
            &lane_edge_fields(b"city/vector", b"edge-a"),
            STRING_LIMIT,
        )
        .unwrap();

        let expected = [
            b'L', b'F', b'I', b'D', 1, 0, 4, 0, 2, 0, 1, 0, 11, 0, 0, 0, b'c', b'i', b't', b'y',
            b'/', b'v', b'e', b'c', b't', b'o', b'r', 5, 0, 6, 0, 0, 0, b'e', b'd', b'g', b'e',
            b'-', b'a',
        ];
        assert_eq!(identity.kind(), EntityKind::LaneEdge);
        assert_eq!(identity.canonical_bytes(), expected);

        let mut independent_input = Vec::from(STABLE_ID_DOMAIN_PREFIX);
        independent_input.extend_from_slice(&expected);
        let digest = blake3::hash(&independent_input);
        let mut expected_id = [0_u8; 16];
        expected_id.copy_from_slice(&digest.as_bytes()[..16]);
        assert_eq!(identity.stable_id(), StableId128::from_bytes(expected_id));
        assert_eq!(
            LaneEdgeId::from_untyped(identity.stable_id()).to_string(),
            format!("lfid1_lane-edge_{:x}", identity.stable_id())
        );
    }

    #[test]
    fn encoder_rejects_missing_duplicate_unknown_and_out_of_order_tags() {
        let missing = [IdentityFieldInput::new(
            FieldTag::AuthoringNamespaceId,
            b"city/vector",
        )];
        assert_eq!(
            encode_canonical_identity(EntityKind::LaneEdge, &missing, STRING_LIMIT).unwrap_err(),
            CanonicalIdentityViolation::FieldCountMismatch {
                expected: 2,
                actual: 1,
            }
        );

        let duplicate = [
            IdentityFieldInput::new(FieldTag::AuthoringNamespaceId, b"city/vector"),
            IdentityFieldInput::new(FieldTag::AuthoringNamespaceId, b"edge-a"),
        ];
        assert_eq!(
            encode_canonical_identity(EntityKind::LaneEdge, &duplicate, STRING_LIMIT).unwrap_err(),
            CanonicalIdentityViolation::UnexpectedFieldTag {
                position: 1,
                expected: 5,
                actual: 1,
            }
        );

        let unknown = [
            IdentityFieldInput::new(FieldTag::AuthoringNamespaceId, b"city/vector"),
            IdentityFieldInput::from_raw(36, b"edge-a"),
        ];
        assert_eq!(
            encode_canonical_identity(EntityKind::LaneEdge, &unknown, STRING_LIMIT).unwrap_err(),
            CanonicalIdentityViolation::UnknownFieldTag {
                position: 1,
                tag: 36,
            }
        );

        let out_of_order = [
            IdentityFieldInput::new(FieldTag::LaneEdgeKey, b"edge-a"),
            IdentityFieldInput::new(FieldTag::AuthoringNamespaceId, b"city/vector"),
        ];
        assert_eq!(
            encode_canonical_identity(EntityKind::LaneEdge, &out_of_order, STRING_LIMIT)
                .unwrap_err(),
            CanonicalIdentityViolation::UnexpectedFieldTag {
                position: 0,
                expected: 1,
                actual: 5,
            }
        );

        let inserted = [
            IdentityFieldInput::new(FieldTag::AuthoringNamespaceId, b"city/vector"),
            IdentityFieldInput::new(FieldTag::LaneEdgeKey, b"edge-a"),
            IdentityFieldInput::new(FieldTag::JunctionKey, b"not-a-role"),
        ];
        assert_eq!(
            encode_canonical_identity(EntityKind::LaneEdge, &inserted, STRING_LIMIT).unwrap_err(),
            CanonicalIdentityViolation::FieldCountMismatch {
                expected: 2,
                actual: 3,
            }
        );
    }

    #[test]
    fn identity_changes_only_when_an_authoritative_field_changes() {
        let baseline = encode_canonical_identity(
            EntityKind::LaneEdge,
            &lane_edge_fields(b"city/vector", b"edge-a"),
            STRING_LIMIT,
        )
        .unwrap();
        let changed_namespace = encode_canonical_identity(
            EntityKind::LaneEdge,
            &lane_edge_fields(b"city/other", b"edge-a"),
            STRING_LIMIT,
        )
        .unwrap();
        let changed_key = encode_canonical_identity(
            EntityKind::LaneEdge,
            &lane_edge_fields(b"city/vector", b"edge-b"),
            STRING_LIMIT,
        )
        .unwrap();

        assert_ne!(
            baseline.canonical_bytes(),
            changed_namespace.canonical_bytes()
        );
        assert_ne!(baseline.stable_id(), changed_namespace.stable_id());
        assert_ne!(baseline.canonical_bytes(), changed_key.canonical_bytes());
        assert_ne!(baseline.stable_id(), changed_key.stable_id());
    }

    #[test]
    fn encoder_rejects_invalid_ascii_and_wrong_stable_id_length() {
        let invalid_ascii = lane_edge_fields(b"city/vector", &[0xff]);
        assert_eq!(
            encode_canonical_identity(EntityKind::LaneEdge, &invalid_ascii, STRING_LIMIT)
                .unwrap_err(),
            CanonicalIdentityViolation::InvalidAsciiField {
                tag: 5,
                violation: SourceTextViolation::NonAscii { byte_index: 0 },
            }
        );

        let wrong_parent = [
            IdentityFieldInput::new(FieldTag::AuthoringNamespaceId, b"city/vector"),
            IdentityFieldInput::new(FieldTag::SectionKey, b"section-a"),
            IdentityFieldInput::new(FieldTag::RoadCorridorStableId, &[0x11; 15]),
        ];
        assert_eq!(
            encode_canonical_identity(EntityKind::RoadSection, &wrong_parent, STRING_LIMIT)
                .unwrap_err(),
            CanonicalIdentityViolation::InvalidStableIdLength {
                tag: 33,
                actual: 15
            }
        );
    }

    #[test]
    fn new_registry_kinds_require_their_complete_identity_fields() {
        assert_eq!(
            encode_canonical_identity(EntityKind::ConflictZone, &[], STRING_LIMIT).unwrap_err(),
            CanonicalIdentityViolation::FieldCountMismatch {
                expected: 3,
                actual: 0
            }
        );
        let limits = CompileLimits::p100_initial_v1();
        assert_eq!(
            derive_canonical_stable_id_v1(EntityKind::ParticipantStream, "city/x", "k", &limits)
                .unwrap_err(),
            CanonicalIdentityViolation::FieldCountMismatch {
                expected: 3,
                actual: 2
            }
        );
    }

    #[test]
    fn registry_distinguishes_duplicate_identity_from_digest_collision() {
        let span: SourceLocation = SourceSpan::point("vector".into(), 1, 1).into();
        let other_span: SourceLocation = SourceSpan::point("vector".into(), 2, 1).into();
        let stable_id = StableId128::from_bytes([0x42; 16]);
        let mut registry = IdentityRegistry::with_capacity(2);

        registry
            .register_prederived(stable_id, b"same", &span)
            .unwrap();
        assert!(matches!(
            registry.register_prederived(stable_id, b"same", &other_span),
            Err(IdentityRegistrationError::Duplicate { existing_span })
                if existing_span == span
        ));
        assert!(matches!(
            registry.register_prederived(stable_id, b"different", &other_span),
            Err(IdentityRegistrationError::DigestCollision { existing_span })
                if existing_span == span
        ));
    }
}
