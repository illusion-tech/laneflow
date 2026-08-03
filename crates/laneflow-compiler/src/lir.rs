//! 中层中间表示（MIR）到已验证规范低层中间表示（Canonical LIR）的冻结阶段。
//!
//! 本模块只实现 #292 首个 `LaneEdge` 纵向切片：稳定实体按完整 Identity v1 前像字节
//! 排序，表下标冻结为 [`LaneEdgeOrdinal`]，连接目标全部改写为同一 LIR 实例的有类型
//! 序号。身份字段和值分别进入连续表与共享字节池；来源位置留给同次编译的源映射伴随
//! 数据，不进入 LIR 或语义摘要。
//!
//! `LirUnit` 仍是 crate 私有阶段结果。它不是可移植规范制品或静态镜像 ABI；本模块的
//! 语义摘要只用于验证干净编译的确定性，不能冒充后继制品摘要或路网修订摘要。

use core::cmp::Ordering;

use laneflow_static_contract::{EntityKind, FieldTag, LaneEdgeId, LaneEdgeOrdinal};

use crate::arena::{ArenaKeyOverflow, TableRange};
use crate::diagnostic::DiagnosticCollector;
use crate::mir::{MirLaneEdgeKey, MirUnit};
use crate::{CompilationUnit, CompileLimitDimension, Diagnostic, DiagnosticBundle, SourceSpan};

/// 与公开制品版本轴无关的编译器私有摘要域。
const LIR_SEMANTIC_DIGEST_DOMAIN: &[u8] = b"LANEFLOW-COMPILER-LIR-SEMANTIC-V1\0";
/// `ordinal + stable_id + identity_range + length + speed + successor_range`。
const LIR_LANE_EDGE_LOGICAL_BYTES: u64 = 4 + 16 + 8 + 8 + 8 + 8;
/// `field_tag + value_range`；表归属已经给出实体种类，不在每项重复编码。
const LIR_IDENTITY_FIELD_LOGICAL_BYTES: u64 = 2 + 8;
const LIR_SUCCESSOR_LOGICAL_BYTES: u64 = 4;
const LIR_SEMANTIC_DIGEST_BYTES: u64 = 32;

/// 一项规范身份字段；值位于 `LirUnit::identity_field_bytes` 的连续区间。
pub(crate) struct LirIdentityField {
    /// Identity v1 登记标签；同一实体内严格按登记顺序保存。
    pub(crate) tag: FieldTag,
    /// 字段原始规范字节，不包含标签和长度前缀。
    pub(crate) value_bytes: TableRange<u8>,
}

/// 已冻结的车道图边静态语义。
pub(crate) struct LirLaneEdge {
    /// 此记录在当前 `lane_edges` 表中的有类型逻辑序号。
    pub(crate) ordinal: LaneEdgeOrdinal,
    /// 由同一记录的完整 Identity v1 字段前像派生的稳定标识。
    pub(crate) stable_id: LaneEdgeId,
    /// 此实体在 `identity_fields` 中的完整、规范有序字段区间。
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    /// 交通权威长度，单位为米；输入阶段已证明有限且严格大于零。
    pub(crate) length_meters: f64,
    /// 基础道路限速，单位为米每秒；输入阶段已证明有限且严格大于零。
    pub(crate) speed_limit_meters_per_second: f64,
    /// 按领域顺序保存的下游边序号区间。
    pub(crate) successors: TableRange<LaneEdgeOrdinal>,
}

/// 当前纵向切片冻结出的连续、目标布局中立 LIR 表。
///
/// 每条边的 `ordinal` 必须等于其切片下标；全部身份字段区间和连接区间均落在本实例的
/// 对应平面表内。`controlled_live_bytes` 只统计成功返回后由本结果持有的请求字节，不含
/// 已释放的 MIR 或冻结暂存区。
pub(crate) struct LirUnit {
    pub(crate) lane_edges: Box<[LirLaneEdge]>,
    pub(crate) lane_edge_successors: Box<[LaneEdgeOrdinal]>,
    pub(crate) identity_fields: Box<[LirIdentityField]>,
    pub(crate) identity_field_bytes: Box<[u8]>,
    pub(crate) semantic_digest: [u8; 32],
    pub(crate) lir_record_count: u64,
    pub(crate) output_bytes: u64,
    pub(crate) controlled_live_bytes: u64,
}

