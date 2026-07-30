//! 独立精确研究预言机。
//!
//! 独立构造路径只共享受信任的原始清单、图配置档枚举和不可变结果值类型，不调用
//! `generator` / `manifest` / `identity` 的解析、身份展开、关系展开、排序或编码
//! 辅助函数；规范遍历由本模块自己的 `BTreeMap` / `BTreeSet` 路径完成。本模块的
//! 顶层验证入口再分别调用生产者与该独立构造路径，并比较完整结果。

use crate::identity::{
    IdentityCaseOutput, IdentityDeclarationVector, IdentityFieldVector, SemanticRecord,
    SemanticRecordVector, build_identity_case,
};
use crate::pipeline::build_identity_stage_case;
use crate::stage_oracle::verify_identity_stage_exact;
use crate::{GraphProfileId, TrustedContract};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const ABSENT_LOCAL_INDEX: u32 = u32::MAX;
const STABLE_ID_DOMAIN: &[u8] = b"laneflow.stable-id.v1\0";
const ORACLE_IDENTITY_WORKLOAD_ID: &str = "LF-COMP-ID-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OracleVerificationReport {
    pub checked_cases: u32,
    pub checked_n1_cases: u32,
    pub checked_n2_cases: u32,
    pub checked_stage_cases: u32,
}

pub fn verify_identity_oracle_matrix(
    trusted: &TrustedContract,
) -> Result<OracleVerificationReport, OracleVerificationError> {
    let generator = trusted.generator_contract()?;
    let identity = trusted.identity_contract()?;
    let stage = trusted.stage_contract()?;
    let mut checked_cases = 0_u32;
    for graph_profile in GraphProfileId::ALL {
        for n in [1, 2] {
            let produced = build_identity_case(&generator, &identity, graph_profile, n)?;
            let oracle = build_identity_oracle_case(&trusted.workload_manifest, graph_profile, n)?;
            if produced != oracle {
                return Err(OracleVerificationError::Mismatch { graph_profile, n });
            }
            let produced_stage =
                build_identity_stage_case(&generator, &identity, &stage, graph_profile, n)?;
            verify_identity_stage_exact(
                &trusted.workload_manifest,
                graph_profile,
                n,
                &produced_stage,
            )?;
            checked_cases += 1;
        }
    }

    Ok(OracleVerificationReport {
        checked_cases,
        checked_n1_cases: u32::try_from(GraphProfileId::ALL.len())
            .expect("graph profile count must fit u32"),
        checked_n2_cases: u32::try_from(GraphProfileId::ALL.len())
            .expect("graph profile count must fit u32"),
        checked_stage_cases: checked_cases,
    })
}

