use crate::{GeneratorContract, GraphProfileId, expand_module_graph};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const IDENTITY_KNOWN_VECTOR_SCHEMA: &str =
    "laneflow.compiler-calibration-identity-known-vectors";
pub const IDENTITY_WORKLOAD_ID: &str = "LF-COMP-ID-v1";
pub const SHORT_UNIQUE_STRING_PROFILE_ID: &str = "short-unique-v1";

pub(crate) const STABLE_ID_DOMAIN: &[u8] = b"laneflow.stable-id.v1\0";
pub(crate) const IDENTITY_MAGIC: &[u8; 4] = b"LFID";
pub(crate) const ABSENT_LOCAL_INDEX: u32 = u32::MAX;
#[cfg(test)]
const IDENTITY_KNOWN_VECTOR_BYTE_LENGTH: usize = 108_777;
#[cfg(test)]
const IDENTITY_KNOWN_VECTOR_SHA256: &str =
    "93526846510660d1fe4a9251997d64ca7a3e7ab38d34eddb77efe539074d93fc";

const EXPECTED_ENTITY_KINDS: [&str; 22] = [
    "RoadCorridor",
    "RoadSection",
    "AuthoringLane",
    "LaneEdge",
    "Junction",
    "Movement",
    "ManeuverPath",
    "ManeuverGate",
    "WaitingZone",
    "StopLine",
    "SignalGroup",
    "SignalController",
    "SignalPhase",
    "ParkingArea",
    "ParkingSpace",
    "LaneGroup",
    "FacilityBand",
    "ParticipantClass",
    "AccessRule",
    "VehicleProfile",
    "StaticRoute",
    "CanonicalFrame",
];

const EXPECTED_IDENTITY_FIELDS: [&[(u16, &str)]; 22] = [
    &[(1, "namespace"), (2, "profiled-key(kind=1,local=0)")],
    &[
        (1, "namespace"),
        (3, "profiled-key(kind=2,local=0)"),
        (33, "stable-id(kind=1,local=0)"),
    ],
    &[
        (1, "namespace"),
        (4, "profiled-key(kind=3,local=0)"),
        (32, "stable-id(kind=2,local=0)"),
    ],
    &[(1, "namespace"), (5, "profiled-key(kind=4,local=0)")],
    &[(1, "namespace"), (6, "profiled-key(kind=5,local=0)")],
    &[
        (1, "namespace"),
        (8, "profiled-key(kind=6,local=0)"),
        (9, "profiled-key(kind=6,local=1)"),
        (10, "profiled-key(kind=6,local=2)"),
        (34, "stable-id(kind=5,local=0)"),
    ],
    &[
        (1, "namespace"),
        (7, "profiled-key(kind=7,local=0)"),
        (11, "stable-id(kind=6,local=0)"),
        (12, "stable-id(kind=4,local=0)"),
        (13, "stable-id(kind=4,local=1)"),
    ],
    &[
        (1, "namespace"),
        (14, "stable-id(kind=7,local=0)"),
        (15, "profiled-key(kind=8,local=0)"),
    ],
    &[
        (1, "namespace"),
        (14, "stable-id(kind=7,local=0)"),
        (16, "profiled-key(kind=9,local=0)"),
    ],
    &[(1, "namespace"), (17, "profiled-key(kind=10,local=0)")],
    &[(1, "namespace"), (18, "profiled-key(kind=11,local=0)")],
    &[(1, "namespace"), (19, "profiled-key(kind=12,local=0)")],
    &[
        (1, "namespace"),
        (20, "stable-id(kind=12,local=0)"),
        (21, "profiled-key(kind=13,local=0)"),
    ],
    &[(1, "namespace"), (22, "profiled-key(kind=14,local=0)")],
    &[(1, "namespace"), (24, "profiled-key(kind=15,local=0)")],
    &[
        (1, "namespace"),
        (25, "profiled-key(kind=16,local=0)"),
        (32, "stable-id(kind=2,local=0)"),
    ],
    &[
        (1, "namespace"),
        (26, "profiled-key(kind=17,local=0)"),
        (33, "stable-id(kind=1,local=0)"),
    ],
    &[(1, "namespace"), (27, "profiled-key(kind=18,local=0)")],
    &[(1, "namespace"), (28, "profiled-key(kind=19,local=0)")],
    &[(1, "namespace"), (29, "profiled-key(kind=20,local=0)")],
    &[(1, "namespace"), (30, "profiled-key(kind=21,local=0)")],
    &[(1, "namespace"), (31, "profiled-key(kind=22,local=0)")],
];