/// 将全部 MIR 引用重映射到规范 LIR 序号，并原子冻结连续只读表。
///
/// 排序键是 Identity v1 完整前像的逐字节顺序，而不是 `StableId128` 或普通字符串
/// 顺序。当前实体种类固定为 `LaneEdge`，所以比较器只需比较两个登记字段的编码片段；
/// 字段长度采用 `u32_le`，与规范编码器保持一致。
///
/// # Errors
///
/// 当 LIR 记录数、阶段暂存字节、输出字节、编译器控制存续字节或有类型 `u32` 边界超过
/// 所选资源配置档时，返回结构化资源诊断且不返回部分 LIR。
pub(crate) fn freeze_lir(
    unit: &CompilationUnit,
    mir: &MirUnit,
) -> Result<LirUnit, DiagnosticBundle> {
    let lane_edge_count = u64::try_from(mir.lane_edges.len()).unwrap_or(u64::MAX);
    let successor_count = u64::try_from(mir.lane_edge_connections.len()).unwrap_or(u64::MAX);
    // Identity 字段出现项有独立资源维度；LIR record 指标计实体行和关系出现行，与 MIR
    // 当前纵向切片的计数对象保持一致。
    let lir_record_count = lane_edge_count.saturating_add(successor_count);
    let identity_field_count = lane_edge_count.saturating_mul(2);
    let identity_field_byte_count = mir.lane_edges.iter().fold(0_u64, |total, edge| {
        let namespace = &mir.modules[edge.module.index()].authoring_namespace_id;
        total
            .saturating_add(u64::try_from(namespace.len()).unwrap_or(u64::MAX))
            .saturating_add(u64::try_from(edge.stable_key.len()).unwrap_or(u64::MAX))
    });

    // 排序序列与 MIR→LIR 映射各只保存一个有类型 `u32` 值；完整身份前像通过借用 MIR
    // 字段作分段比较，避免为排序再复制一份变长编码。
    let stage_scratch_bytes = requested_bytes::<MirLaneEdgeKey>(lane_edge_count)
        .saturating_add(requested_bytes::<LaneEdgeOrdinal>(lane_edge_count));
    // OutputBytes 使用设计冻结的目标布局中立字段宽度，不能把 Rust struct padding 或
    // 当前平台对齐冒充规范输出量；受控存续内存则按真实堆容量请求单独计算。
    let output_bytes = lane_edge_count
        .saturating_mul(LIR_LANE_EDGE_LOGICAL_BYTES)
        .saturating_add(successor_count.saturating_mul(LIR_SUCCESSOR_LOGICAL_BYTES))
        .saturating_add(identity_field_count.saturating_mul(LIR_IDENTITY_FIELD_LOGICAL_BYTES))
        .saturating_add(identity_field_byte_count)
        .saturating_add(LIR_SEMANTIC_DIGEST_BYTES);
    let output_owned_bytes = requested_bytes::<LirLaneEdge>(lane_edge_count)
        .saturating_add(requested_bytes::<LaneEdgeOrdinal>(successor_count))
        .saturating_add(requested_bytes::<LirIdentityField>(identity_field_count))
        .saturating_add(identity_field_byte_count);
    let controlled_live_bytes = unit
        .controlled_live_bytes
        .saturating_add(mir.controlled_live_bytes)
        .saturating_add(stage_scratch_bytes)
        .saturating_add(output_owned_bytes);
    let primary_span = mir.modules.first().map(|module| module.source_span.clone());
    let stable_key = mir
        .modules
        .first()
        .map(|module| module.authoring_namespace_id.as_ref().into());
    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    for (dimension, observed) in [
        (CompileLimitDimension::LirRecordCount, lir_record_count),
        (
            CompileLimitDimension::StageScratchBytes,
            stage_scratch_bytes,
        ),
        (CompileLimitDimension::OutputBytes, output_bytes),
        (
            CompileLimitDimension::CompilerControlledLiveBytes,
            controlled_live_bytes,
        ),
    ] {
        if observed > unit.limits.value(dimension) {
            diagnostics.push(Diagnostic::compile_limit_exceeded_at(
                dimension,
                unit.limits.value(dimension),
                observed,
                primary_span.clone(),
                stable_key.clone(),
            ));
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    let edge_capacity = usize::try_from(lane_edge_count)
        .map_err(|_| ordinal_overflow(&unit.limits, primary_span.clone()))?;
    let successor_capacity = usize::try_from(successor_count)
        .map_err(|_| ordinal_overflow(&unit.limits, primary_span.clone()))?;
    let identity_field_capacity = usize::try_from(identity_field_count)
        .map_err(|_| ordinal_overflow(&unit.limits, primary_span.clone()))?;
    let identity_byte_capacity = usize::try_from(identity_field_byte_count)
        .map_err(|_| output_overflow(&unit.limits, primary_span.clone()))?;

    let mut canonical_order = mir
        .lane_edges
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let raw = u32::try_from(index).expect("LIR precheck proved every MIR key fits u32");
            MirLaneEdgeKey::from_raw(raw)
        })
        .collect::<Vec<_>>();
    canonical_order.sort_unstable_by(|left, right| compare_identity_v1(mir, *left, *right));
    debug_assert!(
        canonical_order
            .windows(2)
            .all(|pair| { compare_identity_v1(mir, pair[0], pair[1]) == Ordering::Less })
    );

    let mut mir_to_lir = vec![LaneEdgeOrdinal::from_raw(0); edge_capacity];
    for (index, mir_key) in canonical_order.iter().copied().enumerate() {
        mir_to_lir[mir_key.index()] = LaneEdgeOrdinal::try_from_usize(index)
            .map_err(|_| ordinal_overflow(&unit.limits, primary_span.clone()))?;
    }

    let mut lane_edges = Vec::with_capacity(edge_capacity);
    let mut successors = Vec::with_capacity(successor_capacity);
    let mut identity_fields = Vec::with_capacity(identity_field_capacity);
    let mut identity_field_bytes = Vec::with_capacity(identity_byte_capacity);
    for mir_key in canonical_order {
        let edge = &mir.lane_edges[mir_key.index()];
        let namespace = &mir.modules[edge.module.index()].authoring_namespace_id;
        let identity_start = identity_fields.len();
        push_identity_field(
            &mut identity_fields,
            &mut identity_field_bytes,
            FieldTag::AuthoringNamespaceId,
            namespace.as_bytes(),
            &unit.limits,
            primary_span.clone(),
        )?;
        push_identity_field(
            &mut identity_fields,
            &mut identity_field_bytes,
            FieldTag::LaneEdgeKey,
            edge.stable_key.as_bytes(),
            &unit.limits,
            primary_span.clone(),
        )?;

        let successor_start = successors.len();
        successors.extend(
            mir.lane_edge_connections[edge.connections.as_usize_range()]
                .iter()
                .map(|connection| mir_to_lir[connection.target.index()]),
        );
        let ordinal = mir_to_lir[mir_key.index()];
        lane_edges.push(LirLaneEdge {
            ordinal,
            stable_id: edge.stable_id,
            identity_fields: TableRange::try_from_usize(
                identity_start,
                identity_fields.len().saturating_sub(identity_start),
            )
            .map_err(|overflow| table_overflow(overflow, &unit.limits, primary_span.clone()))?,
            length_meters: edge.length_meters,
            speed_limit_meters_per_second: edge.speed_limit_meters_per_second,
            successors: TableRange::try_from_usize(
                successor_start,
                successors.len().saturating_sub(successor_start),
            )
            .map_err(|overflow| table_overflow(overflow, &unit.limits, primary_span.clone()))?,
        });
    }

    debug_assert_eq!(lane_edges.len(), edge_capacity);
    debug_assert_eq!(successors.len(), successor_capacity);
    debug_assert_eq!(identity_fields.len(), identity_field_capacity);
    debug_assert_eq!(identity_field_bytes.len(), identity_byte_capacity);
    let semantic_digest = semantic_digest(
        &lane_edges,
        &successors,
        &identity_fields,
        &identity_field_bytes,
    );
    Ok(LirUnit {
        lane_edges: lane_edges.into_boxed_slice(),
        lane_edge_successors: successors.into_boxed_slice(),
        identity_fields: identity_fields.into_boxed_slice(),
        identity_field_bytes: identity_field_bytes.into_boxed_slice(),
        semantic_digest,
        lir_record_count,
        output_bytes,
        controlled_live_bytes: output_owned_bytes,
    })
}