pub(crate) fn build_identity_oracle_case(
    manifest: &serde_json::Value,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<IdentityCaseOutput, ExactOracleError> {
    if n == 0 {
        return Err(ExactOracleError::InvalidValue {
            path: "N".to_owned(),
            expected: "at least 1".to_owned(),
        });
    }
    let generator_version = required_u32(manifest, "generatorVersion")?;
    let base_seed = required_hex_u64(manifest, "baseSeedHexU64")?;
    let identity_encoding_version = required_u16(manifest, "identityEncodingVersion")?;
    let semantic_record_stream_version = required_u32(manifest, "semanticRecordStreamVersion")?;
    let semantic_record_domain =
        required_string(manifest, "semanticRecordDomainUtf8NulTerminated")?;
    let namespace_contract = required_object(manifest, "namespaceDerivation")?;
    let namespace_domain = required_string(namespace_contract, "domainUtf8NulTerminated")?;
    require_string_array(
        namespace_contract,
        "inputOrder",
        &[
            "domainUtf8NulTerminated",
            "generatorVersionU32Le",
            "baseSeedU64Le",
            "workloadIdByteLengthU32Le",
            "workloadIdUtf8",
            "graphProfileIdByteLengthU32Le",
            "graphProfileIdUtf8",
            "canonicalModuleNameByteLengthU32Le",
            "canonicalModuleNameUtf8",
        ],
    )?;
    require_string_array(
        namespace_contract,
        "canonicalModuleNamePatterns",
        &["root", "shared/common", "group/{g:08x}", "unit/{i:08x}"],
    )?;
    require_string(namespace_contract, "hash", "BLAKE3")?;
    require_u64(
        required_object(namespace_contract, "selectedDigestBytes")?,
        "offset",
        0,
    )?;
    require_u64(
        required_object(namespace_contract, "selectedDigestBytes")?,
        "length",
        16,
    )?;
    require_string(namespace_contract, "fieldEncoding", "lowercase-ascii-hex")?;
    require_u64(namespace_contract, "fieldLengthBytes", 32)?;
    validate_short_unique_profile(manifest)?;
    validate_identity_workload_profiles(manifest)?;

    let bindings = parse_bindings(manifest)?;
    let reference_slots = parse_reference_slots(manifest)?;
    validate_stable_id_slots(&bindings, &reference_slots)?;
    let owner_relations = parse_owner_relations(manifest, &bindings)?;

    let mut unit_namespaces = Vec::with_capacity(usize::try_from(n).expect("N must fit usize"));
    let mut declarations = BTreeMap::<(u32, u16), OracleDeclaration>::new();
    for unit_index in 0..n {
        let module_name = format!("unit/{unit_index:08x}");
        let namespace = derive_namespace(
            namespace_domain,
            generator_version,
            base_seed,
            oracle_graph_profile_id(graph_profile),
            &module_name,
        );
        for binding in bindings.values() {
            let mut fields = Vec::with_capacity(binding.fields.len());
            for field in &binding.fields {
                let bytes = match field.value {
                    OracleFieldValue::Namespace => namespace.as_bytes().to_vec(),
                    OracleFieldValue::ProfiledKey { kind, local } => {
                        format!("{kind:02x}/{unit_index:08x}/{local:08x}").into_bytes()
                    }
                    OracleFieldValue::StableId { kind, local: _ } => declarations
                        .get(&(unit_index, kind))
                        .ok_or(ExactOracleError::UnresolvedStableId {
                            unit_index,
                            source_kind: binding.entity_kind_code,
                            target_kind: kind,
                        })?
                        .stable_id
                        .to_vec(),
                };
                fields.push(OracleField {
                    tag: field.tag,
                    bytes,
                });
            }
            let canonical_identity_bytes =
                oracle_identity_bytes(identity_encoding_version, binding.entity_kind_code, &fields);
            let mut hasher = blake3::Hasher::new();
            hasher.update(STABLE_ID_DOMAIN);
            hasher.update(&canonical_identity_bytes);
            let mut stable_id = [0_u8; 16];
            stable_id.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
            let key = (unit_index, binding.entity_kind_code);
            let previous = declarations.insert(
                key,
                OracleDeclaration {
                    unit_index,
                    entity_kind_code: binding.entity_kind_code,
                    entity_kind: binding.entity_kind.clone(),
                    stable_id,
                    canonical_identity_bytes,
                    fields,
                },
            );
            if previous.is_some() {
                return Err(ExactOracleError::DuplicateDeclaration {
                    unit_index,
                    entity_kind: binding.entity_kind_code,
                });
            }
        }
        unit_namespaces.push(namespace);
    }

    let owner_ordinals = oracle_owner_ordinals(declarations.values())?;
    let mut ordered_records = BTreeMap::<OracleRecordKey, OracleRecord>::new();
    for declaration in declarations.values() {
        let payload = oracle_identity_payload(&declaration.fields);
        insert_record(
            &mut ordered_records,
            OracleRecord {
                record_kind: 1,
                entity_kind_code: declaration.entity_kind_code,
                entity_kind: declaration.entity_kind.clone(),
                stable_id: declaration.stable_id,
                owner_ordinal: owner_ordinals
                    [&(declaration.entity_kind_code, declaration.stable_id)],
                local_index: ABSENT_LOCAL_INDEX,
                payload,
            },
        )?;
    }
    for unit_index in 0..n {
        for relation in &owner_relations {
            let child = declarations.get(&(unit_index, relation.child_kind)).ok_or(
                ExactOracleError::MissingDeclaration {
                    unit_index,
                    entity_kind: relation.child_kind,
                },
            )?;
            let parent = declarations
                .get(&(unit_index, relation.parent_kind))
                .ok_or(ExactOracleError::MissingDeclaration {
                    unit_index,
                    entity_kind: relation.parent_kind,
                })?;
            let mut payload = Vec::with_capacity(18);
            oracle_u16(&mut payload, relation.parent_kind);
            payload.extend_from_slice(&parent.stable_id);
            insert_record(
                &mut ordered_records,
                OracleRecord {
                    record_kind: 2,
                    entity_kind_code: child.entity_kind_code,
                    entity_kind: child.entity_kind.clone(),
                    stable_id: child.stable_id,
                    owner_ordinal: owner_ordinals[&(child.entity_kind_code, child.stable_id)],
                    local_index: ABSENT_LOCAL_INDEX,
                    payload,
                },
            )?;
        }
    }

    let semantic_record_stream = oracle_semantic_stream(
        semantic_record_domain,
        semantic_record_stream_version,
        ordered_records.values(),
    );
    let raw_records = ordered_records
        .values()
        .map(OracleRecord::to_shared_record)
        .collect();
    Ok(IdentityCaseOutput {
        graph_profile,
        n,
        unit_namespaces,
        declarations: declarations
            .values()
            .map(OracleDeclaration::to_vector)
            .collect(),
        records: ordered_records
            .values()
            .map(OracleRecord::to_vector)
            .collect(),
        raw_records,
        semantic_digest_sha256: oracle_hex(&Sha256::digest(&semantic_record_stream)),
        semantic_record_stream,
    })
}

#[derive(Clone, Debug)]
struct OracleBinding {
    entity_kind_code: u16,
    entity_kind: String,
    fields: Vec<OracleFieldBinding>,
}

#[derive(Clone, Copy, Debug)]
struct OracleFieldBinding {
    tag: u16,
    value: OracleFieldValue,
}

#[derive(Clone, Copy, Debug)]
enum OracleFieldValue {
    Namespace,
    ProfiledKey { kind: u16, local: u32 },
    StableId { kind: u16, local: u32 },
}

#[derive(Clone, Debug)]
struct OracleDeclaration {
    unit_index: u32,
    entity_kind_code: u16,
    entity_kind: String,
    stable_id: [u8; 16],
    canonical_identity_bytes: Vec<u8>,
    fields: Vec<OracleField>,
}

impl OracleDeclaration {
    fn to_vector(&self) -> IdentityDeclarationVector {
        IdentityDeclarationVector {
            unit_index: self.unit_index,
            entity_kind_code: self.entity_kind_code,
            entity_kind: self.entity_kind.clone(),
            stable_id: oracle_hex(&self.stable_id),
            canonical_identity_bytes_hex: oracle_hex(&self.canonical_identity_bytes),
            fields: self
                .fields
                .iter()
                .map(|field| IdentityFieldVector {
                    tag: field.tag,
                    field_bytes_hex: oracle_hex(&field.bytes),
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug)]
struct OracleField {
    tag: u16,
    bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
struct OracleOwnerRelation {
    child_kind: u16,
    parent_kind: u16,
}

#[derive(Clone, Debug)]
struct OracleRecord {
    record_kind: u16,
    entity_kind_code: u16,
    entity_kind: String,
    stable_id: [u8; 16],
    owner_ordinal: u32,
    local_index: u32,
    payload: Vec<u8>,
}

impl OracleRecord {
    fn key(&self) -> OracleRecordKey {
        OracleRecordKey {
            record_kind: self.record_kind,
            entity_kind_code: self.entity_kind_code,
            stable_id: self.stable_id,
            owner_ordinal: self.owner_ordinal,
            local_index: self.local_index,
            payload: self.payload.clone(),
        }
    }

    fn to_vector(&self) -> SemanticRecordVector {
        SemanticRecordVector {
            record_kind: self.record_kind,
            entity_kind_code: self.entity_kind_code,
            entity_kind: self.entity_kind.clone(),
            stable_id: oracle_hex(&self.stable_id),
            owner_ordinal: self.owner_ordinal,
            local_index: self.local_index,
            payload_hex: oracle_hex(&self.payload),
        }
    }

    fn to_shared_record(&self) -> SemanticRecord {
        SemanticRecord {
            record_kind: self.record_kind,
            entity_kind_code: self.entity_kind_code,
            entity_kind: self.entity_kind.clone(),
            stable_id: self.stable_id,
            owner_ordinal: self.owner_ordinal,
            local_index: self.local_index,
            payload: self.payload.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct OracleRecordKey {
    record_kind: u16,
    entity_kind_code: u16,
    stable_id: [u8; 16],
    owner_ordinal: u32,
    local_index: u32,
    payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct OracleReferenceSlot {
    source_entity_kind: String,
    target_entity_kind_code: u16,
    slot: u32,
}

fn parse_bindings(
    manifest: &serde_json::Value,
) -> Result<BTreeMap<u16, OracleBinding>, ExactOracleError> {
    let raw_bindings = required_array(manifest, "identityBindings")?;
    let mut bindings = BTreeMap::new();
    for raw in raw_bindings {
        let entity_kind_code = required_u16(raw, "entityKindCode")?;
        let entity_kind = required_string(raw, "entityKind")?.to_owned();
        let mut fields = Vec::new();
        let mut previous_tag = 0_u16;
        for raw_field in required_array(raw, "fields")? {
            let tag = required_u16(raw_field, "tag")?;
            if tag <= previous_tag {
                return Err(ExactOracleError::InvalidValue {
                    path: format!("identityBindings[{entity_kind_code}].fields[].tag"),
                    expected: "strictly increasing".to_owned(),
                });
            }
            previous_tag = tag;
            let value = parse_oracle_field_value(required_string(raw_field, "value")?)?;
            if let OracleFieldValue::ProfiledKey { kind, .. } = value
                && kind != entity_kind_code
            {
                return Err(ExactOracleError::InvalidValue {
                    path: format!("identityBindings[{entity_kind_code}].fields[].value"),
                    expected: format!("profiled-key kind {entity_kind_code}"),
                });
            }
            fields.push(OracleFieldBinding { tag, value });
        }
        let previous = bindings.insert(
            entity_kind_code,
            OracleBinding {
                entity_kind_code,
                entity_kind,
                fields,
            },
        );
        if previous.is_some() {
            return Err(ExactOracleError::DuplicateEntityKind(entity_kind_code));
        }
    }
    if bindings.len() != 22 || bindings.keys().copied().ne(1_u16..=22) {
        return Err(ExactOracleError::InvalidValue {
            path: "identityBindings[].entityKindCode".to_owned(),
            expected: "the closed sequence 1..=22".to_owned(),
        });
    }
    Ok(bindings)
}

fn parse_reference_slots(
    manifest: &serde_json::Value,
) -> Result<BTreeSet<OracleReferenceSlot>, ExactOracleError> {
    let expansion = required_object(manifest, "identityBindingExpansion")?;
    let stable_id_expression = required_object(expansion, "stableIdExpression")?;
    let mut slots = BTreeSet::new();
    for raw in required_array(stable_id_expression, "referenceSlots")? {
        let slot = OracleReferenceSlot {
            source_entity_kind: required_string(raw, "sourceEntityKind")?.to_owned(),
            target_entity_kind_code: required_u16(raw, "targetEntityKindCode")?,
            slot: required_u32(raw, "slot")?,
        };
        if !slots.insert(slot) {
            return Err(ExactOracleError::DuplicateReferenceSlot);
        }
    }
    Ok(slots)
}

fn validate_stable_id_slots(
    bindings: &BTreeMap<u16, OracleBinding>,
    slots: &BTreeSet<OracleReferenceSlot>,
) -> Result<(), ExactOracleError> {
    let mut used_slots = BTreeSet::new();
    for binding in bindings.values() {
        for field in &binding.fields {
            if let OracleFieldValue::StableId { kind, local } = field.value {
                let slot = OracleReferenceSlot {
                    source_entity_kind: binding.entity_kind.clone(),
                    target_entity_kind_code: kind,
                    slot: local,
                };
                if !slots.contains(&slot) {
                    return Err(ExactOracleError::UnknownReferenceSlot {
                        source_entity_kind: binding.entity_kind.clone(),
                        target_entity_kind: kind,
                        slot: local,
                    });
                }
                if !used_slots.insert(slot) {
                    return Err(ExactOracleError::DuplicateReferenceSlot);
                }
            }
        }
    }
    if &used_slots != slots {
        return Err(ExactOracleError::InvalidValue {
            path: "identityBindingExpansion.stableIdExpression.referenceSlots".to_owned(),
            expected: "exactly the StableId slots consumed by identityBindings".to_owned(),
        });
    }
    Ok(())
}

fn validate_short_unique_profile(manifest: &serde_json::Value) -> Result<(), ExactOracleError> {
    let profile = required_array(manifest, "stringProfiles")?
        .iter()
        .find(|profile| {
            profile.get("id").and_then(serde_json::Value::as_str) == Some("short-unique-v1")
        })
        .ok_or_else(|| {
            ExactOracleError::MissingPath("stringProfiles[short-unique-v1]".to_owned())
        })?;
    require_string(
        profile,
        "profiledKeyFormula",
        "kindCodeHex2 || '/' || unitIndexHex8 || '/' || localIndexHex8",
    )?;
    require_u64(profile, "profiledKeyLengthBytes", 20)?;
    require_string(profile, "encoding", "ascii")
}

fn validate_identity_workload_profiles(
    manifest: &serde_json::Value,
) -> Result<(), ExactOracleError> {
    let workload = find_identity_workload(manifest)?;
    require_string_array(
        workload,
        "graphProfiles",
        &["wide-star-v1", "deep-chain-v1", "shared-fanin-dag-v1"],
    )?;
    require_string_array(
        workload,
        "stringProfiles",
        &["short-unique-v1", "shared-prefix-256-v1", "long-4096-v1"],
    )
}

fn parse_owner_relations(
    manifest: &serde_json::Value,
    bindings: &BTreeMap<u16, OracleBinding>,
) -> Result<Vec<OracleOwnerRelation>, ExactOracleError> {
    let kind_by_name = bindings
        .values()
        .map(|binding| (binding.entity_kind.as_str(), binding.entity_kind_code))
        .collect::<BTreeMap<_, _>>();
    let workload = find_identity_workload(manifest)?;
    let mut relations = Vec::new();
    for raw in required_array(workload, "ownerRelations")? {
        let relation = raw
            .as_str()
            .ok_or_else(|| ExactOracleError::InvalidType("ownerRelations[]".to_owned()))?;
        let (child, parent) =
            relation
                .split_once("->")
                .ok_or_else(|| ExactOracleError::InvalidValue {
                    path: "ownerRelations[]".to_owned(),
                    expected: "Child->Parent".to_owned(),
                })?;
        relations.push(OracleOwnerRelation {
            child_kind: *kind_by_name
                .get(child)
                .ok_or_else(|| ExactOracleError::UnknownEntityKind(child.to_owned()))?,
            parent_kind: *kind_by_name
                .get(parent)
                .ok_or_else(|| ExactOracleError::UnknownEntityKind(parent.to_owned()))?,
        });
    }
    if relations.len() != 10 {
        return Err(ExactOracleError::InvalidValue {
            path: "workloads[LF-COMP-ID-v1].ownerRelations".to_owned(),
            expected: "exactly 10 relations".to_owned(),
        });
    }
    Ok(relations)
}

fn find_identity_workload(
    manifest: &serde_json::Value,
) -> Result<&serde_json::Value, ExactOracleError> {
    required_array(manifest, "workloads")?
        .iter()
        .find(|workload| {
            workload.get("id").and_then(serde_json::Value::as_str)
                == Some(ORACLE_IDENTITY_WORKLOAD_ID)
        })
        .ok_or_else(|| ExactOracleError::MissingPath("workloads[LF-COMP-ID-v1]".to_owned()))
}

fn parse_oracle_field_value(value: &str) -> Result<OracleFieldValue, ExactOracleError> {
    if value == "namespace" {
        return Ok(OracleFieldValue::Namespace);
    }
    for (prefix, is_profiled_key) in [("profiled-key(kind=", true), ("stable-id(kind=", false)] {
        if let Some(arguments) = value
            .strip_prefix(prefix)
            .and_then(|value| value.strip_suffix(')'))
        {
            let (kind, local) =
                arguments
                    .split_once(",local=")
                    .ok_or_else(|| ExactOracleError::InvalidValue {
                        path: "identityBindings[].fields[].value".to_owned(),
                        expected: "kind/local expression".to_owned(),
                    })?;
            let kind = kind
                .parse::<u16>()
                .map_err(|_| ExactOracleError::InvalidInteger(value.to_owned()))?;
            let local = local
                .parse::<u32>()
                .map_err(|_| ExactOracleError::InvalidInteger(value.to_owned()))?;
            return Ok(if is_profiled_key {
                OracleFieldValue::ProfiledKey { kind, local }
            } else {
                OracleFieldValue::StableId { kind, local }
            });
        }
    }
    Err(ExactOracleError::InvalidValue {
        path: "identityBindings[].fields[].value".to_owned(),
        expected: "namespace, profiled-key, or stable-id".to_owned(),
    })
}

fn derive_namespace(
    domain: &str,
    generator_version: u32,
    base_seed: u64,
    graph_profile: &str,
    module_name: &str,
) -> String {
    let mut preimage = Vec::new();
    preimage.extend_from_slice(domain.as_bytes());
    preimage.push(0);
    oracle_u32(&mut preimage, generator_version);
    oracle_u64(&mut preimage, base_seed);
    oracle_length_prefixed(&mut preimage, ORACLE_IDENTITY_WORKLOAD_ID);
    oracle_length_prefixed(&mut preimage, graph_profile);
    oracle_length_prefixed(&mut preimage, module_name);
    oracle_hex(&blake3::hash(&preimage).as_bytes()[..16])
}

fn oracle_graph_profile_id(graph_profile: GraphProfileId) -> &'static str {
    match graph_profile {
        GraphProfileId::WideStar => "wide-star-v1",
        GraphProfileId::DeepChain => "deep-chain-v1",
        GraphProfileId::SharedFaninDag => "shared-fanin-dag-v1",
    }
}

fn oracle_identity_bytes(
    identity_encoding_version: u16,
    entity_kind: u16,
    fields: &[OracleField],
) -> Vec<u8> {
    let mut bytes = b"LFID".to_vec();
    oracle_u16(&mut bytes, identity_encoding_version);
    oracle_u16(&mut bytes, entity_kind);
    bytes.extend_from_slice(&oracle_identity_payload(fields));
    bytes
}

fn oracle_identity_payload(fields: &[OracleField]) -> Vec<u8> {
    let mut payload = Vec::new();
    oracle_u16(
        &mut payload,
        u16::try_from(fields.len()).expect("identity field count must fit u16"),
    );
    for field in fields {
        oracle_u16(&mut payload, field.tag);
        oracle_u32(
            &mut payload,
            u32::try_from(field.bytes.len()).expect("identity field length must fit u32"),
        );
        payload.extend_from_slice(&field.bytes);
    }
    payload
}

fn oracle_owner_ordinals<'a>(
    declarations: impl Iterator<Item = &'a OracleDeclaration>,
) -> Result<BTreeMap<(u16, [u8; 16]), u32>, ExactOracleError> {
    let mut stable_ids_by_kind = BTreeMap::<u16, BTreeSet<[u8; 16]>>::new();
    for declaration in declarations {
        if !stable_ids_by_kind
            .entry(declaration.entity_kind_code)
            .or_default()
            .insert(declaration.stable_id)
        {
            return Err(ExactOracleError::DuplicateStableId {
                entity_kind: declaration.entity_kind_code,
            });
        }
    }
    let mut ordinals = BTreeMap::new();
    for (entity_kind, stable_ids) in stable_ids_by_kind {
        for (ordinal, stable_id) in stable_ids.into_iter().enumerate() {
            ordinals.insert(
                (entity_kind, stable_id),
                u32::try_from(ordinal).expect("owner ordinal must fit u32"),
            );
        }
    }
    Ok(ordinals)
}

fn insert_record(
    records: &mut BTreeMap<OracleRecordKey, OracleRecord>,
    record: OracleRecord,
) -> Result<(), ExactOracleError> {
    if records.insert(record.key(), record).is_some() {
        return Err(ExactOracleError::DuplicateRecord);
    }
    Ok(())
}

fn oracle_semantic_stream<'a>(
    domain: &str,
    stream_version: u32,
    records: impl ExactSizeIterator<Item = &'a OracleRecord>,
) -> Vec<u8> {
    let mut stream = Vec::new();
    stream.extend_from_slice(domain.as_bytes());
    stream.push(0);
    oracle_u32(&mut stream, stream_version);
    oracle_u64(
        &mut stream,
        u64::try_from(records.len()).expect("record count must fit u64"),
    );
    for record in records {
        oracle_u16(&mut stream, record.record_kind);
        oracle_u16(&mut stream, record.entity_kind_code);
        stream.extend_from_slice(&record.stable_id);
        oracle_u32(&mut stream, record.owner_ordinal);
        oracle_u32(&mut stream, record.local_index);
        oracle_u64(
            &mut stream,
            u64::try_from(record.payload.len()).expect("payload length must fit u64"),
        );
        stream.extend_from_slice(&record.payload);
    }
    stream
}

fn oracle_length_prefixed(output: &mut Vec<u8>, value: &str) {
    oracle_u32(
        output,
        u32::try_from(value.len()).expect("research identifier must fit u32"),
    );
    output.extend_from_slice(value.as_bytes());
}

fn oracle_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn oracle_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn oracle_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn oracle_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn required_object<'a>(
    value: &'a serde_json::Value,
    field: &str,
) -> Result<&'a serde_json::Value, ExactOracleError> {
    value
        .get(field)
        .filter(|value| value.is_object())
        .ok_or_else(|| ExactOracleError::MissingPath(field.to_owned()))
}

fn required_array<'a>(
    value: &'a serde_json::Value,
    field: &str,
) -> Result<&'a [serde_json::Value], ExactOracleError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| ExactOracleError::MissingPath(field.to_owned()))
}

fn required_string<'a>(
    value: &'a serde_json::Value,
    field: &str,
) -> Result<&'a str, ExactOracleError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ExactOracleError::MissingPath(field.to_owned()))
}

