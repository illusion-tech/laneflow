//! `LF-COMP-CORRIDOR-v1` 的独立完整记录流预言机。
//!
//! 本模块只共享已验证的不可变模板值类型，不调用生产者的身份编码、StableId 派生、
//! 关系展开、局部序号分配、规范排序或记录流编码函数。

use crate::corridor::{
    CORRIDOR_WORKLOAD_ID, CorridorContract, CorridorError, CorridorTemplate, EntityRef,
    TemplateGeometryRule, TemplateRelation, build_corridor_stage_case,
};
use crate::identity::SemanticRecord;
use crate::{GraphProfileId, TrustedContract};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

const ABSENT_LOCAL_INDEX: u32 = u32::MAX;
const STABLE_ID_DOMAIN: &[u8] = b"laneflow.stable-id.v1\0";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorridorOracleVerificationReport {
    pub checked_cases: u32,
    pub checked_n1_cases: u32,
    pub checked_n2_cases: u32,
    pub production_loader_fixture_sets: u32,
    pub independent_template_projection_checked: bool,
}

pub fn verify_corridor_oracle_matrix(
    trusted: &TrustedContract,
) -> Result<CorridorOracleVerificationReport, CorridorOracleError> {
    let contract = CorridorContract::from_manifest(&trusted.workload_manifest)?;
    let template = contract.load_template()?;
    let mut checked_cases = 0_u32;
    for graph_profile in GraphProfileId::ALL {
        for n in [1, 2] {
            verify_corridor_oracle_case_with_template(
                trusted,
                &contract,
                &template,
                graph_profile,
                n,
            )?;
            checked_cases = checked_cases
                .checked_add(1)
                .ok_or_else(|| CorridorOracleError::Contract("checkedCases overflow".to_owned()))?;
        }
    }
    Ok(CorridorOracleVerificationReport {
        checked_cases,
        checked_n1_cases: 3,
        checked_n2_cases: 3,
        production_loader_fixture_sets: 0,
        independent_template_projection_checked: true,
    })
}

