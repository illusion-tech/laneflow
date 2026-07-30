use serde::Deserialize;
use std::collections::BTreeMap;

const EXPECTED_NAMESPACE_INPUT_ORDER: [&str; 9] = [
    "domainUtf8NulTerminated",
    "generatorVersionU32Le",
    "baseSeedU64Le",
    "workloadIdByteLengthU32Le",
    "workloadIdUtf8",
    "graphProfileIdByteLengthU32Le",
    "graphProfileIdUtf8",
    "canonicalModuleNameByteLengthU32Le",
    "canonicalModuleNameUtf8",
];

const EXPECTED_MODULE_NAME_PATTERNS: [&str; 4] =
    ["root", "shared/common", "group/{g:08x}", "unit/{i:08x}"];

const EXPECTED_ID_ENTITY_KINDS: [&str; 22] = [
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

const EXPECTED_OWNER_RELATIONS: [&str; 10] = [
    "RoadSection->RoadCorridor",
    "AuthoringLane->RoadSection",
    "Movement->Junction",
    "ManeuverPath->Movement",
    "ManeuverGate->ManeuverPath",
    "WaitingZone->ManeuverPath",
    "SignalPhase->SignalController",
    "ParkingSpace->ParkingArea",
    "LaneGroup->RoadSection",
    "FacilityBand->RoadCorridor",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratorContract {
    pub(crate) generator_version: u32,
    pub(crate) base_seed: u64,
    pub(crate) namespace_domain: String,
    pub(crate) namespace_digest_offset: usize,
    pub(crate) namespace_digest_length: usize,
    pub(crate) splitmix64_increment: u64,
    pub(crate) splitmix64_multiplier_1: u64,
    pub(crate) splitmix64_multiplier_2: u64,
    pub(crate) imports_sequence_kind: u8,
    pub(crate) declarations_sequence_kind: u8,
    pub(crate) references_sequence_kind: u8,
    pub(crate) relations_sequence_kind: u8,
    pub(crate) geometry_sequence_kind: u8,
    pub(crate) shared_fanin_group_width: u32,
}

impl GeneratorContract {
    pub const fn generator_version(&self) -> u32 {
        self.generator_version
    }

    pub const fn base_seed(&self) -> u64 {
        self.base_seed
    }

    pub const fn shared_fanin_group_width(&self) -> u32 {
        self.shared_fanin_group_width
    }

    pub fn from_manifest(value: &serde_json::Value) -> Result<Self, ManifestContractError> {
        let projection: ManifestProjection =
            serde_json::from_value(value.clone()).map_err(ManifestContractError::InvalidShape)?;

        expect_eq("generatorVersion", projection.generator_version, 1)?;
        expect_eq(
            "baseSeedHexU64",
            projection.base_seed_hex_u64.as_str(),
            "4c46434f4d500001",
        )?;
        let base_seed = parse_hex_u64("baseSeedHexU64", &projection.base_seed_hex_u64)?;
        expect_eq("baseSeedHexU64", base_seed, 0x4c46_434f_4d50_0001)?;

        validate_namespace_contract(&projection.namespace_derivation)?;
        validate_permutation_contract(&projection.permutation)?;
        let shared_fanin_group_width =
            validate_module_graph_profiles(&projection.module_graph_profiles)?;
        validate_id_workload(&projection.workloads)?;

        let digest_offset =
            usize::try_from(projection.namespace_derivation.selected_digest_bytes.offset)
                .map_err(|_| mismatch("namespaceDerivation.selectedDigestBytes.offset", "0"))?;
        let digest_length =
            usize::try_from(projection.namespace_derivation.selected_digest_bytes.length)
                .map_err(|_| mismatch("namespaceDerivation.selectedDigestBytes.length", "16"))?;

        Ok(Self {
            generator_version: projection.generator_version,
            base_seed,
            namespace_domain: projection.namespace_derivation.domain_utf8_nul_terminated,
            namespace_digest_offset: digest_offset,
            namespace_digest_length: digest_length,
            splitmix64_increment: parse_hex_u64(
                "permutation.splitmix64ConstantsHexU64[0]",
                &projection.permutation.splitmix64_constants_hex_u64[0],
            )?,
            splitmix64_multiplier_1: parse_hex_u64(
                "permutation.splitmix64ConstantsHexU64[1]",
                &projection.permutation.splitmix64_constants_hex_u64[1],
            )?,
            splitmix64_multiplier_2: parse_hex_u64(
                "permutation.splitmix64ConstantsHexU64[2]",
                &projection.permutation.splitmix64_constants_hex_u64[2],
            )?,
            imports_sequence_kind: projection.permutation.sequence_kinds.imports,
            declarations_sequence_kind: projection.permutation.sequence_kinds.declarations,
            references_sequence_kind: projection.permutation.sequence_kinds.references,
            relations_sequence_kind: projection.permutation.sequence_kinds.relations,
            geometry_sequence_kind: projection.permutation.sequence_kinds.geometry,
            shared_fanin_group_width,
        })
    }
}

fn validate_namespace_contract(
    contract: &NamespaceDerivation,
) -> Result<(), ManifestContractError> {
    expect_eq(
        "namespaceDerivation.domainUtf8NulTerminated",
        contract.domain_utf8_nul_terminated.as_str(),
        "LF-COMP-NAMESPACE-v1",
    )?;
    expect_string_slice(
        "namespaceDerivation.inputOrder",
        &contract.input_order,
        &EXPECTED_NAMESPACE_INPUT_ORDER,
    )?;
    expect_eq(
        "namespaceDerivation.moduleNameCoverage",
        contract.module_name_coverage.as_str(),
        "every-expanded-module-graph-node",
    )?;
    expect_string_slice(
        "namespaceDerivation.canonicalModuleNamePatterns",
        &contract.canonical_module_name_patterns,
        &EXPECTED_MODULE_NAME_PATTERNS,
    )?;
    expect_eq("namespaceDerivation.hash", contract.hash.as_str(), "BLAKE3")?;
    expect_eq(
        "namespaceDerivation.selectedDigestBytes.offset",
        contract.selected_digest_bytes.offset,
        0,
    )?;
    expect_eq(
        "namespaceDerivation.selectedDigestBytes.length",
        contract.selected_digest_bytes.length,
        16,
    )?;
    expect_eq(
        "namespaceDerivation.fieldEncoding",
        contract.field_encoding.as_str(),
        "lowercase-ascii-hex",
    )?;
    expect_eq(
        "namespaceDerivation.fieldLengthBytes",
        contract.field_length_bytes,
        32,
    )
}

fn validate_permutation_contract(contract: &Permutation) -> Result<(), ManifestContractError> {
    expect_eq(
        "permutation.algorithm",
        contract.algorithm.as_str(),
        "fisher-yates-descending",
    )?;
    expect_eq("permutation.prng", contract.prng.as_str(), "splitmix64")?;
    expect_string_slice(
        "permutation.splitmix64ConstantsHexU64",
        &contract.splitmix64_constants_hex_u64,
        &["9e3779b97f4a7c15", "bf58476d1ce4e5b9", "94d049bb133111eb"],
    )?;
    expect_eq(
        "permutation.seedFormula",
        contract.seed_formula.as_str(),
        "baseSeed XOR (sequenceKindU64 << 56) XOR moduleSeedOrdinalU64",
    )?;
    expect_eq(
        "permutation.moduleSeedOrdinalFormula.root",
        contract.module_seed_ordinal_formula.root.as_str(),
        "0",
    )?;
    expect_eq(
        "permutation.moduleSeedOrdinalFormula.shared/common",
        contract.module_seed_ordinal_formula.shared_common.as_str(),
        "1",
    )?;
    expect_eq(
        "permutation.moduleSeedOrdinalFormula.group/{g:08x}",
        contract.module_seed_ordinal_formula.group.as_str(),
        "(1_u64 << 40) | g_u64",
    )?;
    expect_eq(
        "permutation.moduleSeedOrdinalFormula.unit/{i:08x}",
        contract.module_seed_ordinal_formula.unit.as_str(),
        "(2_u64 << 40) | i_u64",
    )?;
    expect_eq(
        "permutation.moduleSeedOrdinalFormula.indexRange",
        contract.module_seed_ordinal_formula.index_range.as_str(),
        "g and i are exact u32 values; bits 40..47 identify the module category and bits 56..63 remain reserved for sequenceKindU64",
    )?;
    expect_eq(
        "permutation.sequenceKinds.imports",
        contract.sequence_kinds.imports,
        1,
    )?;
    expect_eq(
        "permutation.sequenceKinds.declarations",
        contract.sequence_kinds.declarations,
        2,
    )?;
    expect_eq(
        "permutation.sequenceKinds.references",
        contract.sequence_kinds.references,
        3,
    )?;
    expect_eq(
        "permutation.sequenceKinds.relations",
        contract.sequence_kinds.relations,
        4,
    )?;
    expect_eq(
        "permutation.sequenceKinds.geometry",
        contract.sequence_kinds.geometry,
        5,
    )?;
    expect_eq(
        "permutation.swapFormula",
        contract.swap_formula.as_str(),
        "swap(i, nextU64 mod (i + 1))",
    )?;
    expect_eq(
        "permutation.sequenceScope",
        contract.sequence_scope.as_str(),
        "independently permute each expanded module's imports, declarations, references, relations, and geometry list with that module's moduleSeedOrdinalU64; empty and singleton lists still use the same defined seed but perform no swap",
    )
}

fn validate_module_graph_profiles(
    profiles: &[ModuleGraphProfile],
) -> Result<u32, ManifestContractError> {
    let identifiers = profiles
        .iter()
        .map(|profile| profile.id.as_str())
        .collect::<Vec<_>>();
    expect_string_slice(
        "moduleGraphProfiles[].id",
        &identifiers,
        &["wide-star-v1", "deep-chain-v1", "shared-fanin-dag-v1"],
    )?;

    let shared_fanin = profiles
        .iter()
        .find(|profile| profile.id == "shared-fanin-dag-v1")
        .ok_or_else(|| mismatch("moduleGraphProfiles", "shared-fanin-dag-v1"))?;
    expect_eq(
        "moduleGraphProfiles[shared-fanin-dag-v1].groupWidth",
        shared_fanin.group_width,
        Some(64),
    )?;
    expect_eq(
        "moduleGraphProfiles[wide-star-v1].groupWidth",
        profiles[0].group_width,
        None,
    )?;
    expect_eq(
        "moduleGraphProfiles[deep-chain-v1].groupWidth",
        profiles[1].group_width,
        None,
    )?;

    let expected_modules = [
        serde_json::json!({"root": 1, "unit": "N"}),
        serde_json::json!({"root": 1, "unit": "N"}),
        serde_json::json!({
            "root": 1,
            "shared/common": 1,
            "group": "ceil(N / 64)",
            "unit": "N"
        }),
    ];
    let expected_edges: [Vec<(&str, &str, &str)>; 3] = [
        vec![("root", "unit/{i:08x}", "i in [0,N)")],
        vec![
            ("root", "unit/00000000", "N >= 1"),
            ("unit/{i:08x}", "unit/{i+1:08x}", "i in [0,N-1)"),
        ],
        vec![
            ("root", "group/{g:08x}", "g in [0,ceil(N/64))"),
            ("group/{floor(i/64):08x}", "unit/{i:08x}", "i in [0,N)"),
            ("unit/{i:08x}", "shared/common", "i in [0,N)"),
        ],
    ];
    let expected_references: [Vec<(&str, &str, &str)>; 3] = [
        vec![(
            "root",
            "canonical-first-declaration(unit/{i:08x})",
            "i in [0,N)",
        )],
        vec![(
            "unit/{i:08x}",
            "canonical-first-declaration(unit/{i+1:08x})",
            "i in [0,N-1)",
        )],
        vec![(
            "unit/{i:08x}",
            "shared/common::shared-calibration-anchor",
            "i in [0,N)",
        )],
    ];
    let expected_counts = [
        serde_json::json!({
            "moduleCount": "N + 1",
            "importEdgeCount": "N",
            "crossModuleReferenceCount": "N",
            "maximumImportDepth": "1"
        }),
        serde_json::json!({
            "moduleCount": "N + 1",
            "importEdgeCount": "N",
            "crossModuleReferenceCount": "max(N - 1, 0)",
            "maximumImportDepth": "N"
        }),
        serde_json::json!({
            "moduleCount": "N + ceil(N / 64) + 2",
            "importEdgeCount": "2 * N + ceil(N / 64)",
            "crossModuleReferenceCount": "N",
            "maximumImportDepth": "3"
        }),
    ];

    for (profile_index, profile) in profiles.iter().enumerate() {
        expect_eq(
            "moduleGraphProfiles[].sharedSourceConstantRecordCount",
            profile.shared_source_constant_record_count,
            if profile_index == 2 { 1 } else { 0 },
        )?;
        expect_eq(
            "moduleGraphProfiles[].modules",
            &profile.modules,
            &expected_modules[profile_index],
        )?;
        expect_graph_edges(
            "moduleGraphProfiles[].edges",
            &profile.edges,
            &expected_edges[profile_index],
        )?;
        expect_graph_edges(
            "moduleGraphProfiles[].crossModuleReferences",
            &profile.cross_module_references,
            &expected_references[profile_index],
        )?;
        expect_eq(
            "moduleGraphProfiles[].expectedCounts",
            &profile.expected_counts,
            &expected_counts[profile_index],
        )?;
    }

    if profiles[0].shared_source_constant.is_some() || profiles[1].shared_source_constant.is_some()
    {
        return Err(mismatch(
            "moduleGraphProfiles[].sharedSourceConstant",
            "only shared-fanin-dag-v1 may define it",
        ));
    }
    expect_eq(
        "moduleGraphProfiles[shared-fanin-dag-v1].sharedSourceConstant",
        profiles[2].shared_source_constant.as_ref().ok_or_else(|| {
            mismatch(
                "moduleGraphProfiles[shared-fanin-dag-v1].sharedSourceConstant",
                "object",
            )
        })?,
        &serde_json::json!({
            "module": "shared/common",
            "nameUtf8": "shared-calibration-anchor",
            "valueUtf8": "laneflow-compiler-calibration-v1",
            "typedAstRecordCount": 1,
            "hirRecordCount": 1,
            "mirRecordCount": 0,
            "lirRecordCount": 0
        }),
    )?;

    Ok(shared_fanin.group_width.expect("validated group width"))
}

fn expect_graph_edges(
    field: &'static str,
    actual: &[GraphEdge],
    expected: &[(&str, &str, &str)],
) -> Result<(), ManifestContractError> {
    expect_eq(field, actual.len(), expected.len())?;
    for (edge, (expected_from, expected_to, expected_condition)) in actual.iter().zip(expected) {
        expect_eq(field, edge.from.as_str(), *expected_from)?;
        expect_eq(field, edge.to.as_str(), *expected_to)?;
        expect_eq(field, edge.condition.as_str(), *expected_condition)?;
    }
    Ok(())
}

fn validate_id_workload(workloads: &[Workload]) -> Result<(), ManifestContractError> {
    let workload = workloads
        .iter()
        .find(|workload| workload.id == "LF-COMP-ID-v1")
        .ok_or_else(|| mismatch("workloads", "LF-COMP-ID-v1"))?;
    expect_eq("workloads[LF-COMP-ID-v1].scalable", workload.scalable, true)?;
    expect_string_slice(
        "workloads[LF-COMP-ID-v1].graphProfiles",
        &workload.graph_profiles,
        &["wide-star-v1", "deep-chain-v1", "shared-fanin-dag-v1"],
    )?;
    expect_string_slice(
        "workloads[LF-COMP-ID-v1].stringProfiles",
        &workload.string_profiles,
        &["short-unique-v1", "shared-prefix-256-v1", "long-4096-v1"],
    )?;

    let expected_stage_inputs = [
        ("sourceDeclarationCount", 22),
        ("identityFieldOccurrenceCount", 58),
        ("profiledKeyOccurrenceCount", 24),
        ("sourceReferenceCount", 22),
        ("sourceRelationCount", 10),
        ("sourceGeometryCount", 0),
    ];
    expect_u64_map(
        "workloads[LF-COMP-ID-v1].perUnitStageInputs",
        &workload.per_unit_stage_inputs,
        &expected_stage_inputs,
    )?;

    let mut expected_counts = EXPECTED_ID_ENTITY_KINDS
        .iter()
        .map(|kind| (*kind, 1_u64))
        .collect::<Vec<_>>();
    expected_counts.extend([
        ("identityDeclaration", 22),
        ("requiredIdentityFieldOccurrence", 58),
        ("ownerRelation", 10),
        ("semanticOutputRecord", 32),
    ]);
    expect_u64_map(
        "workloads[LF-COMP-ID-v1].perUnitCounts",
        &workload.per_unit_counts,
        &expected_counts,
    )?;
    expect_string_slice(
        "workloads[LF-COMP-ID-v1].ownerRelations",
        &workload.owner_relations,
        &EXPECTED_OWNER_RELATIONS,
    )
}

fn expect_u64_map(
    field: &'static str,
    actual: &BTreeMap<String, u64>,
    expected_entries: &[(&str, u64)],
) -> Result<(), ManifestContractError> {
    let expected = expected_entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), *value))
        .collect::<BTreeMap<_, _>>();
    expect_eq(field, actual, &expected)
}