fn required_u16(value: &serde_json::Value, field: &str) -> Result<u16, ExactOracleError> {
    let value = required_u64(value, field)?;
    u16::try_from(value).map_err(|_| ExactOracleError::InvalidInteger(field.to_owned()))
}

fn required_u32(value: &serde_json::Value, field: &str) -> Result<u32, ExactOracleError> {
    let value = required_u64(value, field)?;
    u32::try_from(value).map_err(|_| ExactOracleError::InvalidInteger(field.to_owned()))
}

fn required_u64(value: &serde_json::Value, field: &str) -> Result<u64, ExactOracleError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| ExactOracleError::MissingPath(field.to_owned()))
}

fn required_hex_u64(value: &serde_json::Value, field: &str) -> Result<u64, ExactOracleError> {
    let text = required_string(value, field)?;
    u64::from_str_radix(text, 16).map_err(|_| ExactOracleError::InvalidInteger(field.to_owned()))
}

fn require_string(
    value: &serde_json::Value,
    field: &str,
    expected: &str,
) -> Result<(), ExactOracleError> {
    if required_string(value, field)? != expected {
        return Err(ExactOracleError::InvalidValue {
            path: field.to_owned(),
            expected: expected.to_owned(),
        });
    }
    Ok(())
}

fn require_u64(
    value: &serde_json::Value,
    field: &str,
    expected: u64,
) -> Result<(), ExactOracleError> {
    if required_u64(value, field)? != expected {
        return Err(ExactOracleError::InvalidValue {
            path: field.to_owned(),
            expected: expected.to_string(),
        });
    }
    Ok(())
}