/// 比较两个 `LaneEdge` 的完整 Identity v1 前像，而不物化拼接缓冲区。
fn compare_identity_v1(mir: &MirUnit, left: MirLaneEdgeKey, right: MirLaneEdgeKey) -> Ordering {
    let left_edge = &mir.lane_edges[left.index()];
    let right_edge = &mir.lane_edges[right.index()];
    let left_namespace = mir.modules[left_edge.module.index()]
        .authoring_namespace_id
        .as_bytes();
    let right_namespace = mir.modules[right_edge.module.index()]
        .authoring_namespace_id
        .as_bytes();

    // magic、encoding version、kind、field count 和字段标签对同种实体完全相同；每个
    // 变长字段在前像中都是 `u32_le(length) || value`，因此只比较这些差异片段即可得到
    // 与完整编码逐字节比较完全相同的顺序。
    compare_lane_edge_identity_fields(
        left_namespace,
        left_edge.stable_key.as_bytes(),
        right_namespace,
        right_edge.stable_key.as_bytes(),
    )
}

fn compare_lane_edge_identity_fields(
    left_namespace: &[u8],
    left_key: &[u8],
    right_namespace: &[u8],
    right_key: &[u8],
) -> Ordering {
    compare_length_prefixed(left_namespace, right_namespace)
        .then_with(|| compare_length_prefixed(left_key, right_key))
}