const EXPECTED_OWNER_RELATIONS: [(&str, &str); 10] = [
    ("RoadSection", "RoadCorridor"),
    ("AuthoringLane", "RoadSection"),
    ("Movement", "Junction"),
    ("ManeuverPath", "Movement"),
    ("ManeuverGate", "ManeuverPath"),
    ("WaitingZone", "ManeuverPath"),
    ("SignalPhase", "SignalController"),
    ("ParkingSpace", "ParkingArea"),
    ("LaneGroup", "RoadSection"),
    ("FacilityBand", "RoadCorridor"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityContract {
    identity_encoding_version: u16,
    semantic_record_stream_version: u32,
    semantic_record_domain: String,
    pub(crate) bindings: Vec<IdentityBinding>,
    pub(crate) owner_relations: Vec<OwnerRelation>,
}

impl IdentityContract {
    pub fn from_manifest(value: &serde_json::Value) -> Result<Self, IdentityContractError> {
        let projection: IdentityManifestProjection =
            serde_json::from_value(value.clone()).map_err(IdentityContractError::InvalidShape)?;

        expect_eq(
            "identityEncodingVersion",
            projection.identity_encoding_version,
            1,
        )?;
        expect_eq(
            "identityRegistryRevision",
            projection.identity_registry_revision,
            1,
        )?;
        expect_eq(
            "semanticRecordStreamVersion",
            projection.semantic_record_stream_version,
            1,
        )?;
        expect_eq(
            "semanticRecordDomainUtf8NulTerminated",
            projection
                .semantic_record_domain_utf8_nul_terminated
                .as_str(),
            "LANEFLOW-COMPILER-CALIBRATION-SEMANTIC-V1",
        )?;
        validate_semantic_record_contract(
            &projection.semantic_record_envelope,
            &projection.semantic_record_envelope_rules,
            &projection.record_kinds,
        )?;
        validate_string_profiles(&projection.string_profiles)?;
        let bindings = validate_identity_bindings(&projection.identity_bindings)?;
        let owner_relations = validate_identity_workload(&projection.workloads)?;

        Ok(Self {
            identity_encoding_version: projection.identity_encoding_version,
            semantic_record_stream_version: projection.semantic_record_stream_version,
            semantic_record_domain: projection.semantic_record_domain_utf8_nul_terminated,
            bindings,
            owner_relations,
        })
    }

    pub(crate) const fn identity_encoding_version(&self) -> u16 {
        self.identity_encoding_version
    }

    pub(crate) const fn semantic_record_stream_version(&self) -> u32 {
        self.semantic_record_stream_version
    }

    pub(crate) fn semantic_record_domain(&self) -> &str {
        &self.semantic_record_domain
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IdentityBinding {
    pub(crate) entity_kind_code: u16,
    pub(crate) entity_kind: String,
    pub(crate) fields: Vec<IdentityFieldBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IdentityFieldBinding {
    pub(crate) tag: u16,
    pub(crate) value: IdentityFieldValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IdentityFieldValue {
    Namespace,
    ProfiledKey { kind: u16, local: u32 },
    StableId { kind: u16, local: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OwnerRelation {
    pub(crate) child_kind: u16,
    pub(crate) parent_kind: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityKnownVectorDocument {
    pub schema: &'static str,
    pub schema_version: u32,
    pub source_workload_manifest_sha256: String,
    pub workload_id: &'static str,
    pub n: u32,
    pub string_profile: &'static str,
    pub vectors: Vec<IdentityKnownVector>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityKnownVector {
    pub graph_profile: GraphProfileId,
    pub namespace_id: String,
    pub declarations: Vec<IdentityDeclarationVector>,
    pub records: Vec<SemanticRecordVector>,
    pub semantic_record_stream_byte_length: usize,
    pub semantic_digest_sha256: String,
    pub semantic_record_stream_hex: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityDeclarationVector {
    pub unit_index: u32,
    pub entity_kind_code: u16,
    pub entity_kind: String,
    pub stable_id: String,
    pub canonical_identity_bytes_hex: String,
    pub fields: Vec<IdentityFieldVector>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityFieldVector {
    pub tag: u16,
    pub field_bytes_hex: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticRecordVector {
    pub record_kind: u16,
    pub entity_kind_code: u16,
    pub entity_kind: String,
    pub stable_id: String,
    pub owner_ordinal: u32,
    pub local_index: u32,
    pub payload_hex: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IdentityCaseOutput {
    pub(crate) graph_profile: GraphProfileId,
    pub(crate) n: u32,
    pub(crate) unit_namespaces: Vec<String>,
    pub(crate) declarations: Vec<IdentityDeclarationVector>,
    pub(crate) records: Vec<SemanticRecordVector>,
    pub(crate) raw_records: Vec<SemanticRecord>,
    pub(crate) semantic_record_stream: Vec<u8>,
    pub(crate) semantic_digest_sha256: String,
}

pub fn build_identity_known_vectors(
    generator_contract: &GeneratorContract,
    identity_contract: &IdentityContract,
    workload_manifest_sha256: &str,
) -> Result<IdentityKnownVectorDocument, IdentityGenerationError> {
    let mut vectors = Vec::with_capacity(GraphProfileId::ALL.len());
    for graph_profile in GraphProfileId::ALL {
        vectors.push(build_identity_known_vector(
            generator_contract,
            identity_contract,
            graph_profile,
        )?);
    }

    Ok(IdentityKnownVectorDocument {
        schema: IDENTITY_KNOWN_VECTOR_SCHEMA,
        schema_version: 1,
        source_workload_manifest_sha256: workload_manifest_sha256.to_owned(),
        workload_id: IDENTITY_WORKLOAD_ID,
        n: 1,
        string_profile: SHORT_UNIQUE_STRING_PROFILE_ID,
        vectors,
    })
}

fn build_identity_known_vector(
    generator_contract: &GeneratorContract,
    identity_contract: &IdentityContract,
    graph_profile: GraphProfileId,
) -> Result<IdentityKnownVector, IdentityGenerationError> {
    let output = build_identity_case(generator_contract, identity_contract, graph_profile, 1)?;
    let namespace_id = output
        .unit_namespaces
        .first()
        .expect("N=1 identity case must have one namespace")
        .clone();

    Ok(IdentityKnownVector {
        graph_profile,
        namespace_id,
        declarations: output.declarations,
        records: output.records,
        semantic_record_stream_byte_length: output.semantic_record_stream.len(),
        semantic_digest_sha256: output.semantic_digest_sha256,
        semantic_record_stream_hex: encode_lower_hex(&output.semantic_record_stream),
    })
}

pub(crate) fn build_identity_case(
    generator_contract: &GeneratorContract,
    identity_contract: &IdentityContract,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<IdentityCaseOutput, IdentityGenerationError> {
    let graph = expand_module_graph(generator_contract, IDENTITY_WORKLOAD_ID, graph_profile, n)?;
    let mut unit_namespaces = Vec::with_capacity(usize::try_from(n).expect("N must fit usize"));
    let mut declarations = Vec::with_capacity(
        usize::try_from(n)
            .expect("N must fit usize")
            .saturating_mul(identity_contract.bindings.len()),
    );
    for unit_index in 0..n {
        let module_name = format!("unit/{unit_index:08x}");
        let namespace_id = graph
            .modules
            .iter()
            .find(|module| module.canonical_name == module_name)
            .ok_or(IdentityGenerationError::MissingUnitModule { unit_index })?
            .namespace_id
            .clone();
        declarations.extend(materialize_declarations(
            identity_contract,
            &namespace_id,
            unit_index,
        )?);
        unit_namespaces.push(namespace_id);
    }

    let records = materialize_records(identity_contract, &declarations)?;
    let semantic_record_stream = encode_semantic_record_stream(identity_contract, &records);
    let semantic_digest_sha256 = encode_lower_hex(&Sha256::digest(&semantic_record_stream));

    Ok(IdentityCaseOutput {
        graph_profile,
        n,
        unit_namespaces,
        declarations: declarations
            .iter()
            .map(IdentityDeclaration::to_vector)
            .collect(),
        records: records.iter().map(SemanticRecord::to_vector).collect(),
        raw_records: records,
        semantic_record_stream,
        semantic_digest_sha256,
    })
}

fn materialize_declarations(
    contract: &IdentityContract,
    namespace_id: &str,
    unit_index: u32,
) -> Result<Vec<IdentityDeclaration>, IdentityGenerationError> {
    let mut declarations_by_kind = BTreeMap::<u16, IdentityDeclaration>::new();
    for binding in &contract.bindings {
        let mut fields = Vec::with_capacity(binding.fields.len());
        for field in &binding.fields {
            let bytes = match field.value {
                IdentityFieldValue::Namespace => namespace_id.as_bytes().to_vec(),
                IdentityFieldValue::ProfiledKey { kind, local } => {
                    short_unique_profiled_key(kind, unit_index, local).into_bytes()
                }
                IdentityFieldValue::StableId { kind, local: _ } => declarations_by_kind
                    .get(&kind)
                    .ok_or(IdentityGenerationError::UnresolvedStableIdDependency {
                        source_kind: binding.entity_kind_code,
                        target_kind: kind,
                    })?
                    .stable_id
                    .to_vec(),
            };
            fields.push(IdentityField {
                tag: field.tag,
                bytes,
            });
        }

        let canonical_identity_bytes = encode_canonical_identity(
            contract.identity_encoding_version,
            binding.entity_kind_code,
            &fields,
        );
        let digest = blake3::hash(&[STABLE_ID_DOMAIN, &canonical_identity_bytes].concat());
        let mut stable_id = [0_u8; 16];
        stable_id.copy_from_slice(&digest.as_bytes()[..16]);

        declarations_by_kind.insert(
            binding.entity_kind_code,
            IdentityDeclaration {
                unit_index,
                entity_kind_code: binding.entity_kind_code,
                entity_kind: binding.entity_kind.clone(),
                stable_id,
                canonical_identity_bytes,
                fields,
            },
        );
    }

    Ok(declarations_by_kind.into_values().collect())
}

fn materialize_records(
    contract: &IdentityContract,
    declarations: &[IdentityDeclaration],
) -> Result<Vec<SemanticRecord>, IdentityGenerationError> {
    let by_unit_and_kind = declarations
        .iter()
        .map(|declaration| {
            (
                (declaration.unit_index, declaration.entity_kind_code),
                declaration,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut stable_ids_by_kind = BTreeMap::<u16, Vec<[u8; 16]>>::new();
    for declaration in declarations {
        stable_ids_by_kind
            .entry(declaration.entity_kind_code)
            .or_default()
            .push(declaration.stable_id);
    }
    let mut owner_ordinals = BTreeMap::<(u16, [u8; 16]), u32>::new();
    for (entity_kind, stable_ids) in &mut stable_ids_by_kind {
        stable_ids.sort_unstable();
        for (ordinal, stable_id) in stable_ids.iter().enumerate() {
            let ordinal = u32::try_from(ordinal).expect("same-kind identity count must fit u32");
            owner_ordinals.insert((*entity_kind, *stable_id), ordinal);
        }
    }
    let unit_count = declarations
        .iter()
        .map(|declaration| declaration.unit_index)
        .max()
        .map_or(0, |maximum| maximum + 1);
    let mut records = Vec::with_capacity(
        declarations.len() + contract.owner_relations.len() * unit_count as usize,
    );

    for declaration in declarations {
        records.push(SemanticRecord {
            record_kind: 1,
            entity_kind_code: declaration.entity_kind_code,
            entity_kind: declaration.entity_kind.clone(),
            stable_id: declaration.stable_id,
            owner_ordinal: *owner_ordinals
                .get(&(declaration.entity_kind_code, declaration.stable_id))
                .expect("every declaration must have an owner ordinal"),
            local_index: ABSENT_LOCAL_INDEX,
            payload: encode_identity_payload(&declaration.fields),
        });
    }

    for unit_index in 0..unit_count {
        for relation in &contract.owner_relations {
            let child = by_unit_and_kind
                .get(&(unit_index, relation.child_kind))
                .ok_or(IdentityGenerationError::MissingDeclaration {
                    unit_index,
                    entity_kind: relation.child_kind,
                })?;
            let parent = by_unit_and_kind
                .get(&(unit_index, relation.parent_kind))
                .ok_or(IdentityGenerationError::MissingDeclaration {
                    unit_index,
                    entity_kind: relation.parent_kind,
                })?;
            let mut payload = Vec::with_capacity(18);
            append_u16(&mut payload, relation.parent_kind);
            payload.extend_from_slice(&parent.stable_id);
            records.push(SemanticRecord {
                record_kind: 2,
                entity_kind_code: child.entity_kind_code,
                entity_kind: child.entity_kind.clone(),
                stable_id: child.stable_id,
                owner_ordinal: *owner_ordinals
                    .get(&(child.entity_kind_code, child.stable_id))
                    .expect("every child declaration must have an owner ordinal"),
                local_index: ABSENT_LOCAL_INDEX,
                payload,
            });
        }
    }

    records.sort_by(|left, right| left.canonical_key().cmp(&right.canonical_key()));
    Ok(records)
}

pub(crate) fn encode_canonical_identity(
    identity_encoding_version: u16,
    entity_kind_code: u16,
    fields: &[IdentityField],
) -> Vec<u8> {
    let mut encoded = Vec::new();
    encoded.extend_from_slice(IDENTITY_MAGIC);
    append_u16(&mut encoded, identity_encoding_version);
    append_u16(&mut encoded, entity_kind_code);
    encoded.extend_from_slice(&encode_identity_payload(fields));
    encoded
}

pub(crate) fn encode_identity_payload(fields: &[IdentityField]) -> Vec<u8> {
    let mut payload = Vec::new();
    append_u16(
        &mut payload,
        u16::try_from(fields.len()).expect("v1 identity field count must fit u16"),
    );
    for field in fields {
        append_u16(&mut payload, field.tag);
        append_u32(
            &mut payload,
            u32::try_from(field.bytes.len()).expect("v1 identity field length must fit u32"),
        );
        payload.extend_from_slice(&field.bytes);
    }
    payload
}

pub(crate) fn encode_semantic_record_stream(
    contract: &IdentityContract,
    records: &[SemanticRecord],
) -> Vec<u8> {
    let mut stream = Vec::new();
    stream.extend_from_slice(contract.semantic_record_domain.as_bytes());
    stream.push(0);
    append_u32(&mut stream, contract.semantic_record_stream_version);
    append_u64(
        &mut stream,
        u64::try_from(records.len()).expect("semantic record count must fit u64"),
    );
    for record in records {
        append_u16(&mut stream, record.record_kind);
        append_u16(&mut stream, record.entity_kind_code);
        stream.extend_from_slice(&record.stable_id);
        append_u32(&mut stream, record.owner_ordinal);
        append_u32(&mut stream, record.local_index);
        append_u64(
            &mut stream,
            u64::try_from(record.payload.len()).expect("semantic payload length must fit u64"),
        );
        stream.extend_from_slice(&record.payload);
    }
    stream
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IdentityField {
    pub(crate) tag: u16,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IdentityDeclaration {
    unit_index: u32,
    entity_kind_code: u16,
    entity_kind: String,
    stable_id: [u8; 16],
    canonical_identity_bytes: Vec<u8>,
    fields: Vec<IdentityField>,
}

impl IdentityDeclaration {
    fn to_vector(&self) -> IdentityDeclarationVector {
        IdentityDeclarationVector {
            unit_index: self.unit_index,
            entity_kind_code: self.entity_kind_code,
            entity_kind: self.entity_kind.clone(),
            stable_id: encode_lower_hex(&self.stable_id),
            canonical_identity_bytes_hex: encode_lower_hex(&self.canonical_identity_bytes),
            fields: self
                .fields
                .iter()
                .map(|field| IdentityFieldVector {
                    tag: field.tag,
                    field_bytes_hex: encode_lower_hex(&field.bytes),
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SemanticRecord {
    pub(crate) record_kind: u16,
    pub(crate) entity_kind_code: u16,
    pub(crate) entity_kind: String,
    pub(crate) stable_id: [u8; 16],
    pub(crate) owner_ordinal: u32,
    pub(crate) local_index: u32,
    pub(crate) payload: Vec<u8>,
}

impl SemanticRecord {
    pub(crate) fn canonical_key(&self) -> (u16, u16, [u8; 16], u32, u32, &[u8]) {
        (
            self.record_kind,
            self.entity_kind_code,
            self.stable_id,
            self.owner_ordinal,
            self.local_index,
            &self.payload,
        )
    }

    pub(crate) fn to_vector(&self) -> SemanticRecordVector {
        SemanticRecordVector {
            record_kind: self.record_kind,
            entity_kind_code: self.entity_kind_code,
            entity_kind: self.entity_kind.clone(),
            stable_id: encode_lower_hex(&self.stable_id),
            owner_ordinal: self.owner_ordinal,
            local_index: self.local_index,
            payload_hex: encode_lower_hex(&self.payload),
        }
    }
}

fn short_unique_profiled_key(kind: u16, unit_index: u32, local_index: u32) -> String {
    format!("{kind:02x}/{unit_index:08x}/{local_index:08x}")
}

fn validate_semantic_record_contract(
    envelope: &[String],
    rules: &RawSemanticRecordEnvelopeRules,
    record_kinds: &[RawRecordKind],
) -> Result<(), IdentityContractError> {
    let expected_envelope = [
        "recordKindU16Le",
        "entityKindU16Le",
        "stableIdBytes16",
        "ownerOrdinalU32Le",
        "localIndexU32Le",
        "payloadLengthU64Le",
        "payloadBytes",
    ];
    let actual_envelope = envelope.iter().map(String::as_str).collect::<Vec<_>>();
    expect_eq(
        "semanticRecordEnvelope",
        actual_envelope.as_slice(),
        expected_envelope.as_slice(),
    )?;
    expect_eq(
        "semanticRecordEnvelopeRules.entityKindAbsenceCodeU16",
        rules.entity_kind_absence_code_u16,
        0,
    )?;
    expect_eq(
        "semanticRecordEnvelopeRules.ownerOrLocalIndexAbsenceCodeU32",
        rules.owner_or_local_index_absence_code_u32,
        ABSENT_LOCAL_INDEX,
    )?;
    expect_eq(
        "semanticRecordEnvelopeRules.ownerOrdinalScope",
        rules.owner_ordinal_scope.as_str(),
        "within-owner-entity-kind",
    )?;
    expect_eq(
        "semanticRecordEnvelopeRules.ownerOrdinalDerivation",
        rules.owner_ordinal_derivation.as_str(),
        "sort all identity-declaration records with the same entityKindU16Le by stableIdBytes16 in unsigned lexicographic byte order; ownerOrdinalU32Le is the zero-based position of the record-owning entity",
    )?;

    expect_eq("recordKinds.length", record_kinds.len(), 13)?;
    let expected = [
        (
            1,
            "identity-declaration",
            serde_json::json!({
                "ownerEntitySource": "declared-entity",
                "entityKindSource": "declaredEntityKindCodeU16",
                "stableIdSource": "declaredEntityStableIdBytes16",
                "ownerOrdinalSource": "same-kind-canonical-owner-ordinal",
                "localIndexSource": {"kind": "absent-u32-max"}
            }),
            serde_json::json!([
                "requiredFieldCountU16Le",
                "repeated(requiredFieldCount):fieldTagU16Le",
                "repeated(requiredFieldCount):fieldByteLengthU32Le",
                "repeated(requiredFieldCount):exactFieldBytes"
            ]),
        ),
        (
            2,
            "owner-relation",
            serde_json::json!({
                "ownerEntitySource": "child-entity",
                "entityKindSource": "childEntityKindCodeU16",
                "stableIdSource": "childEntityStableIdBytes16",
                "ownerOrdinalSource": "same-kind-canonical-owner-ordinal",
                "localIndexSource": {"kind": "absent-u32-max"}
            }),
            serde_json::json!(["parentEntityKindU16Le", "parentStableIdBytes16"]),
        ),
    ];
    for (record_kind, (code, name, envelope_binding, payload_fields)) in
        record_kinds.iter().take(2).zip(expected)
    {
        expect_eq("recordKinds[].code", record_kind.code, code)?;
        expect_eq("recordKinds[].name", record_kind.name.as_str(), name)?;
        expect_eq(
            "recordKinds[].envelopeBinding",
            &record_kind.envelope_binding,
            &envelope_binding,
        )?;
        expect_eq(
            "recordKinds[].payloadFields",
            &record_kind.payload_fields,
            &payload_fields,
        )?;
    }
    Ok(())
}

fn validate_string_profiles(profiles: &[RawStringProfile]) -> Result<(), IdentityContractError> {
    let expected = [
        (
            "short-unique-v1",
            "kindCodeHex2 || '/' || unitIndexHex8 || '/' || localIndexHex8",
            20,
            None,
        ),
        (
            "shared-prefix-256-v1",
            "repeat('p',238) || kindCodeHex2 || unitIndexHex8 || localIndexHex8",
            256,
            Some(238),
        ),
        (
            "long-4096-v1",
            "repeat('q',4078) || kindCodeHex2 || unitIndexHex8 || localIndexHex8",
            4_096,
            Some(4_078),
        ),
    ];
    expect_eq("stringProfiles.length", profiles.len(), expected.len())?;
    for (profile, (id, formula, length, shared_prefix_length)) in profiles.iter().zip(expected) {
        expect_eq("stringProfiles[].id", profile.id.as_str(), id)?;
        expect_eq(
            "stringProfiles[].profiledKeyFormula",
            profile.profiled_key_formula.as_str(),
            formula,
        )?;
        expect_eq(
            "stringProfiles[].profiledKeyLengthBytes",
            profile.profiled_key_length_bytes,
            length,
        )?;
        expect_eq(
            "stringProfiles[].sharedPrefixLengthBytes",
            profile.shared_prefix_length_bytes,
            shared_prefix_length,
        )?;
        expect_eq(
            "stringProfiles[].encoding",
            profile.encoding.as_str(),
            "ascii",
        )?;
    }
    Ok(())
}

fn validate_identity_bindings(
    bindings: &[RawIdentityBinding],
) -> Result<Vec<IdentityBinding>, IdentityContractError> {
    expect_eq(
        "identityBindings.length",
        bindings.len(),
        EXPECTED_ENTITY_KINDS.len(),
    )?;
    let mut validated = Vec::with_capacity(bindings.len());
    for (index, binding) in bindings.iter().enumerate() {
        let expected_kind_code =
            u16::try_from(index + 1).expect("identity kind count must fit u16");
        expect_eq(
            "identityBindings[].entityKindCode",
            binding.entity_kind_code,
            expected_kind_code,
        )?;
        expect_eq(
            "identityBindings[].entityKind",
            binding.entity_kind.as_str(),
            EXPECTED_ENTITY_KINDS[index],
        )?;
        let expected_fields = EXPECTED_IDENTITY_FIELDS[index];
        expect_eq(
            "identityBindings[].fields.length",
            binding.fields.len(),
            expected_fields.len(),
        )?;
        let mut fields = Vec::with_capacity(binding.fields.len());
        for (raw, (expected_tag, expected_value)) in binding.fields.iter().zip(expected_fields) {
            expect_eq("identityBindings[].fields[].tag", raw.tag, *expected_tag)?;
            expect_eq(
                "identityBindings[].fields[].value",
                raw.value.as_str(),
                *expected_value,
            )?;
            fields.push(IdentityFieldBinding {
                tag: raw.tag,
                value: parse_identity_field_value(&raw.value)?,
            });
        }
        validated.push(IdentityBinding {
            entity_kind_code: binding.entity_kind_code,
            entity_kind: binding.entity_kind.clone(),
            fields,
        });
    }
    Ok(validated)
}

fn validate_identity_workload(
    workloads: &[RawIdentityWorkload],
) -> Result<Vec<OwnerRelation>, IdentityContractError> {
    let workload = workloads
        .iter()
        .find(|workload| workload.id == IDENTITY_WORKLOAD_ID)
        .ok_or_else(|| mismatch("workloads", IDENTITY_WORKLOAD_ID))?;
    let expected = EXPECTED_OWNER_RELATIONS
        .iter()
        .map(|(child, parent)| format!("{child}->{parent}"))
        .collect::<Vec<_>>();
    expect_eq(
        "workloads[LF-COMP-ID-v1].ownerRelations",
        &workload.owner_relations,
        &expected,
    )?;

    EXPECTED_OWNER_RELATIONS
        .iter()
        .map(|(child, parent)| {
            Ok(OwnerRelation {
                child_kind: entity_kind_code(child)?,
                parent_kind: entity_kind_code(parent)?,
            })
        })
        .collect()
}

fn entity_kind_code(name: &str) -> Result<u16, IdentityContractError> {
    EXPECTED_ENTITY_KINDS
        .iter()
        .position(|candidate| *candidate == name)
        .and_then(|index| u16::try_from(index + 1).ok())
        .ok_or_else(|| mismatch("entityKind", name))
}

fn parse_identity_field_value(value: &str) -> Result<IdentityFieldValue, IdentityContractError> {
    if value == "namespace" {
        return Ok(IdentityFieldValue::Namespace);
    }
    if let Some(arguments) = value
        .strip_prefix("profiled-key(kind=")
        .and_then(|value| value.strip_suffix(')'))
    {
        let (kind, local) = parse_kind_and_local(arguments, value)?;
        return Ok(IdentityFieldValue::ProfiledKey { kind, local });
    }
    if let Some(arguments) = value
        .strip_prefix("stable-id(kind=")
        .and_then(|value| value.strip_suffix(')'))
    {
        let (kind, local) = parse_kind_and_local(arguments, value)?;
        return Ok(IdentityFieldValue::StableId { kind, local });
    }
    Err(mismatch("identityBindings[].fields[].value", value))
}

fn parse_kind_and_local(
    arguments_after_kind: &str,
    original: &str,
) -> Result<(u16, u32), IdentityContractError> {
    let (kind, local) = arguments_after_kind
        .split_once(",local=")
        .ok_or_else(|| mismatch("identityBindings[].fields[].value", original))?;
    let kind = kind
        .parse::<u16>()
        .map_err(|_| mismatch("identityBindings[].fields[].value", original))?;
    let local = local
        .parse::<u32>()
        .map_err(|_| mismatch("identityBindings[].fields[].value", original))?;
    Ok((kind, local))
}

fn append_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn append_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn append_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn encode_lower_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn expect_eq<T>(field: &'static str, actual: T, expected: T) -> Result<(), IdentityContractError>
where
    T: PartialEq + std::fmt::Debug,
{
    if actual != expected {
        return Err(IdentityContractError::Mismatch {
            field,
            expected: format!("{expected:?}"),
            actual: format!("{actual:?}"),
        });
    }
    Ok(())
}

fn mismatch(field: &'static str, expected: impl Into<String>) -> IdentityContractError {
    IdentityContractError::Mismatch {
        field,
        expected: expected.into(),
        actual: "missing or different".to_owned(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IdentityContractError {
    #[error("工作负载清单身份契约不是预期形状：{0}")]
    InvalidShape(serde_json::Error),
    #[error("工作负载清单身份字段 {field} 不匹配：期望 {expected}，实际 {actual}")]
    Mismatch {
        field: &'static str,
        expected: String,
        actual: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum IdentityGenerationError {
    #[error(transparent)]
    ModuleGraph(#[from] crate::GeneratorError),
    #[error("模块图缺少工作单元模块 unit/{unit_index:08x}")]
    MissingUnitModule { unit_index: u32 },
    #[error("实体种类 {source_kind} 的 StableId 依赖尚未解析的实体种类 {target_kind}")]
    UnresolvedStableIdDependency { source_kind: u16, target_kind: u16 },
    #[error("工作单元 {unit_index} 缺少实体种类 {entity_kind} 的身份声明")]
    MissingDeclaration { unit_index: u32, entity_kind: u16 },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdentityManifestProjection {
    semantic_record_stream_version: u32,
    semantic_record_domain_utf8_nul_terminated: String,
    identity_encoding_version: u16,
    identity_registry_revision: u32,
    semantic_record_envelope: Vec<String>,
    semantic_record_envelope_rules: RawSemanticRecordEnvelopeRules,
    record_kinds: Vec<RawRecordKind>,
    string_profiles: Vec<RawStringProfile>,
    identity_bindings: Vec<RawIdentityBinding>,
    workloads: Vec<RawIdentityWorkload>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSemanticRecordEnvelopeRules {
    entity_kind_absence_code_u16: u16,
    owner_or_local_index_absence_code_u32: u32,
    owner_ordinal_scope: String,
    owner_ordinal_derivation: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRecordKind {
    code: u16,
    name: String,
    envelope_binding: serde_json::Value,
    payload_fields: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawStringProfile {
    id: String,
    profiled_key_formula: String,
    profiled_key_length_bytes: usize,
    shared_prefix_length_bytes: Option<usize>,
    encoding: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawIdentityBinding {
    entity_kind_code: u16,
    entity_kind: String,
    fields: Vec<RawIdentityField>,
}

#[derive(Clone, Debug, Deserialize)]
struct RawIdentityField {
    tag: u16,
    value: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawIdentityWorkload {
    id: String,
    #[serde(default)]
    owner_relations: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_repository_contract;

    fn contracts() -> (GeneratorContract, IdentityContract, String) {
        let trusted = load_repository_contract().expect("frozen contract");
        let generator = trusted.generator_contract().expect("generator contract");
        let identity =
            IdentityContract::from_manifest(&trusted.workload_manifest).expect("identity contract");
        (
            generator,
            identity,
            trusted.descriptor.workload_manifest.sha256,
        )
    }

    #[test]
    fn frozen_identity_contract_is_accepted() {
        let (_, identity, _) = contracts();

        assert_eq!(identity.bindings.len(), 22);
        assert_eq!(identity.owner_relations.len(), 10);
        assert_eq!(identity.semantic_record_stream_version, 1);
    }

    #[test]
    fn n1_vectors_have_exact_identity_and_record_counts() {
        let (generator, identity, manifest_sha256) = contracts();
        let document =
            build_identity_known_vectors(&generator, &identity, &manifest_sha256).expect("vectors");

        assert_eq!(document.vectors.len(), 3);
        for vector in document.vectors {
            assert_eq!(vector.declarations.len(), 22);
            assert_eq!(vector.records.len(), 32);
            assert_eq!(
                vector
                    .records
                    .iter()
                    .filter(|record| record.record_kind == 1)
                    .count(),
                22
            );
            assert_eq!(
                vector
                    .records
                    .iter()
                    .filter(|record| record.record_kind == 2)
                    .count(),
                10
            );
        }
    }

    #[test]
    fn graph_profiles_produce_distinct_namespaces_and_stable_ids() {
        let (generator, identity, manifest_sha256) = contracts();
        let document =
            build_identity_known_vectors(&generator, &identity, &manifest_sha256).expect("vectors");

        let namespaces = document
            .vectors
            .iter()
            .map(|vector| vector.namespace_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let corridor_ids = document
            .vectors
            .iter()
            .map(|vector| vector.declarations[0].stable_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(namespaces.len(), 3);
        assert_eq!(corridor_ids.len(), 3);
    }

    #[test]
    fn published_identity_and_semantic_digest_summary_is_stable() {
        let (generator, identity, manifest_sha256) = contracts();
        let document =
            build_identity_known_vectors(&generator, &identity, &manifest_sha256).expect("vectors");
        let actual = document
            .vectors
            .iter()
            .map(|vector| {
                (
                    vector.graph_profile,
                    vector.namespace_id.as_str(),
                    vector.declarations[0].stable_id.as_str(),
                    vector.semantic_record_stream_byte_length,
                    vector.semantic_digest_sha256.as_str(),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            actual,
            [
                (
                    GraphProfileId::WideStar,
                    "77ee06c1fe5111a1e002ff604266896e",
                    "1f0b404546606bbdb629ebd69c1a4bf6",
                    3_132,
                    "897447f2192acf03a42bfb46cb75a6e0f9de5409adbb752136902fcc5560e31e",
                ),
                (
                    GraphProfileId::DeepChain,
                    "7bf928ee48d1e32409db6608f97e38fd",
                    "0a625360baf39fa79252816b734287ba",
                    3_132,
                    "012499aefcc4598a0b42df79aeabf2b114ce2622bb05b8d85090e6fbeae3e75b",
                ),
                (
                    GraphProfileId::SharedFaninDag,
                    "fc33897a7819b3e53f1c47ef5c199f4d",
                    "9c5b27fcc7b6d2bb367d0eb223abcc2d",
                    3_132,
                    "4394a7aa0d2a72e71f8c39354df7165a2e230f6402a604c40f84b698065e79f6",
                ),
            ]
        );
    }

    #[test]
    fn owner_relation_payload_uses_the_resolved_parent_stable_id() {
        let (generator, identity, manifest_sha256) = contracts();
        let document =
            build_identity_known_vectors(&generator, &identity, &manifest_sha256).expect("vectors");
        let vector = &document.vectors[0];
        let corridor = &vector.declarations[0];
        let section_relation = vector
            .records
            .iter()
            .find(|record| record.record_kind == 2 && record.entity_kind_code == 2)
            .expect("RoadSection owner relation");

        assert_eq!(
            section_relation.payload_hex,
            format!("0100{}", corridor.stable_id)
        );
    }

    #[test]
    fn published_identity_vectors_equal_fresh_generation_and_bound_bytes() {
        let (generator, identity, manifest_sha256) = contracts();
        let document =
            build_identity_known_vectors(&generator, &identity, &manifest_sha256).expect("vectors");
        let rendered = format!(
            "{}\n",
            serde_json::to_string_pretty(&document).expect("serialize identity vectors")
        );
        let published = include_bytes!("../known-vectors/identity-records-v1.json");

        assert_eq!(published, rendered.as_bytes());
        assert_eq!(published.len(), IDENTITY_KNOWN_VECTOR_BYTE_LENGTH);
        assert_eq!(
            encode_lower_hex(&Sha256::digest(published)),
            IDENTITY_KNOWN_VECTOR_SHA256
        );
    }
}