fn require_string_array(
    value: &serde_json::Value,
    field: &str,
    expected: &[&str],
) -> Result<(), ExactOracleError> {
    let actual = required_array(value, field)?
        .iter()
        .map(|item| {
            item.as_str()
                .ok_or_else(|| ExactOracleError::InvalidType(format!("{field}[]")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if actual != expected {
        return Err(ExactOracleError::InvalidValue {
            path: field.to_owned(),
            expected: expected.join(", "),
        });
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum OracleVerificationError {
    #[error(transparent)]
    GeneratorContract(#[from] crate::ManifestContractError),
    #[error(transparent)]
    IdentityContract(#[from] crate::IdentityContractError),
    #[error(transparent)]
    StageContract(#[from] crate::StageContractError),
    #[error(transparent)]
    Producer(#[from] crate::IdentityGenerationError),
    #[error(transparent)]
    Oracle(#[from] ExactOracleError),
    #[error(transparent)]
    StageProducer(#[from] crate::StageGenerationError),
    #[error(transparent)]
    StageOracle(#[from] crate::StageOracleError),
    #[error("生产者与独立预言机结果不一致：graphProfile={graph_profile:?}, N={n}")]
    Mismatch {
        graph_profile: GraphProfileId,
        n: u32,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ExactOracleError {
    #[error("独立预言机缺少清单路径 {0}")]
    MissingPath(String),
    #[error("独立预言机清单字段类型错误：{0}")]
    InvalidType(String),
    #[error("独立预言机无法解析整数：{0}")]
    InvalidInteger(String),
    #[error("独立预言机字段 {path} 不匹配：期望 {expected}")]
    InvalidValue { path: String, expected: String },
    #[error("独立预言机发现重复实体种类代码 {0}")]
    DuplicateEntityKind(u16),
    #[error("独立预言机发现未知实体种类 {0}")]
    UnknownEntityKind(String),
    #[error("独立预言机发现重复 StableId 引用槽")]
    DuplicateReferenceSlot,
    #[error(
        "独立预言机找不到 StableId 引用槽：source={source_entity_kind}, target={target_entity_kind}, slot={slot}"
    )]
    UnknownReferenceSlot {
        source_entity_kind: String,
        target_entity_kind: u16,
        slot: u32,
    },
    #[error(
        "独立预言机无法解析 StableId：unit={unit_index}, source={source_kind}, target={target_kind}"
    )]
    UnresolvedStableId {
        unit_index: u32,
        source_kind: u16,
        target_kind: u16,
    },
    #[error("独立预言机发现重复声明：unit={unit_index}, kind={entity_kind}")]
    DuplicateDeclaration { unit_index: u32, entity_kind: u16 },
    #[error("独立预言机缺少声明：unit={unit_index}, kind={entity_kind}")]
    MissingDeclaration { unit_index: u32, entity_kind: u16 },
    #[error("独立预言机发现实体种类 {entity_kind} 内 StableId 冲突")]
    DuplicateStableId { entity_kind: u16 },
    #[error("独立预言机发现重复规范记录")]
    DuplicateRecord,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_repository_contract;

    #[test]
    fn exact_oracle_matches_the_producer_for_n1_and_n2() {
        let trusted = load_repository_contract().expect("frozen contract");
        let report = verify_identity_oracle_matrix(&trusted).expect("oracle matrix");

        assert_eq!(report.checked_cases, 6);
        assert_eq!(report.checked_n1_cases, 3);
        assert_eq!(report.checked_n2_cases, 3);
        assert_eq!(report.checked_stage_cases, 6);
    }

    #[test]
    fn exact_oracle_rejects_an_unregistered_stable_id_slot() {
        let trusted = load_repository_contract().expect("frozen contract");
        let mut manifest = trusted.workload_manifest;
        manifest["identityBindings"][6]["fields"][3]["value"] =
            serde_json::json!("stable-id(kind=4,local=9)");

        assert!(matches!(
            build_identity_oracle_case(&manifest, GraphProfileId::WideStar, 1),
            Err(ExactOracleError::UnknownReferenceSlot { .. })
        ));
    }

    #[test]
    fn exact_oracle_rejects_unused_reference_slots_and_wrong_profiled_key_kinds() {
        let trusted = load_repository_contract().expect("frozen contract");
        let mut manifest_with_extra_slot = trusted.workload_manifest.clone();
        manifest_with_extra_slot["identityBindingExpansion"]["stableIdExpression"]
            ["referenceSlots"]
            .as_array_mut()
            .expect("reference slots")
            .push(serde_json::json!({
                "sourceEntityKind": "RoadCorridor",
                "targetEntityKindCode": 1,
                "slot": 99,
                "role": "unused"
            }));
        assert!(matches!(
            build_identity_oracle_case(&manifest_with_extra_slot, GraphProfileId::WideStar, 1),
            Err(ExactOracleError::InvalidValue { .. })
        ));

        let mut manifest_with_wrong_kind = trusted.workload_manifest;
        manifest_with_wrong_kind["identityBindings"][0]["fields"][1]["value"] =
            serde_json::json!("profiled-key(kind=2,local=0)");
        assert!(matches!(
            build_identity_oracle_case(&manifest_with_wrong_kind, GraphProfileId::WideStar, 1),
            Err(ExactOracleError::InvalidValue { .. })
        ));
    }

    #[test]
    fn n2_owner_ordinals_are_dense_within_each_entity_kind() {
        let trusted = load_repository_contract().expect("frozen contract");
        let output =
            build_identity_oracle_case(&trusted.workload_manifest, GraphProfileId::WideStar, 2)
                .expect("N=2 oracle case");

        assert_eq!(output.declarations.len(), 44);
        assert_eq!(output.records.len(), 64);
        for entity_kind_code in 1_u16..=22 {
            let ordinals = output
                .records
                .iter()
                .filter(|record| {
                    record.record_kind == 1 && record.entity_kind_code == entity_kind_code
                })
                .map(|record| record.owner_ordinal)
                .collect::<BTreeSet<_>>();
            assert_eq!(ordinals, BTreeSet::from([0, 1]));
        }
    }
}