fn compare_length_prefixed(left: &[u8], right: &[u8]) -> Ordering {
    let left_length = u32::try_from(left.len())
        .expect("source validation proved Identity v1 field length fits u32");
    let right_length = u32::try_from(right.len())
        .expect("source validation proved Identity v1 field length fits u32");
    left_length
        .to_le_bytes()
        .cmp(&right_length.to_le_bytes())
        .then_with(|| left.cmp(right))
}

fn push_identity_field(
    fields: &mut Vec<LirIdentityField>,
    bytes: &mut Vec<u8>,
    tag: FieldTag,
    value: &[u8],
    limits: &crate::CompileLimits,
    primary_span: Option<SourceSpan>,
) -> Result<(), DiagnosticBundle> {
    let start = bytes.len();
    bytes.extend_from_slice(value);
    let value_bytes = TableRange::try_from_usize(start, value.len())
        .map_err(|_| output_overflow(limits, primary_span))?;
    fields.push(LirIdentityField { tag, value_bytes });
    Ok(())
}

fn semantic_digest(
    edges: &[LirLaneEdge],
    successors: &[LaneEdgeOrdinal],
    identity_fields: &[LirIdentityField],
    identity_field_bytes: &[u8],
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(LIR_SEMANTIC_DIGEST_DOMAIN);
    hash_u32(&mut hasher, EntityKind::LaneEdge.code().into());
    hash_u32(
        &mut hasher,
        u32::try_from(edges.len()).expect("LIR edge count was validated before allocation"),
    );
    for edge in edges {
        hash_u32(&mut hasher, edge.ordinal.raw());
        hasher.update(edge.stable_id.as_untyped().as_bytes());
        hash_u32(&mut hasher, edge.identity_fields.len());
        for field in &identity_fields[edge.identity_fields.as_usize_range()] {
            hasher.update(&field.tag.code().to_le_bytes());
            hash_u32(&mut hasher, field.value_bytes.len());
            hasher.update(&identity_field_bytes[field.value_bytes.as_usize_range()]);
        }
        hasher.update(&edge.length_meters.to_bits().to_le_bytes());
        hasher.update(&edge.speed_limit_meters_per_second.to_bits().to_le_bytes());
        hash_u32(&mut hasher, edge.successors.len());
        for successor in &successors[edge.successors.as_usize_range()] {
            hash_u32(&mut hasher, successor.raw());
        }
    }
    *hasher.finalize().as_bytes()
}

fn hash_u32(hasher: &mut blake3::Hasher, value: u32) {
    hasher.update(&value.to_le_bytes());
}

fn requested_bytes<T>(count: u64) -> u64 {
    count.saturating_mul(u64::try_from(size_of::<T>()).unwrap_or(u64::MAX))
}

fn table_overflow(
    _: ArenaKeyOverflow,
    limits: &crate::CompileLimits,
    primary_span: Option<SourceSpan>,
) -> DiagnosticBundle {
    ordinal_overflow(limits, primary_span)
}

fn ordinal_overflow(
    limits: &crate::CompileLimits,
    primary_span: Option<SourceSpan>,
) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::compile_limit_exceeded_at(
        CompileLimitDimension::LirRecordCount,
        limits.value(CompileLimitDimension::LirRecordCount),
        u64::from(u32::MAX) + 1,
        primary_span,
        None,
    ))
}