fn expect_string_slice<T>(
    field: &'static str,
    actual: &[T],
    expected: &[&str],
) -> Result<(), ManifestContractError>
where
    T: AsRef<str>,
{
    let actual = actual.iter().map(AsRef::as_ref).collect::<Vec<_>>();
    if actual != expected {
        return Err(ManifestContractError::Mismatch {
            field,
            expected: format!("{expected:?}"),
            actual: format!("{actual:?}"),
        });
    }
    Ok(())
}

fn expect_eq<T>(field: &'static str, actual: T, expected: T) -> Result<(), ManifestContractError>
where
    T: PartialEq + std::fmt::Debug,
{
    if actual != expected {
        return Err(ManifestContractError::Mismatch {
            field,
            expected: format!("{expected:?}"),
            actual: format!("{actual:?}"),
        });
    }
    Ok(())
}

fn parse_hex_u64(field: &'static str, value: &str) -> Result<u64, ManifestContractError> {
    u64::from_str_radix(value, 16).map_err(|_| ManifestContractError::Mismatch {
        field,
        expected: "16 lowercase hexadecimal digits".to_owned(),
        actual: value.to_owned(),
    })
}

fn mismatch(field: &'static str, expected: impl Into<String>) -> ManifestContractError {
    ManifestContractError::Mismatch {
        field,
        expected: expected.into(),
        actual: "missing or different".to_owned(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestContractError {
    #[error("工作负载清单生成契约不是预期形状：{0}")]
    InvalidShape(serde_json::Error),
    #[error("工作负载清单字段 {field} 不匹配：期望 {expected}，实际 {actual}")]
    Mismatch {
        field: &'static str,
        expected: String,
        actual: String,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestProjection {
    generator_version: u32,
    base_seed_hex_u64: String,
    namespace_derivation: NamespaceDerivation,
    permutation: Permutation,
    module_graph_profiles: Vec<ModuleGraphProfile>,
    workloads: Vec<Workload>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NamespaceDerivation {
    domain_utf8_nul_terminated: String,
    input_order: Vec<String>,
    module_name_coverage: String,
    canonical_module_name_patterns: Vec<String>,
    hash: String,
    selected_digest_bytes: SelectedDigestBytes,
    field_encoding: String,
    field_length_bytes: u32,
}

#[derive(Clone, Copy, Debug, Deserialize)]
struct SelectedDigestBytes {
    offset: u32,
    length: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Permutation {
    algorithm: String,
    prng: String,
    splitmix64_constants_hex_u64: Vec<String>,
    seed_formula: String,
    module_seed_ordinal_formula: ModuleSeedOrdinalFormula,
    sequence_kinds: SequenceKinds,
    sequence_scope: String,
    swap_formula: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ModuleSeedOrdinalFormula {
    root: String,
    #[serde(rename = "shared/common")]
    shared_common: String,
    #[serde(rename = "group/{g:08x}")]
    group: String,
    #[serde(rename = "unit/{i:08x}")]
    unit: String,
    #[serde(rename = "indexRange")]
    index_range: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
struct SequenceKinds {
    imports: u8,
    declarations: u8,
    references: u8,
    relations: u8,
    geometry: u8,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModuleGraphProfile {
    id: String,
    group_width: Option<u32>,
    shared_source_constant_record_count: u32,
    modules: serde_json::Value,
    edges: Vec<GraphEdge>,
    cross_module_references: Vec<GraphEdge>,
    shared_source_constant: Option<serde_json::Value>,
    expected_counts: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize)]
struct GraphEdge {
    from: String,
    to: String,
    #[serde(rename = "for")]
    condition: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Workload {
    id: String,
    #[serde(default)]
    scalable: bool,
    #[serde(default)]
    graph_profiles: Vec<String>,
    #[serde(default)]
    string_profiles: Vec<String>,
    #[serde(default)]
    per_unit_stage_inputs: BTreeMap<String, u64>,
    #[serde(default)]
    per_unit_counts: BTreeMap<String, u64>,
    #[serde(default)]
    owner_relations: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_repository_contract;

    #[test]
    fn frozen_manifest_projection_is_accepted() {
        let trusted = load_repository_contract().expect("frozen contract");
        let contract = trusted.generator_contract().expect("generator contract");

        assert_eq!(contract.generator_version, 1);
        assert_eq!(contract.base_seed, 0x4c46_434f_4d50_0001);
        assert_eq!(contract.shared_fanin_group_width, 64);
    }

    #[test]
    fn changed_generator_seed_is_rejected_after_trust_bootstrap() {
        let trusted = load_repository_contract().expect("frozen contract");
        let mut manifest = trusted.workload_manifest;
        manifest["baseSeedHexU64"] = serde_json::json!("0000000000000000");

        assert!(matches!(
            GeneratorContract::from_manifest(&manifest),
            Err(ManifestContractError::Mismatch {
                field: "baseSeedHexU64",
                ..
            })
        ));
    }

    #[test]
    fn noncanonical_generator_seed_spelling_is_rejected() {
        let trusted = load_repository_contract().expect("frozen contract");
        let mut manifest = trusted.workload_manifest;
        manifest["baseSeedHexU64"] = serde_json::json!("4C46434F4D500001");

        assert!(matches!(
            GeneratorContract::from_manifest(&manifest),
            Err(ManifestContractError::Mismatch {
                field: "baseSeedHexU64",
                ..
            })
        ));
    }

    #[test]
    fn changed_module_graph_reference_formula_is_rejected() {
        let trusted = load_repository_contract().expect("frozen contract");
        let mut manifest = trusted.workload_manifest;
        manifest["moduleGraphProfiles"][0]["crossModuleReferences"][0]["to"] =
            serde_json::json!("untrusted-alternative");

        assert!(matches!(
            GeneratorContract::from_manifest(&manifest),
            Err(ManifestContractError::Mismatch {
                field: "moduleGraphProfiles[].crossModuleReferences",
                ..
            })
        ));
    }
}