fn verify_corridor_oracle_case_with_template(
    trusted: &TrustedContract,
    contract: &CorridorContract,
    template: &CorridorTemplate,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<crate::CorridorStageSummary, CorridorOracleError> {
    let generator = trusted.generator_contract()?;
    let identity = trusted.identity_contract()?;
    let stage = trusted.stage_contract()?;
    let produced = build_corridor_stage_case(
        &generator,
        &identity,
        &stage,
        contract,
        template,
        graph_profile,
        n,
    )?;
    let oracle = build_template_oracle_records(
        &trusted.workload_manifest,
        CORRIDOR_WORKLOAD_ID,
        template,
        graph_profile,
        n,
    )?;
    if produced.records != oracle.records
        || produced.semantic_record_stream != oracle.stream
        || produced.materialization.output != produced.semantic_record_stream
    {
        return Err(CorridorOracleError::Mismatch { graph_profile, n });
    }
    Ok(produced.summary)
}

pub(crate) struct OracleOutput {
    pub(crate) records: Vec<SemanticRecord>,
    pub(crate) stream: Vec<u8>,
    #[allow(dead_code)]
    pub(crate) declarations: Vec<OracleDeclarationIdentity>,
    #[allow(dead_code)]
    pub(crate) route_occurrences: Vec<OracleRouteOccurrence>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OracleDeclarationIdentity {
    pub(crate) unit: u32,
    pub(crate) entity: EntityRef,
    pub(crate) stable_id: [u8; 16],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OracleRouteOccurrence {
    pub(crate) unit: u32,
    pub(crate) relation_sequence_ordinal: u32,
    pub(crate) route_ordinal_within_unit: u32,
    pub(crate) reference_ordinal: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct OracleOwner {
    unit: u32,
    entity: EntityRef,
}

#[derive(Clone)]
struct OracleField {
    tag: u16,
    bytes: Vec<u8>,
}

#[derive(Clone)]
struct OracleDeclaration {
    owner: OracleOwner,
    stable_id: [u8; 16],
    fields: Vec<OracleField>,
}

#[derive(Clone, Copy)]
enum OracleFieldSource {
    Namespace,
    ProfiledKey { kind: u16, local: u32 },
    StableId { kind: u16 },
}

struct OracleBinding {
    entity_kind: String,
    fields: Vec<(u16, OracleFieldSource)>,
}

#[derive(Clone)]
enum OracleIndex {
    Absent,
    Explicit(u32),
    PayloadOrder,
    Gate(u32, [u8; 16]),
    Waiting(u32, u32, [u8; 16]),
}

#[derive(Clone)]
struct OraclePending {
    record_kind: u16,
    owner: OracleOwner,
    index: OracleIndex,
    payload: Vec<u8>,
}

pub(crate) fn build_template_oracle_records(
    manifest: &Value,
    workload_id: &str,
    template: &CorridorTemplate,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<OracleOutput, CorridorOracleError> {
    build_oracle_records(
        manifest,
        workload_id,
        template,
        OracleProjection::Scalable(graph_profile),
        n,
    )
}

#[cfg(feature = "fixture-oracle")]
pub(crate) fn build_fixed_fixture_oracle_records(
    manifest: &Value,
    workload_id: &str,
    template: &CorridorTemplate,
) -> Result<OracleOutput, CorridorOracleError> {
    build_oracle_records(
        manifest,
        workload_id,
        template,
        OracleProjection::FixedFixture,
        1,
    )
}

#[derive(Clone, Copy)]
enum OracleProjection {
    Scalable(GraphProfileId),
    #[cfg(feature = "fixture-oracle")]
    FixedFixture,
}

fn build_oracle_records(
    manifest: &Value,
    workload_id: &str,
    template: &CorridorTemplate,
    projection: OracleProjection,
    n: u32,
) -> Result<OracleOutput, CorridorOracleError> {
    if n == 0 {
        return Err(CorridorOracleError::Contract(
            "N must be positive".to_owned(),
        ));
    }
    let generator_version = value_u32(manifest, "generatorVersion")?;
    let base_seed = u64::from_str_radix(value_string(manifest, "baseSeedHexU64")?, 16)
        .map_err(|_| CorridorOracleError::Contract("baseSeedHexU64".to_owned()))?;
    let identity_version = u16::try_from(value_u64(manifest, "identityEncodingVersion")?)
        .map_err(|_| CorridorOracleError::Contract("identityEncodingVersion".to_owned()))?;
    let stream_version = value_u32(manifest, "semanticRecordStreamVersion")?;
    let stream_domain = value_string(manifest, "semanticRecordDomainUtf8NulTerminated")?;
    let namespace_domain = value_string(
        value_object(manifest, "namespaceDerivation")?,
        "domainUtf8NulTerminated",
    )?;
    let bindings = oracle_bindings(manifest)?;

    let mut declarations = Vec::with_capacity(
        usize::try_from(n)
            .expect("N must fit usize")
            .saturating_mul(template.entities.len()),
    );
    let mut stable_ids = BTreeMap::<OracleOwner, [u8; 16]>::new();
    for unit in 0..n {
        for entity in &template.entities {
            let (graph_profile_id, canonical_module_name) = match projection {
                OracleProjection::Scalable(graph_profile) => {
                    (graph_profile.as_str(), format!("unit/{unit:08x}"))
                }
                #[cfg(feature = "fixture-oracle")]
                OracleProjection::FixedFixture => (
                    "not-applicable",
                    if entity.reference.kind == 22 {
                        "spatial".to_owned()
                    } else {
                        "traffic".to_owned()
                    },
                ),
            };
            let namespace = oracle_namespace(
                namespace_domain,
                generator_version,
                base_seed,
                workload_id,
                graph_profile_id,
                &canonical_module_name,
            );
            let binding = bindings.get(&entity.reference.kind).ok_or_else(|| {
                CorridorOracleError::Contract(format!(
                    "missing identity binding {}",
                    entity.reference.kind
                ))
            })?;
            let profiled_count = u32::try_from(
                binding
                    .fields
                    .iter()
                    .filter(|(_, source)| matches!(source, OracleFieldSource::ProfiledKey { .. }))
                    .count(),
            )
            .expect("profiled field count must fit u32");
            let mut fields = Vec::with_capacity(binding.fields.len());
            for (tag, source) in &binding.fields {
                let bytes = match *source {
                    OracleFieldSource::Namespace => namespace.as_bytes().to_vec(),
                    OracleFieldSource::ProfiledKey { kind, local } => {
                        let expanded = entity
                            .reference
                            .local
                            .checked_mul(profiled_count)
                            .and_then(|base| base.checked_add(local))
                            .ok_or_else(|| {
                                CorridorOracleError::Contract(
                                    "profiled local index overflow".to_owned(),
                                )
                            })?;
                        match projection {
                            OracleProjection::Scalable(_) => {
                                format!("{kind:02x}/{unit:08x}/{expanded:08x}").into_bytes()
                            }
                            #[cfg(feature = "fixture-oracle")]
                            OracleProjection::FixedFixture => {
                                format!("fixture/{kind:02x}/{expanded:08x}").into_bytes()
                            }
                        }
                    }
                    OracleFieldSource::StableId { kind } => {
                        let target = entity.identity_references.get(tag).ok_or_else(|| {
                            CorridorOracleError::Contract(format!(
                                "missing identity reference kind={} local={} tag={tag}",
                                entity.reference.kind, entity.reference.local
                            ))
                        })?;
                        if target.kind != kind {
                            return Err(CorridorOracleError::Contract(format!(
                                "identity reference kind mismatch: expected {kind}, actual {}",
                                target.kind
                            )));
                        }
                        stable_ids
                            .get(&OracleOwner {
                                unit,
                                entity: *target,
                            })
                            .ok_or_else(|| {
                                CorridorOracleError::Contract(
                                    "unresolved stable identity reference".to_owned(),
                                )
                            })?
                            .to_vec()
                    }
                };
                fields.push(OracleField { tag: *tag, bytes });
            }
            let canonical = oracle_identity_bytes(identity_version, entity.reference.kind, &fields);
            let mut hasher = blake3::Hasher::new();
            hasher.update(STABLE_ID_DOMAIN);
            hasher.update(&canonical);
            let mut stable_id = [0_u8; 16];
            stable_id.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
            let owner = OracleOwner {
                unit,
                entity: entity.reference,
            };
            if stable_ids.insert(owner, stable_id).is_some() {
                return Err(CorridorOracleError::Contract(
                    "duplicate declaration".to_owned(),
                ));
            }
            declarations.push(OracleDeclaration {
                owner,
                stable_id,
                fields,
            });
        }
    }
    let owner_ordinals = oracle_owner_ordinals(&declarations)?;
    let mut route_sources = BTreeMap::new();
    for unit in 0..n {
        for (relation_sequence_ordinal, relation) in template.relations.iter().enumerate() {
            let TemplateRelation::RouteOccurrence { route, index, .. } = relation else {
                continue;
            };
            let stable_id = oracle_stable_id(&stable_ids, unit, *route)?;
            if route_sources
                .insert(
                    (stable_id, *index),
                    (
                        unit,
                        u32::try_from(relation_sequence_ordinal).map_err(|_| {
                            CorridorOracleError::Contract(
                                "route relation sequence ordinal overflow".to_owned(),
                            )
                        })?,
                    ),
                )
                .is_some()
            {
                return Err(CorridorOracleError::Contract(
                    "duplicate route occurrence source identity".to_owned(),
                ));
            }
        }
    }
    let mut pending = Vec::new();
    for declaration in &declarations {
        pending.push(OraclePending {
            record_kind: 1,
            owner: declaration.owner,
            index: OracleIndex::Absent,
            payload: oracle_identity_payload(&declaration.fields),
        });
    }
    for unit in 0..n {
        for relation in &template.relations {
            pending.push(oracle_relation(unit, relation, &stable_ids)?);
        }
        for point in &template.geometry {
            let (x_bits, y_bits, z_bits) = oracle_geometry_coordinate_bits(point, unit)?;
            let mut payload = Vec::with_capacity(32);
            payload.extend_from_slice(&oracle_stable_id(&stable_ids, unit, point.frame)?);
            put_u32(&mut payload, point.point_index);
            put_u32(&mut payload, x_bits);
            put_u32(&mut payload, y_bits);
            put_u32(&mut payload, z_bits);
            pending.push(OraclePending {
                record_kind: 5,
                owner: OracleOwner {
                    unit,
                    entity: point.edge,
                },
                index: OracleIndex::Explicit(point.point_index),
                payload,
            });
        }
    }
    assign_oracle_indexes(&mut pending);
    let mut records = Vec::with_capacity(pending.len());
    for record in pending {
        let stable_id = oracle_stable_id(&stable_ids, record.owner.unit, record.owner.entity)?;
        let local_index = match record.index {
            OracleIndex::Absent => ABSENT_LOCAL_INDEX,
            OracleIndex::Explicit(value) => value,
            OracleIndex::PayloadOrder | OracleIndex::Gate(..) | OracleIndex::Waiting(..) => {
                return Err(CorridorOracleError::Contract(
                    "unassigned oracle local index".to_owned(),
                ));
            }
        };
        let binding = bindings
            .get(&record.owner.entity.kind)
            .ok_or_else(|| CorridorOracleError::Contract("missing owner binding".to_owned()))?;
        records.push(SemanticRecord {
            record_kind: record.record_kind,
            entity_kind_code: record.owner.entity.kind,
            entity_kind: binding.entity_kind.clone(),
            stable_id,
            owner_ordinal: owner_ordinals[&(record.owner.entity.kind, stable_id)],
            local_index,
            payload: record.payload,
        });
    }
    records.sort_by(|left, right| left.canonical_key().cmp(&right.canonical_key()));
    let mut per_unit_route_ordinals = BTreeMap::<u32, u32>::new();
    let mut route_occurrences = Vec::with_capacity(route_sources.len());
    for (reference_ordinal, record) in records
        .iter()
        .filter(|record| record.record_kind == 4)
        .enumerate()
    {
        let (unit, relation_sequence_ordinal) = route_sources
            .get(&(record.stable_id, record.local_index))
            .copied()
            .ok_or_else(|| {
                CorridorOracleError::Contract(
                    "canonical route occurrence lacks source identity".to_owned(),
                )
            })?;
        let route_ordinal_within_unit = per_unit_route_ordinals.entry(unit).or_default();
        route_occurrences.push(OracleRouteOccurrence {
            unit,
            relation_sequence_ordinal,
            route_ordinal_within_unit: *route_ordinal_within_unit,
            reference_ordinal: u64::try_from(reference_ordinal).map_err(|_| {
                CorridorOracleError::Contract("route reference ordinal overflow".to_owned())
            })?,
        });
        *route_ordinal_within_unit = route_ordinal_within_unit.checked_add(1).ok_or_else(|| {
            CorridorOracleError::Contract("route ordinal within unit overflow".to_owned())
        })?;
    }
    if route_occurrences.len() != route_sources.len() {
        return Err(CorridorOracleError::Contract(
            "canonical route occurrence source coverage mismatch".to_owned(),
        ));
    }
    let declaration_identities = declarations
        .iter()
        .map(|declaration| OracleDeclarationIdentity {
            unit: declaration.owner.unit,
            entity: declaration.owner.entity,
            stable_id: declaration.stable_id,
        })
        .collect();
    let stream = oracle_stream(stream_domain, stream_version, &records);
    Ok(OracleOutput {
        records,
        stream,
        declarations: declaration_identities,
        route_occurrences,
    })
}

fn oracle_relation(
    unit: u32,
    relation: &TemplateRelation,
    stable_ids: &BTreeMap<OracleOwner, [u8; 16]>,
) -> Result<OraclePending, CorridorOracleError> {
    let (record_kind, entity, index, payload) = match relation {
        TemplateRelation::Owner { child, parent } => {
            let mut payload = Vec::with_capacity(18);
            put_u16(&mut payload, parent.kind);
            payload.extend_from_slice(&oracle_stable_id(stable_ids, unit, *parent)?);
            (2, *child, OracleIndex::Absent, payload)
        }
        TemplateRelation::EdgeConnection { source, target } => (
            3,
            *source,
            OracleIndex::PayloadOrder,
            oracle_stable_id(stable_ids, unit, *target)?.to_vec(),
        ),
        TemplateRelation::RouteOccurrence { route, index, edge } => {
            let mut payload = Vec::with_capacity(20);
            put_u32(&mut payload, *index);
            payload.extend_from_slice(&oracle_stable_id(stable_ids, unit, *edge)?);
            (4, *route, OracleIndex::Explicit(*index), payload)
        }
        TemplateRelation::Access {
            rule,
            participant,
            target,
            decision,
        } => {
            let mut payload = Vec::with_capacity(35);
            payload.extend_from_slice(&oracle_stable_id(stable_ids, unit, *participant)?);
            put_u16(&mut payload, target.kind);
            payload.extend_from_slice(&oracle_stable_id(stable_ids, unit, *target)?);
            payload.push(*decision);
            (6, *rule, OracleIndex::PayloadOrder, payload)
        }
        TemplateRelation::SignalGroup { group, gate } => (
            7,
            *group,
            OracleIndex::PayloadOrder,
            oracle_stable_id(stable_ids, unit, *gate)?.to_vec(),
        ),
        TemplateRelation::PhaseState {
            phase,
            group,
            state,
        } => {
            let mut payload = Vec::with_capacity(17);
            payload.extend_from_slice(&oracle_stable_id(stable_ids, unit, *group)?);
            payload.push(*state);
            (8, *phase, OracleIndex::PayloadOrder, payload)
        }
        TemplateRelation::Gate {
            path,
            transition_index,
            gate,
            stop_line,
            edge,
            edge_position_bits,
        } => {
            let gate_id = oracle_stable_id(stable_ids, unit, *gate)?;
            let mut payload = Vec::with_capacity(56);
            put_u32(&mut payload, 0);
            payload.extend_from_slice(&gate_id);
            payload.extend_from_slice(&oracle_stable_id(stable_ids, unit, *stop_line)?);
            payload.extend_from_slice(&oracle_stable_id(stable_ids, unit, *edge)?);
            put_u32(&mut payload, *edge_position_bits);
            (
                9,
                *path,
                OracleIndex::Gate(*transition_index, gate_id),
                payload,
            )
        }
        TemplateRelation::WaitingZone {
            path,
            entry_transition_index,
            release_transition_index,
            zone,
            before_gate,
            after_gate,
            capacity,
        } => {
            let zone_id = oracle_stable_id(stable_ids, unit, *zone)?;
            let mut payload = Vec::with_capacity(56);
            put_u32(&mut payload, 0);
            payload.extend_from_slice(&zone_id);
            payload.extend_from_slice(&oracle_stable_id(stable_ids, unit, *before_gate)?);
            payload.extend_from_slice(&oracle_stable_id(stable_ids, unit, *after_gate)?);
            put_u32(&mut payload, *capacity);
            (
                10,
                *path,
                OracleIndex::Waiting(*entry_transition_index, *release_transition_index, zone_id),
                payload,
            )
        }
        TemplateRelation::Parking {
            space,
            entry_edge,
            entry_high_bits,
            entry_residual_bits,
            exit_edge,
            exit_high_bits,
            exit_residual_bits,
        } => {
            let mut payload = Vec::with_capacity(64);
            payload.extend_from_slice(&oracle_stable_id(stable_ids, unit, *space)?);
            payload.extend_from_slice(&oracle_stable_id(stable_ids, unit, *entry_edge)?);
            put_u32(&mut payload, *entry_high_bits);
            put_u32(&mut payload, *entry_residual_bits);
            payload.extend_from_slice(&oracle_stable_id(stable_ids, unit, *exit_edge)?);
            put_u32(&mut payload, *exit_high_bits);
            put_u32(&mut payload, *exit_residual_bits);
            (11, *space, OracleIndex::Absent, payload)
        }
        TemplateRelation::LaneCoverage { lane, index, edge } => {
            let mut payload = Vec::with_capacity(20);
            put_u32(&mut payload, *index);
            payload.extend_from_slice(&oracle_stable_id(stable_ids, unit, *edge)?);
            (12, *lane, OracleIndex::Explicit(*index), payload)
        }
        TemplateRelation::JunctionInternalEdge { junction, edge } => (
            13,
            *junction,
            OracleIndex::PayloadOrder,
            oracle_stable_id(stable_ids, unit, *edge)?.to_vec(),
        ),
    };
    Ok(OraclePending {
        record_kind,
        owner: OracleOwner { unit, entity },
        index,
        payload,
    })
}

fn assign_oracle_indexes(records: &mut [OraclePending]) {
    let mut payload = BTreeMap::<(u16, OracleOwner), Vec<usize>>::new();
    let mut gates = BTreeMap::<OracleOwner, Vec<usize>>::new();
    let mut waiting = BTreeMap::<OracleOwner, Vec<usize>>::new();
    for (index, record) in records.iter().enumerate() {
        match record.index {
            OracleIndex::PayloadOrder => {
                payload
                    .entry((record.record_kind, record.owner))
                    .or_default()
                    .push(index);
            }
            OracleIndex::Gate(..) => gates.entry(record.owner).or_default().push(index),
            OracleIndex::Waiting(..) => waiting.entry(record.owner).or_default().push(index),
            OracleIndex::Absent | OracleIndex::Explicit(_) => {}
        }
    }
    for indexes in payload.values_mut() {
        indexes.sort_by(|left, right| records[*left].payload.cmp(&records[*right].payload));
        for (ordinal, index) in indexes.iter().copied().enumerate() {
            records[index].index = OracleIndex::Explicit(ordinal as u32);
        }
    }
    for indexes in gates.values_mut() {
        indexes.sort_by_key(|index| match records[*index].index {
            OracleIndex::Gate(transition, stable_id) => (transition, stable_id),
            _ => unreachable!(),
        });
        for (ordinal, index) in indexes.iter().copied().enumerate() {
            let ordinal = ordinal as u32;
            records[index].payload[..4].copy_from_slice(&ordinal.to_le_bytes());
            records[index].index = OracleIndex::Explicit(ordinal);
        }
    }
    for indexes in waiting.values_mut() {
        indexes.sort_by_key(|index| match records[*index].index {
            OracleIndex::Waiting(entry, release, stable_id) => (entry, release, stable_id),
            _ => unreachable!(),
        });
        for (ordinal, index) in indexes.iter().copied().enumerate() {
            let ordinal = ordinal as u32;
            records[index].payload[..4].copy_from_slice(&ordinal.to_le_bytes());
            records[index].index = OracleIndex::Explicit(ordinal);
        }
    }
}

fn oracle_bindings(manifest: &Value) -> Result<BTreeMap<u16, OracleBinding>, CorridorOracleError> {
    let raw = manifest
        .get("identityBindings")
        .and_then(Value::as_array)
        .ok_or_else(|| CorridorOracleError::Contract("identityBindings".to_owned()))?;
    let mut bindings = BTreeMap::new();
    for binding in raw {
        let kind = u16::try_from(value_u64(binding, "entityKindCode")?)
            .map_err(|_| CorridorOracleError::Contract("entityKindCode".to_owned()))?;
        let entity_kind = value_string(binding, "entityKind")?.to_owned();
        let raw_fields = binding
            .get("fields")
            .and_then(Value::as_array)
            .ok_or_else(|| CorridorOracleError::Contract("identity fields".to_owned()))?;
        let mut fields = Vec::with_capacity(raw_fields.len());
        for field in raw_fields {
            let tag = u16::try_from(value_u64(field, "tag")?)
                .map_err(|_| CorridorOracleError::Contract("identity tag".to_owned()))?;
            fields.push((tag, parse_field_source(value_string(field, "value")?)?));
        }
        if bindings
            .insert(
                kind,
                OracleBinding {
                    entity_kind,
                    fields,
                },
            )
            .is_some()
        {
            return Err(CorridorOracleError::Contract(
                "duplicate identity binding".to_owned(),
            ));
        }
    }
    Ok(bindings)
}

fn parse_field_source(value: &str) -> Result<OracleFieldSource, CorridorOracleError> {
    if value == "namespace" {
        return Ok(OracleFieldSource::Namespace);
    }
    if let Some(inner) = value
        .strip_prefix("profiled-key(kind=")
        .and_then(|value| value.strip_suffix(')'))
    {
        let (kind, local) = inner
            .split_once(",local=")
            .ok_or_else(|| CorridorOracleError::Contract(value.to_owned()))?;
        return Ok(OracleFieldSource::ProfiledKey {
            kind: kind
                .parse()
                .map_err(|_| CorridorOracleError::Contract(value.to_owned()))?,
            local: local
                .parse()
                .map_err(|_| CorridorOracleError::Contract(value.to_owned()))?,
        });
    }
    if let Some(inner) = value
        .strip_prefix("stable-id(kind=")
        .and_then(|value| value.strip_suffix(')'))
    {
        let (kind, _local) = inner
            .split_once(",local=")
            .ok_or_else(|| CorridorOracleError::Contract(value.to_owned()))?;
        return Ok(OracleFieldSource::StableId {
            kind: kind
                .parse()
                .map_err(|_| CorridorOracleError::Contract(value.to_owned()))?,
        });
    }
    Err(CorridorOracleError::Contract(format!(
        "unsupported identity field source: {value}"
    )))
}

fn oracle_namespace(
    domain: &str,
    generator_version: u32,
    base_seed: u64,
    workload_id: &str,
    graph_profile: &str,
    module_name: &str,
) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(domain.as_bytes());
    bytes.push(0);
    put_u32(&mut bytes, generator_version);
    put_u64(&mut bytes, base_seed);
    put_string(&mut bytes, workload_id);
    put_string(&mut bytes, graph_profile);
    put_string(&mut bytes, module_name);
    hex(&blake3::hash(&bytes).as_bytes()[..16])
}

fn oracle_geometry_coordinate_bits(
    point: &crate::corridor::TemplateGeometry,
    unit: u32,
) -> Result<(u32, u32, u32), CorridorOracleError> {
    match point.coordinate_rule {
        TemplateGeometryRule::Fixed => {
            return Ok((point.x_bits, point.y_bits, point.z_bits));
        }
        TemplateGeometryRule::JunctionGridV1 => {}
    }
    let unit_x = unit % 4_096;
    let unit_y = unit / 4_096;
    let x = unit_x
        .checked_mul(128)
        .and_then(|base| {
            point
                .edge
                .local
                .checked_mul(2)
                .and_then(|edge| base.checked_add(edge))
        })
        .and_then(|base| base.checked_add(point.point_index))
        .ok_or_else(|| CorridorOracleError::Contract("junction grid x overflow".to_owned()))?;
    let y = unit_y
        .checked_mul(128)
        .ok_or_else(|| CorridorOracleError::Contract("junction grid y overflow".to_owned()))?;
    Ok((
        (x as f32).to_bits(),
        (y as f32).to_bits(),
        0.0_f32.to_bits(),
    ))
}

fn oracle_identity_bytes(version: u16, kind: u16, fields: &[OracleField]) -> Vec<u8> {
    let mut bytes = b"LFID".to_vec();
    put_u16(&mut bytes, version);
    put_u16(&mut bytes, kind);
    bytes.extend_from_slice(&oracle_identity_payload(fields));
    bytes
}

fn oracle_identity_payload(fields: &[OracleField]) -> Vec<u8> {
    let mut bytes = Vec::new();
    put_u16(&mut bytes, fields.len() as u16);
    for field in fields {
        put_u16(&mut bytes, field.tag);
        put_u32(&mut bytes, field.bytes.len() as u32);
        bytes.extend_from_slice(&field.bytes);
    }
    bytes
}

fn oracle_owner_ordinals(
    declarations: &[OracleDeclaration],
) -> Result<BTreeMap<(u16, [u8; 16]), u32>, CorridorOracleError> {
    let mut by_kind = BTreeMap::<u16, Vec<[u8; 16]>>::new();
    for declaration in declarations {
        by_kind
            .entry(declaration.owner.entity.kind)
            .or_default()
            .push(declaration.stable_id);
    }
    let mut result = BTreeMap::new();
    for (kind, stable_ids) in &mut by_kind {
        stable_ids.sort_unstable();
        for (ordinal, stable_id) in stable_ids.iter().enumerate() {
            if result.insert((*kind, *stable_id), ordinal as u32).is_some() {
                return Err(CorridorOracleError::Contract(
                    "duplicate StableId128".to_owned(),
                ));
            }
        }
    }
    Ok(result)
}

fn oracle_stable_id(
    stable_ids: &BTreeMap<OracleOwner, [u8; 16]>,
    unit: u32,
    entity: EntityRef,
) -> Result<[u8; 16], CorridorOracleError> {
    stable_ids
        .get(&OracleOwner { unit, entity })
        .copied()
        .ok_or_else(|| CorridorOracleError::Contract("missing StableId128".to_owned()))
}

fn oracle_stream(domain: &str, version: u32, records: &[SemanticRecord]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(domain.as_bytes());
    bytes.push(0);
    put_u32(&mut bytes, version);
    put_u64(&mut bytes, records.len() as u64);
    for record in records {
        put_u16(&mut bytes, record.record_kind);
        put_u16(&mut bytes, record.entity_kind_code);
        bytes.extend_from_slice(&record.stable_id);
        put_u32(&mut bytes, record.owner_ordinal);
        put_u32(&mut bytes, record.local_index);
        put_u64(&mut bytes, record.payload.len() as u64);
        bytes.extend_from_slice(&record.payload);
    }
    bytes
}

fn value_object<'a>(value: &'a Value, field: &str) -> Result<&'a Value, CorridorOracleError> {
    let child = value
        .get(field)
        .ok_or_else(|| CorridorOracleError::Contract(field.to_owned()))?;
    child
        .as_object()
        .ok_or_else(|| CorridorOracleError::Contract(field.to_owned()))?;
    Ok(child)
}

fn value_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, CorridorOracleError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| CorridorOracleError::Contract(field.to_owned()))
}