fn output_overflow(
    limits: &crate::CompileLimits,
    primary_span: Option<SourceSpan>,
) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::compile_limit_exceeded_at(
        CompileLimitDimension::OutputBytes,
        limits.value(CompileLimitDimension::OutputBytes),
        u64::MAX,
        primary_span,
        None,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::build_hir;
    use crate::identity::{IdentityFieldInput, encode_canonical_identity};
    use crate::mir::lower_to_mir;
    use crate::{
        CompilationUnitBuilder, CompileLimits, DiagnosticPayload, LaneEdgeInput, LaneEdgeReference,
        SourceModuleHeader, SourceModuleHeaderInput, SyntheticModule, SyntheticModuleBuilder,
    };

    fn module(
        namespace: &str,
        source_document_key: &str,
        imports: &[&str],
        edges: &[(&str, f64, &[LaneEdgeReference<'_>])],
    ) -> SyntheticModule {
        let limits = CompileLimits::p100_initial_v1();
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: namespace,
                source_document_key,
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x11; 32],
                frontend_options_digest: [0x22; 32],
                random_seed: Some(42),
                provenance: "repository:laneflow",
            },
            &limits,
        )
        .unwrap();
        let mut builder = SyntheticModuleBuilder::new(header, &limits).unwrap();
        for import in imports {
            builder.add_import(import).unwrap();
        }
        for (key, length_meters, successors) in edges {
            builder
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: key,
                    length_meters: *length_meters,
                    speed_limit_meters_per_second: 13.75,
                    successors,
                })
                .unwrap();
        }
        builder.finish().unwrap()
    }

    fn unit(modules: impl IntoIterator<Item = SyntheticModule>) -> CompilationUnit {
        let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
        for module in modules {
            builder.add_synthetic_module(module).unwrap();
        }
        builder.build().unwrap()
    }

    fn lir(unit: &CompilationUnit) -> LirUnit {
        let hir = build_hir(unit).unwrap();
        let mir = lower_to_mir(unit, &hir).unwrap();
        freeze_lir(unit, &mir).unwrap()
    }

    fn identity_values(lir: &LirUnit, edge: &LirLaneEdge) -> Vec<(FieldTag, Vec<u8>)> {
        lir.identity_fields[edge.identity_fields.as_usize_range()]
            .iter()
            .map(|field| {
                (
                    field.tag,
                    lir.identity_field_bytes[field.value_bytes.as_usize_range()].to_vec(),
                )
            })
            .collect()
    }

    #[test]
    fn lir_sorts_by_complete_identity_bytes_and_remaps_connections() {
        let app_successors = [LaneEdgeReference::imported("z", "edge-z")];
        let unit = unit([
            module("a", "app", &["z"], &[("edge-a", 10.0, &app_successors)]),
            module("z", "base", &[], &[("edge-z", 20.0, &[])]),
        ]);
        let lir = lir(&unit);

        assert_eq!(lir.lane_edges.len(), 2);
        assert_eq!(lir.lane_edge_successors.len(), 1);
        assert_eq!(lir.lir_record_count, 3);
        assert_eq!(lir.output_bytes, 194);
        assert!(lir.controlled_live_bytes > 0);
        assert_eq!(lir.lane_edges[0].ordinal.raw(), 0);
        assert_eq!(lir.lane_edges[1].ordinal.raw(), 1);
        assert_eq!(
            identity_values(&lir, &lir.lane_edges[0]),
            [
                (FieldTag::AuthoringNamespaceId, b"a".to_vec()),
                (FieldTag::LaneEdgeKey, b"edge-a".to_vec()),
            ]
        );
        assert_eq!(
            identity_values(&lir, &lir.lane_edges[1]),
            [
                (FieldTag::AuthoringNamespaceId, b"z".to_vec()),
                (FieldTag::LaneEdgeKey, b"edge-z".to_vec()),
            ]
        );
        assert_eq!(
            lir.lane_edge_successors[lir.lane_edges[0].successors.as_usize_range()][0].raw(),
            1
        );
    }

    #[test]
    fn identity_order_uses_little_endian_length_prefix_before_text_bytes() {
        let unit = unit([
            module("aa", "aa", &[], &[("edge", 10.0, &[])]),
            module("z", "z", &[], &[("edge", 10.0, &[])]),
        ]);
        let lir = lir(&unit);

        // 普通文本顺序是 "aa" < "z"，但 Identity v1 在字段值前编码 u32_le 长度；
        // 完整前像的第一个差异字节因此是 1 < 2。
        assert_eq!(
            identity_values(&lir, &lir.lane_edges[0])[0].1,
            b"z".to_vec()
        );
        assert_eq!(
            identity_values(&lir, &lir.lane_edges[1])[0].1,
            b"aa".to_vec()
        );
    }

    #[test]
    fn allocation_free_sort_key_matches_the_identity_v1_encoder() {
        let namespaces = [b"a".as_slice(), b"aa", b"z", b"city/a"];
        let keys = [b"e".as_slice(), b"edge", b"edge-00", b"edge-longer"];

        for left_namespace in namespaces {
            for left_key in keys {
                let left_fields = [
                    IdentityFieldInput::new(FieldTag::AuthoringNamespaceId, left_namespace),
                    IdentityFieldInput::new(FieldTag::LaneEdgeKey, left_key),
                ];
                let left =
                    encode_canonical_identity(EntityKind::LaneEdge, &left_fields, 53).unwrap();
                for right_namespace in namespaces {
                    for right_key in keys {
                        let right_fields = [
                            IdentityFieldInput::new(
                                FieldTag::AuthoringNamespaceId,
                                right_namespace,
                            ),
                            IdentityFieldInput::new(FieldTag::LaneEdgeKey, right_key),
                        ];
                        let right =
                            encode_canonical_identity(EntityKind::LaneEdge, &right_fields, 53)
                                .unwrap();

                        assert_eq!(
                            compare_lane_edge_identity_fields(
                                left_namespace,
                                left_key,
                                right_namespace,
                                right_key,
                            ),
                            left.canonical_bytes().cmp(right.canonical_bytes())
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn semantic_digest_is_invariant_to_declaration_order_and_source_spans() {
        let successors = [
            LaneEdgeReference::local("edge-c"),
            LaneEdgeReference::local("edge-b"),
        ];
        let left = unit([module(
            "city/a",
            "left.document",
            &[],
            &[
                ("edge-a", 10.0, &successors),
                ("edge-b", 20.0, &[]),
                ("edge-c", 30.0, &[]),
            ],
        )]);
        let right = unit([module(
            "city/a",
            "right.document",
            &[],
            &[
                ("edge-c", 30.0, &[]),
                ("edge-a", 10.0, &successors),
                ("edge-b", 20.0, &[]),
            ],
        )]);

        assert_eq!(lir(&left).semantic_digest, lir(&right).semantic_digest);
    }

    #[test]
    fn semantic_digest_changes_with_static_semantics() {
        let left = unit([module(
            "city/a",
            "same.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]);
        let right = unit([module(
            "city/a",
            "same.document",
            &[],
            &[("edge-a", 11.0, &[])],
        )]);

        assert_ne!(lir(&left).semantic_digest, lir(&right).semantic_digest);
    }

    #[test]
    fn lir_checks_record_scratch_output_and_live_limits_before_allocation() {
        let successors = [LaneEdgeReference::local("edge-a")];
        let mut unit = unit([module(
            "city/a",
            "city/a",
            &[],
            &[("edge-a", 10.0, &successors)],
        )]);
        let hir = build_hir(&unit).unwrap();
        let mir = lower_to_mir(&unit, &hir).unwrap();

        for (limits, expected_dimension) in [
            (
                CompileLimits::p100_initial_v1().with_test_lir_limits(
                    1,
                    u32::MAX,
                    u32::MAX,
                    u32::MAX,
                ),
                CompileLimitDimension::LirRecordCount,
            ),
            (
                CompileLimits::p100_initial_v1().with_test_lir_limits(
                    u32::MAX,
                    0,
                    u32::MAX,
                    u32::MAX,
                ),
                CompileLimitDimension::StageScratchBytes,
            ),
            (
                CompileLimits::p100_initial_v1().with_test_lir_limits(
                    u32::MAX,
                    u32::MAX,
                    0,
                    u32::MAX,
                ),
                CompileLimitDimension::OutputBytes,
            ),
            (
                CompileLimits::p100_initial_v1().with_test_lir_limits(
                    u32::MAX,
                    u32::MAX,
                    u32::MAX,
                    u32::try_from(unit.controlled_live_bytes + mir.controlled_live_bytes).unwrap(),
                ),
                CompileLimitDimension::CompilerControlledLiveBytes,
            ),
        ] {
            unit.limits = limits;
            // 资源限制来自同一不可变配置档；测试只替换限制快照，不改变 MIR 语义。
            let failure = match freeze_lir(&unit, &mir) {
                Ok(_) => panic!("LIR resource limit must fail closed"),
                Err(diagnostics) => diagnostics,
            };
            assert!(failure.diagnostics().iter().any(|diagnostic| matches!(
                diagnostic.payload(),
                DiagnosticPayload::CompileLimitExceeded { dimension, .. }
                    if *dimension == expected_dimension
            )));
        }
    }
}