fn value_u64(value: &Value, field: &str) -> Result<u64, CorridorOracleError> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| CorridorOracleError::Contract(field.to_owned()))
}

fn value_u32(value: &Value, field: &str) -> Result<u32, CorridorOracleError> {
    u32::try_from(value_u64(value, field)?)
        .map_err(|_| CorridorOracleError::Contract(field.to_owned()))
}

fn put_string(bytes: &mut Vec<u8>, value: &str) {
    put_u32(bytes, value.len() as u32);
    bytes.extend_from_slice(value.as_bytes());
}

fn put_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn hex(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut result, "{byte:02x}").expect("writing to String cannot fail");
    }
    result
}

#[derive(Debug, thiserror::Error)]
pub enum CorridorOracleError {
    #[error(transparent)]
    Corridor(#[from] CorridorError),
    #[error(transparent)]
    GeneratorContract(#[from] crate::ManifestContractError),
    #[error(transparent)]
    IdentityContract(#[from] crate::IdentityContractError),
    #[error(transparent)]
    StageContract(#[from] crate::stage::StageContractError),
    #[error("走廊独立预言机与生产者不一致：graphProfile={graph_profile:?}, N={n}")]
    Mismatch {
        graph_profile: GraphProfileId,
        n: u32,
    },
    #[error("走廊独立预言机契约错误：{0}")]
    Contract(String),
    #[error("当前生产加载器拒绝绑定夹具：{0}")]
    ProductionLoader(String),
    #[error("生产者夹具模板与 production loader 独立投影不一致：{0}")]
    TemplateProjectionMismatch(String),
}

#[cfg(feature = "fixture-oracle")]
fn describe_template_mismatch(
    producer: &CorridorTemplate,
    independent: &CorridorTemplate,
) -> String {
    if producer.entities != independent.entities {
        let index = producer
            .entities
            .iter()
            .zip(&independent.entities)
            .position(|(left, right)| left != right);
        return format!(
            "entities producer={} independent={} firstMismatch={index:?}",
            producer.entities.len(),
            independent.entities.len()
        );
    }
    if producer.relations != independent.relations {
        let index = producer
            .relations
            .iter()
            .zip(&independent.relations)
            .position(|(left, right)| left != right);
        return format!(
            "relations producer={} independent={} firstMismatch={index:?} producerValue={:?} independentValue={:?}",
            producer.relations.len(),
            independent.relations.len(),
            index.and_then(|value| producer.relations.get(value)),
            index.and_then(|value| independent.relations.get(value))
        );
    }
    let index = producer
        .geometry
        .iter()
        .zip(&independent.geometry)
        .position(|(left, right)| left != right);
    format!(
        "geometry producer={} independent={} firstMismatch={index:?} producerValue={:?} independentValue={:?}",
        producer.geometry.len(),
        independent.geometry.len(),
        index.and_then(|value| producer.geometry.get(value)),
        index.and_then(|value| independent.geometry.get(value))
    )
}

#[cfg(all(test, feature = "fixture-oracle"))]
mod tests {
    use super::*;

    #[test]
    fn complete_corridor_oracle_matrix_matches_full_record_streams() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let report = verify_corridor_oracle_matrix(&trusted).expect("corridor oracle matrix");
        assert_eq!(report.checked_cases, 6);
        assert_eq!(report.production_loader_fixture_sets, 0);
        assert!(report.independent_template_projection_checked);
    }
}
