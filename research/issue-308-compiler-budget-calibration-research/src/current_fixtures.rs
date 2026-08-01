//! 当前固定样例的非生产研究投影。
//!
//! 原始 ID 只在计时区外解析。ScenarioManifest 只绑定三件套来源，不生成模块、
//! 声明、关系、几何或语义记录。

use crate::corridor::{
    CompiledDeclaration, CorridorError, CorridorTemplate, EntityRef, UnitEntityRef,
    build_current_fixture_raw_template, compile_semantic_records, profiled_key_field_count,
};
use crate::identity::{
    IdentityContract, IdentityField, IdentityFieldValue, STABLE_ID_DOMAIN, SemanticRecord,
    encode_canonical_identity, encode_semantic_record_stream,
};
use crate::stage::{
    HirStageRecord, IdentityAggregateCounts, MirLirStageRecord, SourceSpanRecord, StageBreakdown,
    StageContract, StageShape, TypedAstStageRecord,
};
use crate::{GeneratorContract, TrustedContract};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path};

pub const CURRENT_FIXTURES_WORKLOAD_ID: &str = "LF-COMP-RESEARCH-CURRENT-FIXTURES-v1";
pub const CURRENT_FIXTURES_KNOWN_VECTOR_SCHEMA: &str =
    "laneflow.compiler-calibration-current-fixtures-summary-known-vectors";
pub const CURRENT_FIXTURES_CHILD_SCHEMA: &str =
    "laneflow.compiler-calibration-current-fixtures-child";
pub const CURRENT_FIXTURES_CHILD_SCHEMA_VERSION: u32 = 1;
#[cfg(test)]
const CURRENT_FIXTURES_KNOWN_VECTOR_BYTE_LENGTH: usize = 11_755;
#[cfg(test)]
const CURRENT_FIXTURES_KNOWN_VECTOR_SHA256: &str =
    "9abc9b91027fc29c2ca292e577a29ad3501557c68bc0ad683820985556155e6f";

const NOT_APPLICABLE_GRAPH_PROFILE: &str = "not-applicable";
const FIXED_UNIT_INDEX: u32 = 0;
const TRAFFIC_MODULE: &str = "traffic";
const SPATIAL_MODULE: &str = "spatial";

#[derive(Clone, Debug)]
pub struct CurrentFixturesContract {
    cases: Vec<CurrentFixtureCaseContract>,
}

impl CurrentFixturesContract {
    pub fn from_manifest(manifest: &Value) -> Result<Self, CurrentFixturesError> {
        let workload = required_array(manifest, "workloads")?
            .iter()
            .find(|candidate| {
                candidate.get("id").and_then(Value::as_str) == Some(CURRENT_FIXTURES_WORKLOAD_ID)
            })
            .ok_or_else(|| {
                CurrentFixturesError::Missing(CURRENT_FIXTURES_WORKLOAD_ID.to_owned())
            })?;
        require_exact_keys(
            workload,
            &[
                "id",
                "scalable",
                "projectionContract",
                "cases",
                "stageInputRule",
                "reservedProductionWorkloadId",
                "reservedProductionOwnerIssue",
            ],
        )?;
        require_bool(workload, "scalable", false)?;
        validate_projection_contract(required_object(workload, "projectionContract")?)?;
        require_string(
            workload,
            "stageInputRule",
            "apply projectionContract and the selected case projection exactly; compare every aggregate input, domain count, semantic payload byte count, and exactStageExpectation; current fixtures never enter budget or candidate ranking",
        )?;
        require_string(
            workload,
            "reservedProductionWorkloadId",
            "LF-COMP-CURRENT-EQUIV-v1",
        )?;
        require_u64(workload, "reservedProductionOwnerIssue", 292)?;
        let cases = required_array(workload, "cases")?
            .iter()
            .map(CurrentFixtureCaseContract::parse)
            .collect::<Result<Vec<_>, _>>()?;
        let ids = cases
            .iter()
            .map(|case| case.id.as_str())
            .collect::<Vec<_>>();
        if ids
            != [
                "signalized-corridor",
                "parking-signals-baseline",
                "multi-gate-waiting-zone",
            ]
        {
            return Err(CurrentFixturesError::Mismatch {
                path: "cases[].id".to_owned(),
                expected: "signalized-corridor, parking-signals-baseline, multi-gate-waiting-zone"
                    .to_owned(),
                actual: ids.join(", "),
            });
        }
        Ok(Self { cases })
    }

    pub(crate) fn cases(&self) -> &[CurrentFixtureCaseContract] {
        &self.cases
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CurrentFixtureCaseContract {
    id: String,
    files: Vec<BoundFixtureFile>,
    canonical_modules: Vec<String>,
    import_edges: Vec<(String, String)>,
    entity_counts: BTreeMap<String, u64>,
    relation_counts: BTreeMap<String, u64>,
    counts: IdentityAggregateCounts,
    stages: StageBreakdown,
}

impl CurrentFixtureCaseContract {
    fn parse(value: &Value) -> Result<Self, CurrentFixturesError> {
        require_exact_keys(value, &["id", "files", "projection"])?;
        let id = required_string(value, "id")?.to_owned();
        let files = required_array(value, "files")?
            .iter()
            .map(BoundFixtureFile::parse)
            .collect::<Result<Vec<_>, _>>()?;
        let projection = required_object(value, "projection")?;
        require_exact_keys(
            projection,
            &[
                "canonicalModules",
                "importEdges",
                "entityCounts",
                "relationRecordCounts",
                "exactAggregateInputs",
                "exactStageExpectations",
            ],
        )?;
        let canonical_modules = required_array(projection, "canonicalModules")?
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| CurrentFixturesError::Missing("canonicalModules[]".to_owned()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let import_edges = required_array(projection, "importEdges")?
            .iter()
            .map(|edge| {
                require_exact_keys(edge, &["from", "to"])?;
                Ok((
                    required_string(edge, "from")?.to_owned(),
                    required_string(edge, "to")?.to_owned(),
                ))
            })
            .collect::<Result<Vec<_>, CurrentFixturesError>>()?;
        let entity_counts = parse_u64_object(required_object(projection, "entityCounts")?)?;
        let relation_counts =
            parse_u64_object(required_object(projection, "relationRecordCounts")?)?;
        let counts = parse_aggregate_counts(required_object(projection, "exactAggregateInputs")?)?;
        let stages = parse_stage_breakdown(
            required_object(projection, "exactStageExpectations")?,
            &counts,
        )?;
        Ok(Self {
            id,
            files,
            canonical_modules,
            import_edges,
            entity_counts,
            relation_counts,
            counts,
            stages,
        })
    }

    fn has_spatial(&self) -> bool {
        self.canonical_modules
            .iter()
            .any(|name| name == SPATIAL_MODULE)
    }

    fn load_raw_template(
        &self,
        repository_root: &Path,
    ) -> Result<CorridorTemplate, CurrentFixturesError> {
        let mut traffic = None;
        let mut spatial = None;
        let mut scenario_manifest_count = 0_u32;
        for binding in &self.files {
            let bytes = read_bound_file(repository_root, binding)?;
            let value: Value = serde_json::from_slice(&bytes).map_err(|source| {
                CurrentFixturesError::InvalidJson {
                    path: binding.path.clone(),
                    source,
                }
            })?;
            require_string(&value, "formatVersion", &binding.format_version)?;
            if binding.path.ends_with(".laneflow.json") {
                if traffic.replace(value).is_some() {
                    return Err(CurrentFixturesError::DuplicateArtifact(
                        "traffic".to_owned(),
                    ));
                }
            } else if binding.path.ends_with(".spatial.json") {
                if spatial.replace(value).is_some() {
                    return Err(CurrentFixturesError::DuplicateArtifact(
                        "spatial".to_owned(),
                    ));
                }
            } else if binding.path.ends_with(".scenario.json") {
                scenario_manifest_count =
                    scenario_manifest_count.checked_add(1).ok_or_else(|| {
                        CurrentFixturesError::Contract("scenario count overflow".into())
                    })?;
                validate_scenario_lineage(&value, &self.files)?;
            } else {
                return Err(CurrentFixturesError::Contract(format!(
                    "unsupported fixture artifact {}",
                    binding.path
                )));
            }
        }
        if self.has_spatial() != spatial.is_some() {
            return Err(CurrentFixturesError::Mismatch {
                path: format!("cases[{}].spatial", self.id),
                expected: self.has_spatial().to_string(),
                actual: spatial.is_some().to_string(),
            });
        }
        if self.has_spatial() && scenario_manifest_count != 1 {
            return Err(CurrentFixturesError::Mismatch {
                path: format!("cases[{}].scenarioManifestCount", self.id),
                expected: "1".to_owned(),
                actual: scenario_manifest_count.to_string(),
            });
        }
        if !self.has_spatial() && scenario_manifest_count != 0 {
            return Err(CurrentFixturesError::Mismatch {
                path: format!("cases[{}].scenarioManifestCount", self.id),
                expected: "0".to_owned(),
                actual: scenario_manifest_count.to_string(),
            });
        }
        let traffic =
            traffic.ok_or_else(|| CurrentFixturesError::Missing("traffic file".into()))?;
        Ok(build_current_fixture_raw_template(
            &traffic,
            spatial.as_ref(),
        )?)
    }

    fn validate_template(&self, template: &CorridorTemplate) -> Result<(), CurrentFixturesError> {
        let mut actual_domains = template.domain_counts();
        let mut expected_domains = self.entity_counts.clone();
        for (field, expected) in &self.relation_counts {
            if expected_domains.insert(field.clone(), *expected).is_some() {
                return Err(CurrentFixturesError::Contract(format!(
                    "{} duplicate domain count {field}",
                    self.id
                )));
            }
        }
        expected_domains.insert(
            "semanticOutputRecord".to_owned(),
            self.counts.semantic_output_record,
        );
        for (field, expected) in &expected_domains {
            if *expected == 0 {
                actual_domains.entry(field.clone()).or_insert(0);
            }
        }
        if actual_domains != expected_domains {
            return Err(CurrentFixturesError::Mismatch {
                path: format!("cases[{}].projection domain counts", self.id),
                expected: format!("{expected_domains:?}"),
                actual: format!("{actual_domains:?}"),
            });
        }
        if template.entities.len() as u64 != self.counts.identity_declaration_count
            || template.relations.len() as u64 != self.counts.source_relation_count
            || template.geometry.len() as u64 != self.counts.source_geometry_count
        {
            return Err(CurrentFixturesError::Contract(format!(
                "{} template aggregate mismatch",
                self.id
            )));
        }
        let stage_inputs = template.stage_input_counts();
        compare_exact(
            "identityFieldOccurrenceCount",
            self.counts.identity_field_occurrence_count,
            count(&stage_inputs, "identityFieldOccurrenceCount")?,
        )?;
        compare_exact(
            "profiledKeyOccurrenceCount",
            self.counts.profiled_key_occurrence_count,
            count(&stage_inputs, "profiledKeyOccurrenceCount")?,
        )?;
        let cross = self.counts.cross_module_reference_count;
        let joined_spatial_edges = template
            .geometry
            .iter()
            .map(|point| point.edge)
            .collect::<BTreeSet<_>>()
            .len() as u64;
        compare_exact("crossModuleReferenceCount", cross, joined_spatial_edges)?;
        compare_exact(
            "sourceReferenceCount",
            self.counts.source_reference_count,
            count(&stage_inputs, "sourceReferenceCount")?
                .checked_add(cross)
                .ok_or_else(|| CurrentFixturesError::Contract("reference count overflow".into()))?,
        )
    }
}

#[derive(Clone, Debug)]
struct BoundFixtureFile {
    path: String,
    format_version: String,
    byte_length: u64,
    sha256: String,
}

impl BoundFixtureFile {
    fn parse(value: &Value) -> Result<Self, CurrentFixturesError> {
        require_exact_keys(value, &["path", "formatVersion", "byteLength", "sha256"])?;
        Ok(Self {
            path: required_string(value, "path")?.to_owned(),
            format_version: required_string(value, "formatVersion")?.to_owned(),
            byte_length: required_u64(value, "byteLength")?,
            sha256: required_string(value, "sha256")?.to_owned(),
        })
    }
}

#[derive(Clone, Debug)]
struct FixedModule {
    name: &'static str,
    namespace: String,
    imports: Vec<&'static str>,
    declaration_count: u32,
    reference_count: u32,
    relation_count: u32,
    geometry_count: u32,
}

#[derive(Clone, Debug)]
struct FixedFixtureMaterialization {
    source_spans: Vec<SourceSpanRecord>,
    source_records: Vec<TypedAstStageRecord>,
    source_payload: Vec<u8>,
    typed_records: Vec<TypedAstStageRecord>,
    typed_payload: Vec<u8>,
    hir_records: Vec<HirStageRecord>,
    hir_payload: Vec<u8>,
    mir_records: Vec<MirLirStageRecord>,
    mir_payload: Vec<u8>,
    lir_records: Vec<MirLirStageRecord>,
    lir_payload: Vec<u8>,
    diagnostics: Vec<u8>,
    scratch: Vec<u64>,
    output: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(crate) struct CurrentFixtureCaseOutput {
    pub(crate) summary: CurrentFixtureCaseSummary,
    #[cfg(feature = "fixture-oracle")]
    pub(crate) template: CorridorTemplate,
    #[cfg(feature = "fixture-oracle")]
    pub(crate) records: Vec<SemanticRecord>,
    #[cfg(feature = "fixture-oracle")]
    pub(crate) semantic_record_stream: Vec<u8>,
    #[cfg(feature = "fixture-oracle")]
    materialization: FixedFixtureMaterialization,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentFixtureCaseSummary {
    pub case_id: String,
    pub counts: IdentityAggregateCounts,
    pub stages: StageBreakdown,
    pub entity_counts: BTreeMap<String, u64>,
    pub relation_record_counts: BTreeMap<String, u64>,
    pub record_kind_counts: BTreeMap<String, u64>,
    pub semantic_digest_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentFixturesKnownVectorDocument {
    pub schema: &'static str,
    pub schema_version: u32,
    pub source_workload_manifest_sha256: String,
    pub workload_id: &'static str,
    pub scalable: bool,
    pub cases: Vec<CurrentFixtureCaseSummary>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentFixturesOracleVerificationReport {
    pub checked_cases: u32,
    pub production_loader_cases: u32,
    pub independent_identity_and_stream_checked: bool,
    pub scenario_manifest_emits_no_records: bool,
    pub excluded_from_budget_and_candidate_ranking: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentFixturesChildReport {
    pub schema: String,
    pub schema_version: u32,
    pub binary_id: String,
    pub child_pid: u32,
    pub cases: Vec<CurrentFixtureCaseSummary>,
    pub verification: CurrentFixturesOracleVerificationReport,
}

pub fn measure_current_fixtures_child(
    trusted: &TrustedContract,
) -> Result<CurrentFixturesChildReport, CurrentFixturesOracleError> {
    Ok(CurrentFixturesChildReport {
        schema: CURRENT_FIXTURES_CHILD_SCHEMA.to_owned(),
        schema_version: CURRENT_FIXTURES_CHILD_SCHEMA_VERSION,
        binary_id: crate::ORACLE_BINARY_ID.to_owned(),
        child_pid: std::process::id(),
        cases: build_current_fixture_summaries(trusted)?,
        verification: verify_current_fixtures_oracle(trusted)?,
    })
}

pub fn build_current_fixtures_known_vectors(
    trusted: &TrustedContract,
) -> Result<CurrentFixturesKnownVectorDocument, CurrentFixturesError> {
    let contract = CurrentFixturesContract::from_manifest(&trusted.workload_manifest)?;
    let generator = trusted
        .generator_contract()
        .map_err(|error| CurrentFixturesError::Contract(error.to_string()))?;
    let identity = trusted
        .identity_contract()
        .map_err(|error| CurrentFixturesError::Contract(error.to_string()))?;
    let stage = trusted
        .stage_contract()
        .map_err(|error| CurrentFixturesError::Contract(error.to_string()))?;
    let mut cases = Vec::with_capacity(contract.cases.len());
    for case in contract.cases() {
        cases.push(
            build_current_fixture_case(
                &generator,
                &identity,
                &stage,
                case,
                &crate::repository_root(),
            )?
            .summary,
        );
    }
    Ok(CurrentFixturesKnownVectorDocument {
        schema: CURRENT_FIXTURES_KNOWN_VECTOR_SCHEMA,
        schema_version: 1,
        source_workload_manifest_sha256: trusted.descriptor.workload_manifest.sha256.clone(),
        workload_id: CURRENT_FIXTURES_WORKLOAD_ID,
        scalable: false,
        cases,
    })
}

pub fn build_current_fixture_summaries(
    trusted: &TrustedContract,
) -> Result<Vec<CurrentFixtureCaseSummary>, CurrentFixturesError> {
    Ok(build_current_fixtures_known_vectors(trusted)?.cases)
}

#[cfg(feature = "fixture-oracle")]
pub fn verify_current_fixtures_oracle(
    trusted: &TrustedContract,
) -> Result<CurrentFixturesOracleVerificationReport, CurrentFixturesOracleError> {
    let contract = CurrentFixturesContract::from_manifest(&trusted.workload_manifest)?;
    let generator = trusted.generator_contract()?;
    let identity = trusted.identity_contract()?;
    let stage = trusted.stage_contract()?;
    let repository_root = crate::repository_root();
    let mut checked_cases = 0_u32;
    for case in contract.cases() {
        let produced =
            build_current_fixture_case(&generator, &identity, &stage, case, &repository_root)?;
        let production_template =
            crate::corridor_fixture_oracle::build_production_loader_fixture_case(&case.id)
                .map_err(CurrentFixturesOracleError::ProductionLoader)?;
        if produced.template != production_template {
            return Err(CurrentFixturesOracleError::TemplateMismatch {
                case_id: case.id.clone(),
                details: describe_template_mismatch(&produced.template, &production_template),
            });
        }
        let oracle = crate::corridor_oracle::build_fixed_fixture_oracle_records(
            &trusted.workload_manifest,
            CURRENT_FIXTURES_WORKLOAD_ID,
            &production_template,
        )?;
        if produced.records != oracle.records
            || produced.semantic_record_stream != oracle.stream
            || produced.materialization.output != produced.semantic_record_stream
        {
            return Err(CurrentFixturesOracleError::RecordStreamMismatch(
                case.id.clone(),
            ));
        }
        checked_cases = checked_cases
            .checked_add(1)
            .ok_or_else(|| CurrentFixturesOracleError::Contract("case count overflow".into()))?;
    }
    Ok(CurrentFixturesOracleVerificationReport {
        checked_cases,
        production_loader_cases: checked_cases,
        independent_identity_and_stream_checked: true,
        scenario_manifest_emits_no_records: true,
        excluded_from_budget_and_candidate_ranking: true,
    })
}

#[cfg(not(feature = "fixture-oracle"))]
pub fn verify_current_fixtures_oracle(
    _trusted: &TrustedContract,
) -> Result<CurrentFixturesOracleVerificationReport, CurrentFixturesOracleError> {
    Err(CurrentFixturesOracleError::ProductionLoader(
        "fixture-oracle feature is required".to_owned(),
    ))
}

pub(crate) fn build_current_fixture_case(
    generator: &GeneratorContract,
    identity: &IdentityContract,
    stage: &StageContract,
    case: &CurrentFixtureCaseContract,
    repository_root: &Path,
) -> Result<CurrentFixtureCaseOutput, CurrentFixturesError> {
    let template = case.load_raw_template(repository_root)?;
    case.validate_template(&template)?;
    let modules = build_modules(generator, case, &template)?;
    validate_module_graph(case, &modules)?;
    let declarations = compile_fixed_declarations(identity, &template, &modules)?;
    let (mut records, unsorted_records) =
        compile_semantic_records(identity, &template, &declarations, 1)?;
    records.sort_by(|left, right| left.canonical_key().cmp(&right.canonical_key()));
    let stream = encode_semantic_record_stream(identity, &records);
    validate_semantic_results(case, &template, &records, &stream)?;
    let materialization = materialize_fixed_stages(
        stage,
        case,
        &template,
        &modules,
        &declarations,
        &unsorted_records,
        &records,
        &stream,
    )?;
    verify_materialization(&materialization, &case.counts, &case.stages)?;
    Ok(CurrentFixtureCaseOutput {
        summary: CurrentFixtureCaseSummary {
            case_id: case.id.clone(),
            counts: case.counts.clone(),
            stages: case.stages.clone(),
            entity_counts: case.entity_counts.clone(),
            relation_record_counts: case.relation_counts.clone(),
            record_kind_counts: count_record_kinds(&records),
            semantic_digest_sha256: lower_hex(&Sha256::digest(&stream)),
        },
        #[cfg(feature = "fixture-oracle")]
        template,
        #[cfg(feature = "fixture-oracle")]
        records,
        #[cfg(feature = "fixture-oracle")]
        semantic_record_stream: stream,
        #[cfg(feature = "fixture-oracle")]
        materialization,
    })
}

fn validate_module_graph(
    case: &CurrentFixtureCaseContract,
    modules: &[FixedModule],
) -> Result<(), CurrentFixturesError> {
    let import_count = modules
        .iter()
        .try_fold(0_u64, |sum, module| {
            sum.checked_add(module.imports.len() as u64)
        })
        .ok_or_else(|| CurrentFixturesError::Contract("module import count overflow".into()))?;
    compare_exact(
        "moduleCount",
        case.counts.module_count,
        modules.len() as u64,
    )?;
    compare_exact(
        "sourceDocumentCount",
        case.counts.source_document_count,
        modules.len() as u64,
    )?;
    compare_exact(
        "importEdgeCount",
        case.counts.import_edge_count,
        import_count,
    )?;
    compare_exact(
        "maximumImportDepth",
        case.counts.maximum_import_depth,
        u64::from(modules.iter().any(|module| !module.imports.is_empty())),
    )
}

fn build_modules(
    generator: &GeneratorContract,
    case: &CurrentFixtureCaseContract,
    template: &CorridorTemplate,
) -> Result<Vec<FixedModule>, CurrentFixturesError> {
    if case.canonical_modules == [TRAFFIC_MODULE]
        && case.import_edges.is_empty()
        && !case.has_spatial()
    {
        return Ok(vec![FixedModule {
            name: TRAFFIC_MODULE,
            namespace: fixed_namespace(generator, TRAFFIC_MODULE),
            imports: Vec::new(),
            declaration_count: u32_count(template.entities.len(), "traffic declarations")?,
            reference_count: u32_count(case.counts.source_reference_count, "traffic references")?,
            relation_count: u32_count(template.relations.len(), "traffic relations")?,
            geometry_count: 0,
        }]);
    }
    if case.canonical_modules != [TRAFFIC_MODULE, SPATIAL_MODULE]
        || case.import_edges != [(SPATIAL_MODULE.to_owned(), TRAFFIC_MODULE.to_owned())]
    {
        return Err(CurrentFixturesError::Contract(format!(
            "{} canonical module graph mismatch",
            case.id
        )));
    }
    let geometry_references = u64::try_from(template.geometry.len())
        .map_err(|_| CurrentFixturesError::Contract("geometry reference count overflow".into()))?;
    let spatial_references = geometry_references
        .checked_add(case.counts.cross_module_reference_count)
        .ok_or_else(|| CurrentFixturesError::Contract("spatial reference count overflow".into()))?;
    let traffic_references = case
        .counts
        .source_reference_count
        .checked_sub(spatial_references)
        .ok_or_else(|| CurrentFixturesError::Contract("traffic reference underflow".into()))?;
    Ok(vec![
        FixedModule {
            name: TRAFFIC_MODULE,
            namespace: fixed_namespace(generator, TRAFFIC_MODULE),
            imports: Vec::new(),
            declaration_count: u32_count(template.entities.len() - 1, "traffic declarations")?,
            reference_count: u32_count(traffic_references, "traffic references")?,
            relation_count: u32_count(template.relations.len(), "traffic relations")?,
            geometry_count: 0,
        },
        FixedModule {
            name: SPATIAL_MODULE,
            namespace: fixed_namespace(generator, SPATIAL_MODULE),
            imports: vec![TRAFFIC_MODULE],
            declaration_count: 1,
            reference_count: u32_count(spatial_references, "spatial references")?,
            relation_count: 0,
            geometry_count: u32_count(template.geometry.len(), "spatial geometry")?,
        },
    ])
}

fn fixed_namespace(generator: &GeneratorContract, module: &str) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(generator.namespace_domain.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&generator.generator_version.to_le_bytes());
    bytes.extend_from_slice(&generator.base_seed.to_le_bytes());
    put_string(&mut bytes, CURRENT_FIXTURES_WORKLOAD_ID);
    put_string(&mut bytes, NOT_APPLICABLE_GRAPH_PROFILE);
    put_string(&mut bytes, module);
    let digest = blake3::hash(&bytes);
    lower_hex(
        &digest.as_bytes()[generator.namespace_digest_offset
            ..generator.namespace_digest_offset + generator.namespace_digest_length],
    )
}

fn compile_fixed_declarations(
    identity: &IdentityContract,
    template: &CorridorTemplate,
    modules: &[FixedModule],
) -> Result<Vec<CompiledDeclaration>, CurrentFixturesError> {
    let bindings = identity
        .bindings
        .iter()
        .map(|binding| (binding.entity_kind_code, binding))
        .collect::<BTreeMap<_, _>>();
    let traffic_namespace = modules
        .iter()
        .find(|module| module.name == TRAFFIC_MODULE)
        .ok_or_else(|| CurrentFixturesError::Missing("traffic namespace".into()))?
        .namespace
        .as_bytes();
    let spatial_namespace = modules
        .iter()
        .find(|module| module.name == SPATIAL_MODULE)
        .map(|module| module.namespace.as_bytes());
    let mut stable_ids = BTreeMap::<EntityRef, [u8; 16]>::new();
    let mut declarations = Vec::with_capacity(template.entities.len());
    for entity in &template.entities {
        let binding = bindings.get(&entity.reference.kind).ok_or_else(|| {
            CurrentFixturesError::Missing(format!("identityBindings[{}]", entity.reference.kind))
        })?;
        let profiled_count = u32_count(
            binding
                .fields
                .iter()
                .filter(|field| matches!(field.value, IdentityFieldValue::ProfiledKey { .. }))
                .count(),
            "profiled field count",
        )?;
        let namespace = if entity.reference.kind == 22 {
            spatial_namespace.ok_or_else(|| {
                CurrentFixturesError::Missing("CanonicalFrame spatial namespace".into())
            })?
        } else {
            traffic_namespace
        };
        let mut fields = Vec::with_capacity(binding.fields.len());
        for field in &binding.fields {
            let bytes = match field.value {
                IdentityFieldValue::Namespace => namespace.to_vec(),
                IdentityFieldValue::ProfiledKey { kind, local } => {
                    let expanded = entity
                        .reference
                        .local
                        .checked_mul(profiled_count)
                        .and_then(|base| base.checked_add(local))
                        .ok_or_else(|| {
                            CurrentFixturesError::Contract("profiled key overflow".into())
                        })?;
                    format!("fixture/{kind:02x}/{expanded:08x}").into_bytes()
                }
                IdentityFieldValue::StableId { kind, .. } => {
                    let target = entity
                        .identity_references
                        .get(&field.tag)
                        .copied()
                        .ok_or_else(|| {
                            CurrentFixturesError::Missing(format!(
                                "identity reference {:?}/{}",
                                entity.reference, field.tag
                            ))
                        })?;
                    if target.kind != kind {
                        return Err(CurrentFixturesError::Contract(
                            "identity reference kind mismatch".into(),
                        ));
                    }
                    stable_ids
                        .get(&target)
                        .copied()
                        .ok_or_else(|| {
                            CurrentFixturesError::Missing(format!(
                                "stable identity target {target:?}"
                            ))
                        })?
                        .to_vec()
                }
            };
            fields.push(IdentityField {
                tag: field.tag,
                bytes,
            });
        }
        let canonical = encode_canonical_identity(
            identity.identity_encoding_version(),
            entity.reference.kind,
            &fields,
        );
        let digest = blake3::hash(&[STABLE_ID_DOMAIN, canonical.as_slice()].concat());
        let mut stable_id = [0_u8; 16];
        stable_id.copy_from_slice(&digest.as_bytes()[..16]);
        if stable_ids.insert(entity.reference, stable_id).is_some() {
            return Err(CurrentFixturesError::DuplicateArtifact(format!(
                "declaration {:?}",
                entity.reference
            )));
        }
        declarations.push(CompiledDeclaration {
            owner: UnitEntityRef {
                unit: FIXED_UNIT_INDEX,
                entity: entity.reference,
            },
            stable_id,
            fields,
        });
    }
    Ok(declarations)
}

fn validate_semantic_results(
    case: &CurrentFixtureCaseContract,
    template: &CorridorTemplate,
    records: &[SemanticRecord],
    stream: &[u8],
) -> Result<(), CurrentFixturesError> {
    compare_exact(
        "semanticOutputRecord",
        case.counts.semantic_output_record,
        records.len() as u64,
    )?;
    let payload_bytes = records.iter().try_fold(0_u64, |sum, record| {
        sum.checked_add(record.payload.len() as u64)
            .ok_or_else(|| CurrentFixturesError::Contract("semantic payload overflow".into()))
    })?;
    compare_exact(
        "semanticPayloadByteCount",
        case.counts.semantic_payload_byte_count,
        payload_bytes,
    )?;
    compare_exact(
        "outputByteCount",
        case.counts.output_byte_count,
        stream.len() as u64,
    )?;
    compare_exact(
        "semantic template count",
        records.len() as u64,
        (template.entities.len() + template.relations.len() + template.geometry.len()) as u64,
    )
}

#[allow(clippy::too_many_arguments)]
fn materialize_fixed_stages(
    stage: &StageContract,
    case: &CurrentFixtureCaseContract,
    template: &CorridorTemplate,
    modules: &[FixedModule],
    declarations: &[CompiledDeclaration],
    unsorted_records: &[SemanticRecord],
    records: &[SemanticRecord],
    stream: &[u8],
) -> Result<FixedFixtureMaterialization, CurrentFixturesError> {
    let FixedSourceMaterialization {
        source_spans,
        source_records,
        source_payload,
        category_spans,
    } = materialize_fixed_source(stage, case, template, modules)?;
    let mut typed_records = source_records
        .iter()
        .filter(|record| {
            record.record_kind == stage.record_kind_module
                || record.record_kind == stage.record_kind_import
        })
        .copied()
        .collect::<Vec<_>>();
    append_fixed_typed_entities(
        &mut typed_records,
        stage,
        template,
        declarations,
        modules,
        &category_spans,
    )?;
    append_fixed_typed_category_records(
        &mut typed_records,
        stage,
        case,
        &category_spans,
        &source_records,
    )?;
    compare_exact(
        "typedAst.recordCount",
        case.stages.typed_ast.record_count,
        typed_records.len() as u64,
    )?;
    let mut typed_payload = source_payload.clone();
    encode_source_spans(&mut typed_payload, &source_spans);

    let mut hir_records = typed_records
        .iter()
        .map(|typed| HirStageRecord {
            record_kind: if typed.record_kind == stage.record_kind_declaration {
                stage.record_kind_symbol
            } else {
                typed.record_kind
            },
            entity_kind: typed.entity_kind,
            module_ordinal: typed.module_ordinal,
            symbol_ordinal: typed.owner_local_index,
            resolved_target_ordinal: stage.absent_ordinal,
            payload_offset: 0,
            payload_length: 0,
        })
        .collect::<Vec<_>>();
    hir_records.resize(
        usize_count(case.stages.hir.record_count, "HIR record count")?,
        HirStageRecord::default(),
    );
    let string_start = usize_count(case.counts.source_byte_count, "source bytes")?;
    let mut hir_payload = source_payload[string_start..].to_vec();
    hir_payload.resize(
        usize_count(case.stages.hir.payload_logical_bytes, "HIR payload")?,
        0,
    );

    let (mir_records, mir_payload) = materialize_semantic_records(unsorted_records)?;
    let (lir_records, lir_payload) = materialize_semantic_records(records)?;
    let scratch = vec![
        0_u64;
        usize_count(
            case.stages.scratch.payload_logical_bytes / 8,
            "scratch words"
        )?
    ];
    Ok(FixedFixtureMaterialization {
        source_spans,
        source_records,
        source_payload,
        typed_records,
        typed_payload,
        hir_records,
        hir_payload,
        mir_records,
        mir_payload,
        lir_records,
        lir_payload,
        diagnostics: Vec::new(),
        scratch,
        output: stream.to_vec(),
    })
}

#[derive(Default)]
struct CategorySpans {
    declarations: Vec<u32>,
    references: Vec<u32>,
    relations: Vec<u32>,
    geometry: Vec<u32>,
}

struct FixedSourceMaterialization {
    source_spans: Vec<SourceSpanRecord>,
    source_records: Vec<TypedAstStageRecord>,
    source_payload: Vec<u8>,
    category_spans: CategorySpans,
}

fn materialize_fixed_source(
    stage: &StageContract,
    case: &CurrentFixtureCaseContract,
    template: &CorridorTemplate,
    modules: &[FixedModule],
) -> Result<FixedSourceMaterialization, CurrentFixturesError> {
    let mut payload = Vec::with_capacity(usize_count(
        case.stages.source_input.payload_logical_bytes,
        "source",
    )?);
    let mut spans = Vec::with_capacity(usize_count(case.counts.source_span_count, "source spans")?);
    let mut span_records = Vec::with_capacity(spans.capacity());
    let mut categories = CategorySpans::default();
    for (module_ordinal, module) in modules.iter().enumerate() {
        let module_ordinal = u32_count(module_ordinal, "module ordinal")?;
        let mut line = 1_u32;
        for (record_kind, token, count, destination) in [
            (
                stage.record_kind_declaration,
                stage.declaration_token.as_str(),
                module.declaration_count,
                &mut categories.declarations,
            ),
            (
                stage.record_kind_reference,
                stage.reference_token.as_str(),
                module.reference_count,
                &mut categories.references,
            ),
            (
                stage.record_kind_relation,
                stage.relation_token.as_str(),
                module.relation_count,
                &mut categories.relations,
            ),
            (
                stage.record_kind_geometry,
                stage.geometry_token.as_str(),
                module.geometry_count,
                &mut categories.geometry,
            ),
        ] {
            for local in 0..count {
                let offset = payload.len();
                payload.extend_from_slice(token.as_bytes());
                payload.push(b'/');
                payload.extend_from_slice(format!("{local:08x}").as_bytes());
                payload.push(b'\n');
                let length = payload.len() - offset;
                let span_ordinal = u32_count(spans.len(), "source span ordinal")?;
                destination.push(span_ordinal);
                spans.push(SourceSpanRecord {
                    source_document_ordinal: module_ordinal,
                    start_line: line,
                    start_column: 1,
                    end_line: line,
                    end_column: u32_count(length, "source token length")?,
                });
                span_records.push(TypedAstStageRecord {
                    record_kind,
                    entity_kind: 0,
                    module_ordinal,
                    source_span_ordinal: span_ordinal,
                    owner_local_index: local,
                    payload_offset: offset as u64,
                    payload_length: length as u64,
                });
                line = line
                    .checked_add(1)
                    .ok_or_else(|| CurrentFixturesError::Contract("source line overflow".into()))?;
            }
        }
    }
    compare_exact(
        "sourceByteCount",
        case.counts.source_byte_count,
        payload.len() as u64,
    )?;
    let string_base = payload.len();
    let strings = fixed_strings(case, template, modules)?;
    let mut string_offsets = Vec::with_capacity(strings.len());
    for string in &strings {
        string_offsets.push((payload.len(), string.len()));
        payload.extend_from_slice(string);
    }
    compare_exact(
        "totalStringBytes",
        case.counts.total_string_bytes,
        (payload.len() - string_base) as u64,
    )?;
    let mut records = Vec::with_capacity(usize_count(
        case.stages.source_input.record_count,
        "source records",
    )?);
    let mut string_index = 0_usize;
    for (module_ordinal, _) in modules.iter().enumerate() {
        let (offset, length) = string_offsets[string_index];
        string_index += 1;
        records.push(TypedAstStageRecord {
            record_kind: stage.record_kind_module,
            entity_kind: 0,
            module_ordinal: u32_count(module_ordinal, "module ordinal")?,
            source_span_ordinal: stage.absent_ordinal,
            owner_local_index: stage.absent_ordinal,
            payload_offset: offset as u64,
            payload_length: length as u64,
        });
    }
    string_index += modules.len();
    for (module_ordinal, module) in modules.iter().enumerate() {
        for (local_index, _) in module.imports.iter().enumerate() {
            let (offset, length) = string_offsets[string_index];
            string_index += 1;
            records.push(TypedAstStageRecord {
                record_kind: stage.record_kind_import,
                entity_kind: 0,
                module_ordinal: u32_count(module_ordinal, "module ordinal")?,
                source_span_ordinal: stage.absent_ordinal,
                owner_local_index: u32_count(local_index, "import ordinal")?,
                payload_offset: offset as u64,
                payload_length: length as u64,
            });
        }
    }
    records.extend(span_records);
    compare_exact(
        "sourceInput.recordCount",
        case.stages.source_input.record_count,
        records.len() as u64,
    )?;
    Ok(FixedSourceMaterialization {
        source_spans: spans,
        source_records: records,
        source_payload: payload,
        category_spans: categories,
    })
}

fn fixed_strings(
    case: &CurrentFixtureCaseContract,
    template: &CorridorTemplate,
    modules: &[FixedModule],
) -> Result<Vec<Vec<u8>>, CurrentFixturesError> {
    let mut strings = Vec::new();
    strings.extend(modules.iter().map(|module| module.name.as_bytes().to_vec()));
    strings.extend(
        modules
            .iter()
            .map(|module| format!("fixture/{}/{}.lfsynthetic", case.id, module.name).into_bytes()),
    );
    for module in modules {
        strings.extend(
            module
                .imports
                .iter()
                .map(|target| target.as_bytes().to_vec()),
        );
    }
    let namespace_by_kind = modules
        .iter()
        .map(|module| (module.name, module.namespace.as_bytes().to_vec()))
        .collect::<BTreeMap<_, _>>();
    for entity in &template.entities {
        let module = if entity.reference.kind == 22 {
            SPATIAL_MODULE
        } else {
            TRAFFIC_MODULE
        };
        strings.push(
            namespace_by_kind
                .get(module)
                .cloned()
                .ok_or_else(|| CurrentFixturesError::Missing(format!("{module} namespace")))?,
        );
        let profiled_count = u32_count(
            profiled_key_field_count(entity.reference.kind),
            "profiled fields",
        )?;
        for local in 0..profiled_count {
            let expanded = entity
                .reference
                .local
                .checked_mul(profiled_count)
                .and_then(|base| base.checked_add(local))
                .ok_or_else(|| CurrentFixturesError::Contract("profiled key overflow".into()))?;
            strings
                .push(format!("fixture/{:02x}/{expanded:08x}", entity.reference.kind).into_bytes());
        }
    }
    for entity in &template.entities {
        for target in entity.identity_references.values() {
            strings.push(fixed_reference_spelling(*target, modules)?);
        }
    }
    let mut relation_targets = Vec::new();
    for relation in &template.relations {
        relation.append_stable_references(&mut relation_targets);
    }
    for target in relation_targets {
        strings.push(fixed_reference_spelling(target, modules)?);
    }
    for point in &template.geometry {
        strings.push(fixed_reference_spelling(point.frame, modules)?);
    }
    if case.has_spatial() {
        for local in 0..case.counts.cross_module_reference_count {
            strings.push(fixed_reference_spelling(
                EntityRef {
                    kind: 4,
                    local: u32_count(local, "cross-module edge ordinal")?,
                },
                modules,
            )?);
        }
    }
    compare_exact(
        "stringItemCount",
        case.counts.string_item_count,
        strings.len() as u64,
    )?;
    let maximum = strings.iter().map(Vec::len).max().unwrap_or(0) as u64;
    compare_exact(
        "maximumStringBytes",
        case.counts.maximum_string_bytes,
        maximum,
    )?;
    Ok(strings)
}

fn fixed_reference_spelling(
    target: EntityRef,
    modules: &[FixedModule],
) -> Result<Vec<u8>, CurrentFixturesError> {
    let module_name = if target.kind == 22 {
        SPATIAL_MODULE
    } else {
        TRAFFIC_MODULE
    };
    let module_ordinal = modules
        .iter()
        .position(|module| module.name == module_name)
        .ok_or_else(|| CurrentFixturesError::Missing(format!("{module_name} module")))?;
    Ok(format!(
        "reference/{:02x}/{:08x}/{:08x}",
        target.kind, module_ordinal, target.local
    )
    .into_bytes())
}

fn append_fixed_typed_entities(
    records: &mut Vec<TypedAstStageRecord>,
    stage: &StageContract,
    template: &CorridorTemplate,
    declarations: &[CompiledDeclaration],
    modules: &[FixedModule],
    spans: &CategorySpans,
) -> Result<(), CurrentFixturesError> {
    for (declaration_span, (entity, declaration)) in
        template.entities.iter().zip(declarations).enumerate()
    {
        let module_ordinal = if entity.reference.kind == 22 {
            modules
                .iter()
                .position(|module| module.name == SPATIAL_MODULE)
                .ok_or_else(|| CurrentFixturesError::Missing("spatial module".into()))?
        } else {
            0
        };
        let source_span = *spans
            .declarations
            .get(declaration_span)
            .ok_or_else(|| CurrentFixturesError::Missing("declaration source span".into()))?;
        records.push(TypedAstStageRecord {
            record_kind: stage.record_kind_declaration,
            entity_kind: entity.reference.kind,
            module_ordinal: u32_count(module_ordinal, "entity module ordinal")?,
            source_span_ordinal: source_span,
            owner_local_index: entity.reference.local,
            payload_offset: 0,
            payload_length: 0,
        });
        for (field_index, _) in declaration.fields.iter().enumerate() {
            records.push(TypedAstStageRecord {
                record_kind: stage.record_kind_identity_field,
                entity_kind: entity.reference.kind,
                module_ordinal: u32_count(module_ordinal, "field module ordinal")?,
                source_span_ordinal: source_span,
                owner_local_index: u32_count(field_index, "identity field ordinal")?,
                payload_offset: 0,
                payload_length: 0,
            });
        }
    }
    Ok(())
}

fn append_fixed_typed_category_records(
    records: &mut Vec<TypedAstStageRecord>,
    stage: &StageContract,
    case: &CurrentFixtureCaseContract,
    spans: &CategorySpans,
    source_records: &[TypedAstStageRecord],
) -> Result<(), CurrentFixturesError> {
    for (record_kind, count, source_spans) in [
        (
            stage.record_kind_reference,
            case.counts.source_reference_count,
            &spans.references,
        ),
        (
            stage.record_kind_relation,
            case.counts.source_relation_count,
            &spans.relations,
        ),
        (
            stage.record_kind_geometry,
            case.counts.source_geometry_count,
            &spans.geometry,
        ),
    ] {
        if source_spans.len() as u64 != count {
            return Err(CurrentFixturesError::Contract(
                "typed category source span count mismatch".into(),
            ));
        }
        for source_span in source_spans {
            let source = source_records
                .iter()
                .find(|record| record.source_span_ordinal == *source_span)
                .ok_or_else(|| {
                    CurrentFixturesError::Missing("typed category source record".into())
                })?;
            records.push(TypedAstStageRecord {
                record_kind,
                entity_kind: 0,
                module_ordinal: source.module_ordinal,
                source_span_ordinal: *source_span,
                owner_local_index: source.owner_local_index,
                payload_offset: 0,
                payload_length: 0,
            });
        }
    }
    Ok(())
}

fn materialize_semantic_records(
    records: &[SemanticRecord],
) -> Result<(Vec<MirLirStageRecord>, Vec<u8>), CurrentFixturesError> {
    let mut stage_records = Vec::with_capacity(records.len());
    let mut payload = Vec::new();
    for record in records {
        let offset = payload.len();
        payload.extend_from_slice(&record.payload);
        stage_records.push(MirLirStageRecord {
            record_kind: record.record_kind,
            entity_kind: record.entity_kind_code,
            stable_id: record.stable_id,
            owner_ordinal: record.owner_ordinal,
            local_index: record.local_index,
            payload_offset: offset as u64,
            payload_length: record.payload.len() as u64,
        });
    }
    Ok((stage_records, payload))
}

fn verify_materialization(
    materialization: &FixedFixtureMaterialization,
    counts: &IdentityAggregateCounts,
    stages: &StageBreakdown,
) -> Result<(), CurrentFixturesError> {
    for (name, records, payload, expected) in [
        (
            "sourceInput",
            materialization.source_records.len(),
            materialization.source_payload.len(),
            stages.source_input,
        ),
        (
            "typedAst",
            materialization.typed_records.len(),
            materialization.typed_payload.len(),
            stages.typed_ast,
        ),
        (
            "hir",
            materialization.hir_records.len(),
            materialization.hir_payload.len(),
            stages.hir,
        ),
        (
            "mir",
            materialization.mir_records.len(),
            materialization.mir_payload.len(),
            stages.mir,
        ),
        (
            "canonicalLir",
            materialization.lir_records.len(),
            materialization.lir_payload.len(),
            stages.canonical_lir,
        ),
    ] {
        if records as u64 != expected.record_count
            || payload as u64 != expected.payload_logical_bytes
        {
            return Err(CurrentFixturesError::Mismatch {
                path: format!("{name} materialization"),
                expected: format!(
                    "{} records / {} payload bytes",
                    expected.record_count, expected.payload_logical_bytes
                ),
                actual: format!("{records} records / {payload} payload bytes"),
            });
        }
    }
    if materialization.source_spans.len() as u64 != counts.source_span_count
        || !materialization.diagnostics.is_empty()
        || materialization.scratch.len() as u64 * 8 != stages.scratch.logical_bytes
        || materialization.output.len() as u64 != stages.output_construction.logical_bytes
    {
        return Err(CurrentFixturesError::Contract(
            "non-primary fixed stage materialization mismatch".into(),
        ));
    }
    Ok(())
}

fn parse_aggregate_counts(value: &Value) -> Result<IdentityAggregateCounts, CurrentFixturesError> {
    require_exact_keys(
        value,
        &[
            "moduleCount",
            "importEdgeCount",
            "crossModuleReferenceCount",
            "maximumImportDepth",
            "sourceDocumentCount",
            "sourceByteCount",
            "identityDeclarationCount",
            "sourceDeclarationCount",
            "sourceSpanCount",
            "identityFieldOccurrenceCount",
            "profiledKeyOccurrenceCount",
            "sourceReferenceCount",
            "sourceRelationCount",
            "sourceGeometryCount",
            "symbolCount",
            "stringItemCount",
            "maximumStringBytes",
            "totalStringBytes",
            "diagnosticCount",
            "semanticOutputRecord",
            "semanticPayloadByteCount",
            "logicalByteCount",
            "outputByteCount",
        ],
    )?;
    Ok(IdentityAggregateCounts {
        module_count: required_u64(value, "moduleCount")?,
        import_edge_count: required_u64(value, "importEdgeCount")?,
        cross_module_reference_count: required_u64(value, "crossModuleReferenceCount")?,
        maximum_import_depth: required_u64(value, "maximumImportDepth")?,
        source_document_count: required_u64(value, "sourceDocumentCount")?,
        source_byte_count: required_u64(value, "sourceByteCount")?,
        identity_declaration_count: required_u64(value, "identityDeclarationCount")?,
        source_declaration_count: required_u64(value, "sourceDeclarationCount")?,
        source_span_count: required_u64(value, "sourceSpanCount")?,
        identity_field_occurrence_count: required_u64(value, "identityFieldOccurrenceCount")?,
        profiled_key_occurrence_count: required_u64(value, "profiledKeyOccurrenceCount")?,
        source_reference_count: required_u64(value, "sourceReferenceCount")?,
        source_relation_count: required_u64(value, "sourceRelationCount")?,
        source_geometry_count: required_u64(value, "sourceGeometryCount")?,
        symbol_count: required_u64(value, "symbolCount")?,
        string_item_count: required_u64(value, "stringItemCount")?,
        maximum_string_bytes: required_u64(value, "maximumStringBytes")?,
        total_string_bytes: required_u64(value, "totalStringBytes")?,
        diagnostic_count: required_u64(value, "diagnosticCount")?,
        semantic_output_record: required_u64(value, "semanticOutputRecord")?,
        semantic_payload_byte_count: required_u64(value, "semanticPayloadByteCount")?,
        logical_byte_count: required_u64(value, "logicalByteCount")?,
        output_byte_count: required_u64(value, "outputByteCount")?,
    })
}

fn parse_stage_breakdown(
    value: &Value,
    counts: &IdentityAggregateCounts,
) -> Result<StageBreakdown, CurrentFixturesError> {
    require_exact_keys(
        value,
        &[
            "sourceInput",
            "typedAst",
            "hir",
            "mir",
            "canonicalLir",
            "diagnostics",
            "scratch",
            "outputConstruction",
        ],
    )?;
    let source_input = stage_shape(
        required_object(value, "sourceInput")?,
        counts.source_byte_count + counts.total_string_bytes,
        32,
        32,
    )?;
    let typed_ast = complete_stage_shape(required_object(value, "typedAst")?)?;
    let hir = complete_stage_shape(required_object(value, "hir")?)?;
    let mir = complete_stage_shape(required_object(value, "mir")?)?;
    let canonical_lir = complete_stage_shape(required_object(value, "canonicalLir")?)?;
    let diagnostics = stage_shape(required_object(value, "diagnostics")?, 0, 0, 0)?;
    let scratch_value = required_object(value, "scratch")?;
    require_exact_keys(scratch_value, &["recordCount", "logicalBytes"])?;
    let scratch_logical = required_u64(scratch_value, "logicalBytes")?;
    let scratch = StageShape {
        record_count: required_u64(scratch_value, "recordCount")?,
        payload_logical_bytes: scratch_logical,
        logical_bytes: scratch_logical,
        record_allocation_bytes: scratch_logical,
    };
    let output_value = required_object(value, "outputConstruction")?;
    require_exact_keys(output_value, &["recordCount", "logicalBytes"])?;
    let output_logical = required_u64(output_value, "logicalBytes")?;
    let output_construction = StageShape {
        record_count: required_u64(output_value, "recordCount")?,
        payload_logical_bytes: counts.semantic_payload_byte_count,
        logical_bytes: output_logical,
        record_allocation_bytes: output_logical,
    };
    let stages = StageBreakdown {
        source_input,
        typed_ast,
        hir,
        mir,
        canonical_lir,
        diagnostics,
        scratch,
        output_construction,
    };
    validate_stage_formulas(&stages, counts)?;
    Ok(stages)
}

fn complete_stage_shape(value: &Value) -> Result<StageShape, CurrentFixturesError> {
    require_exact_keys(
        value,
        &[
            "recordCount",
            "payloadLogicalBytes",
            "logicalBytes",
            "recordAllocationBytes",
        ],
    )?;
    Ok(StageShape {
        record_count: required_u64(value, "recordCount")?,
        payload_logical_bytes: required_u64(value, "payloadLogicalBytes")?,
        logical_bytes: required_u64(value, "logicalBytes")?,
        record_allocation_bytes: required_u64(value, "recordAllocationBytes")?,
    })
}

fn stage_shape(
    value: &Value,
    payload: u64,
    logical_record_bytes: u64,
    allocation_record_bytes: u64,
) -> Result<StageShape, CurrentFixturesError> {
    require_exact_keys(value, &["recordCount", "logicalBytes"])?;
    let record_count = required_u64(value, "recordCount")?;
    let logical_bytes = required_u64(value, "logicalBytes")?;
    compare_exact(
        "stage.logicalBytes",
        logical_bytes,
        record_count
            .checked_mul(logical_record_bytes)
            .and_then(|bytes| bytes.checked_add(payload))
            .ok_or_else(|| CurrentFixturesError::Contract("stage logical overflow".into()))?,
    )?;
    Ok(StageShape {
        record_count,
        payload_logical_bytes: payload,
        logical_bytes,
        record_allocation_bytes: record_count
            .checked_mul(allocation_record_bytes)
            .and_then(|bytes| bytes.checked_add(payload))
            .ok_or_else(|| CurrentFixturesError::Contract("stage allocation overflow".into()))?,
    })
}

fn validate_stage_formulas(
    stages: &StageBreakdown,
    counts: &IdentityAggregateCounts,
) -> Result<(), CurrentFixturesError> {
    compare_exact(
        "sourceDocumentCount",
        counts.module_count,
        counts.source_document_count,
    )?;
    compare_exact(
        "sourceDeclarationCount",
        counts.identity_declaration_count,
        counts.source_declaration_count,
    )?;
    compare_exact(
        "symbolCount",
        counts.source_declaration_count,
        counts.symbol_count,
    )?;
    let expected_source_spans = checked_sum(&[
        counts.source_declaration_count,
        counts.source_reference_count,
        counts.source_relation_count,
        counts.source_geometry_count,
    ])?;
    compare_exact(
        "sourceSpanCount",
        expected_source_spans,
        counts.source_span_count,
    )?;
    let expected_source_bytes = checked_sum(&[
        checked_mul_count(21, counts.source_declaration_count)?,
        checked_mul_count(19, counts.source_reference_count)?,
        checked_mul_count(18, counts.source_relation_count)?,
        checked_mul_count(18, counts.source_geometry_count)?,
    ])?;
    compare_exact(
        "sourceByteCount",
        expected_source_bytes,
        counts.source_byte_count,
    )?;
    let expected_string_items = checked_sum(&[
        counts.module_count,
        counts.source_document_count,
        counts.import_edge_count,
        counts.identity_declaration_count,
        counts.profiled_key_occurrence_count,
        counts.source_reference_count,
    ])?;
    compare_exact(
        "stringItemCount",
        expected_string_items,
        counts.string_item_count,
    )?;
    compare_exact(
        "sourceInput.recordCount",
        checked_sum(&[
            counts.module_count,
            counts.import_edge_count,
            counts.source_span_count,
        ])?,
        stages.source_input.record_count,
    )?;
    let typed_record_count = checked_sum(&[
        counts.module_count,
        counts.import_edge_count,
        counts.source_declaration_count,
        counts.identity_field_occurrence_count,
        counts.source_reference_count,
        counts.source_relation_count,
        counts.source_geometry_count,
    ])?;
    compare_exact(
        "typedAst.recordCount",
        typed_record_count,
        stages.typed_ast.record_count,
    )?;
    compare_exact(
        "typedAst.payloadLogicalBytes",
        checked_sum(&[
            counts.source_byte_count,
            counts.total_string_bytes,
            checked_mul_count(20, counts.source_span_count)?,
        ])?,
        stages.typed_ast.payload_logical_bytes,
    )?;
    validate_complete_stage_shape("typedAst", stages.typed_ast, 32, 32)?;
    let hir_record_count = checked_sum(&[
        counts.module_count,
        counts.import_edge_count,
        counts.symbol_count,
        counts.identity_field_occurrence_count,
        counts.source_reference_count,
        counts.source_relation_count,
        counts.source_geometry_count,
    ])?;
    compare_exact("hir.recordCount", hir_record_count, stages.hir.record_count)?;
    let hir_operands = checked_sum(&[
        counts.identity_field_occurrence_count,
        counts.import_edge_count,
        counts.source_reference_count,
        checked_mul_count(2, counts.source_relation_count)?,
        checked_mul_count(3, counts.source_geometry_count)?,
    ])?;
    compare_exact(
        "hir.payloadLogicalBytes",
        checked_sum(&[
            counts.total_string_bytes,
            checked_mul_count(4, hir_operands)?,
        ])?,
        stages.hir.payload_logical_bytes,
    )?;
    validate_complete_stage_shape("hir", stages.hir, 32, 32)?;
    compare_exact(
        "MIR record count",
        counts.semantic_output_record,
        stages.mir.record_count,
    )?;
    compare_exact(
        "MIR payload",
        counts.semantic_payload_byte_count,
        stages.mir.payload_logical_bytes,
    )?;
    validate_complete_stage_shape("mir", stages.mir, 44, 48)?;
    compare_exact(
        "logicalByteCount",
        counts.logical_byte_count,
        stages.mir.logical_bytes,
    )?;
    compare_exact(
        "canonicalLir.recordCount",
        stages.mir.record_count,
        stages.canonical_lir.record_count,
    )?;
    compare_exact(
        "canonicalLir.payloadLogicalBytes",
        stages.mir.payload_logical_bytes,
        stages.canonical_lir.payload_logical_bytes,
    )?;
    validate_complete_stage_shape("canonicalLir", stages.canonical_lir, 44, 48)?;
    compare_exact(
        "diagnostics.recordCount",
        counts.diagnostic_count,
        stages.diagnostics.record_count,
    )?;
    compare_exact("scratch.recordCount", 0, stages.scratch.record_count)?;
    compare_exact(
        "outputConstruction.recordCount",
        counts.semantic_output_record,
        stages.output_construction.record_count,
    )?;
    compare_exact(
        "outputConstruction formula",
        checked_sum(&[
            54,
            checked_mul_count(36, counts.semantic_output_record)?,
            counts.semantic_payload_byte_count,
        ])?,
        stages.output_construction.logical_bytes,
    )?;
    compare_exact(
        "outputByteCount",
        counts.output_byte_count,
        stages.output_construction.logical_bytes,
    )?;
    compare_exact(
        "scratch.logicalBytes",
        checked_mul_count(
            8,
            counts
                .module_count
                .max(counts.symbol_count)
                .max(counts.semantic_output_record),
        )?,
        stages.scratch.logical_bytes,
    )
}

fn validate_complete_stage_shape(
    name: &str,
    shape: StageShape,
    logical_record_bytes: u64,
    allocation_record_bytes: u64,
) -> Result<(), CurrentFixturesError> {
    compare_exact(
        &format!("{name}.logicalBytes"),
        checked_sum(&[
            checked_mul_count(logical_record_bytes, shape.record_count)?,
            shape.payload_logical_bytes,
        ])?,
        shape.logical_bytes,
    )?;
    compare_exact(
        &format!("{name}.recordAllocationBytes"),
        checked_sum(&[
            checked_mul_count(allocation_record_bytes, shape.record_count)?,
            shape.payload_logical_bytes,
        ])?,
        shape.record_allocation_bytes,
    )
}

fn checked_sum(values: &[u64]) -> Result<u64, CurrentFixturesError> {
    values.iter().try_fold(0_u64, |sum, value| {
        sum.checked_add(*value)
            .ok_or_else(|| CurrentFixturesError::Contract("stage formula sum overflow".into()))
    })
}

fn checked_mul_count(multiplier: u64, value: u64) -> Result<u64, CurrentFixturesError> {
    multiplier
        .checked_mul(value)
        .ok_or_else(|| CurrentFixturesError::Contract("stage formula product overflow".into()))
}

fn validate_projection_contract(value: &Value) -> Result<(), CurrentFixturesError> {
    require_exact_keys(
        value,
        &[
            "scope",
            "moduleOrder",
            "moduleImports",
            "namespaceDerivationGraphProfileId",
            "trafficEntitySources",
            "spatialEntitySources",
            "relationSources",
            "identityRule",
            "sourceReferenceRule",
            "sourceDocumentKeyFormula",
            "sourceRule",
            "scenarioManifestRule",
            "unlistedProductionSemantics",
        ],
    )?;
    require_string(
        value,
        "scope",
        "non-production research projection only; it does not define LF-COMP-CURRENT-EQUIV-v1 or a production frontend",
    )?;
    require_string_array(
        value,
        "moduleOrder",
        &[
            "traffic",
            "spatial when the selected case contains a SpatialPackage",
        ],
    )?;
    require_string(
        value,
        "moduleImports",
        "spatial imports traffic exactly once when present; otherwise no imports",
    )?;
    require_string(
        value,
        "namespaceDerivationGraphProfileId",
        NOT_APPLICABLE_GRAPH_PROFILE,
    )?;
    require_exact_string_object(
        value,
        "trafficEntitySources",
        &[
            ("RoadCorridor", "roadCorridors array order"),
            ("RoadSection", "roadSections array order"),
            (
                "AuthoringLane",
                "roadSections array order then lanes array order",
            ),
            ("LaneEdge", "laneGraph.edges array order"),
            ("Junction", "junctions array order"),
            ("Movement", "movements array order"),
            ("ManeuverPath", "maneuverPaths array order"),
            ("ManeuverGate", "signals.maneuverGates array order"),
            ("WaitingZone", "waitingZones array order"),
            ("StopLine", "signals.stopLines array order"),
            ("SignalGroup", "signals.groups array order"),
            ("SignalController", "signals.controllers array order"),
            (
                "SignalPhase",
                "signals.controllers array order then phases array order",
            ),
            ("ParkingArea", "parking.areas array order"),
            ("ParkingSpace", "parking.spaces array order"),
            ("LaneGroup", "laneGroups array order"),
            ("FacilityBand", "facilityBands array order"),
            ("ParticipantClass", "participantClasses array order"),
            ("AccessRule", "accessRules array order"),
            ("VehicleProfile", "vehicleProfiles array order"),
            ("StaticRoute", "routes array order"),
        ],
    )?;
    require_exact_string_object(
        value,
        "spatialEntitySources",
        &[
            ("CanonicalFrame", "one declaration from frameId"),
            (
                "canonicalGeometryPoint",
                "join edges by trafficEdgeId to the traffic LaneEdge declaration, then enumerate centerline.points in traffic LaneEdge declaration order and point array order",
            ),
            (
                "crossModuleReference",
                "one spatial-to-traffic LaneEdge binding per joined spatial edge",
            ),
        ],
    )?;
    require_exact_string_object(
        value,
        "relationSources",
        &[
            (
                "owner-relation",
                "AuthoringLane nesting; Movement.junctionId; ManeuverPath.movementId; ManeuverGate.maneuverPathId; WaitingZone.maneuverPathId; SignalPhase controller nesting; ParkingSpace.areaId; LaneGroup.roadSectionId; RoadCorridor.elements ownership of RoadSection or FacilityBand",
            ),
            (
                "edge-connection",
                "laneGraph.edges array order then connections array order",
            ),
            (
                "route-occurrence",
                "routes array order then edgeIds array order",
            ),
            (
                "access-relation",
                "accessRules array order then participantClassIds array order",
            ),
            (
                "signal-group-relation",
                "signals.maneuverGates array order where signalControl.kind is group",
            ),
            (
                "signal-phase-state",
                "signals.controllers, phases, then states array order",
            ),
            ("gate-occurrence", "signals.maneuverGates array order"),
            ("waiting-zone-occurrence", "waitingZones array order"),
            ("parking-space-anchors", "parking.spaces array order"),
            (
                "lane-coverage-occurrence",
                "roadSections, lanes, then edgeIds array order",
            ),
            (
                "junction-internal-edge-role",
                "maneuverPaths array order then internalEdgeIds array order",
            ),
        ],
    )?;
    let identity = required_object(value, "identityRule")?;
    require_exact_keys(
        identity,
        &[
            "originalIdUse",
            "profiledKeyFormula",
            "profiledKeyLengthBytes",
            "identityFields",
        ],
    )?;
    require_string(
        identity,
        "originalIdUse",
        "resolve current fixture references outside the measured region, then discard original strings",
    )?;
    require_string(
        identity,
        "profiledKeyFormula",
        "fixture/{entityKindCodeHex2}/{sourceArrayOrdinalHex8}",
    )?;
    require_u64(identity, "profiledKeyLengthBytes", 19)?;
    require_string(
        identity,
        "identityFields",
        "apply identityBindings in ascending field tag order",
    )?;
    let reference = required_object(value, "sourceReferenceRule")?;
    require_exact_keys(reference, &["spellingFormula", "byteLength", "countRule"])?;
    require_string(
        reference,
        "spellingFormula",
        "reference/{targetKindCodeHex2}/{targetModuleOrdinalHex8}/{targetLocalOrdinalHex8}",
    )?;
    require_u64(reference, "byteLength", 30)?;
    require_string(
        reference,
        "countRule",
        "identity stable-id fields plus every StableId payload field of the projected relation and geometry records plus spatial-to-traffic cross-module bindings",
    )?;
    require_string(
        value,
        "sourceDocumentKeyFormula",
        "fixture/{caseId}/{canonicalModuleName}.lfsynthetic",
    )?;
    require_string(
        value,
        "sourceRule",
        "apply sourceSpanRule token ordering and byte lengths, substitute projectionContract.sourceDocumentKeyFormula, and apply researchStageModel to the exact case projection without N scaling",
    )?;
    require_string(
        value,
        "scenarioManifestRule",
        "bind file identity and source lineage only; emit no module, declaration, reference, relation, geometry, semantic, or stage record",
    )?;
    require_string(
        value,
        "unlistedProductionSemantics",
        "not-compared and forbidden from implicit projection",
    )
}

fn validate_scenario_lineage(
    manifest: &Value,
    files: &[BoundFixtureFile],
) -> Result<(), CurrentFixturesError> {
    for (role, suffix, media_type) in [
        (
            "traffic",
            ".laneflow.json",
            "application/vnd.laneflow.traffic+json",
        ),
        (
            "spatial",
            ".spatial.json",
            "application/vnd.laneflow.spatial+json",
        ),
    ] {
        let binding = files
            .iter()
            .find(|file| file.path.ends_with(suffix))
            .ok_or_else(|| CurrentFixturesError::Missing(format!("{role} fixture binding")))?;
        let descriptor = required_object(manifest, role)?;
        let name = Path::new(&binding.path)
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| CurrentFixturesError::Contract("fixture filename".into()))?;
        require_string(descriptor, "artifactRef", name)?;
        require_string(descriptor, "mediaType", media_type)?;
        require_string(descriptor, "digest", &format!("sha256:{}", binding.sha256))?;
        require_u64(descriptor, "size", binding.byte_length)?;
    }
    Ok(())
}

fn read_bound_file(
    repository_root: &Path,
    binding: &BoundFixtureFile,
) -> Result<Vec<u8>, CurrentFixturesError> {
    let relative = Path::new(&binding.path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(CurrentFixturesError::UnsafePath(binding.path.clone()));
    }
    let bytes = fs::read(repository_root.join(relative)).map_err(|source| {
        CurrentFixturesError::ReadArtifact {
            path: binding.path.clone(),
            source,
        }
    })?;
    compare_exact(
        "fixture byteLength",
        binding.byte_length,
        bytes.len() as u64,
    )?;
    let digest = lower_hex(&Sha256::digest(&bytes));
    if digest != binding.sha256 {
        return Err(CurrentFixturesError::Mismatch {
            path: binding.path.clone(),
            expected: binding.sha256.clone(),
            actual: digest,
        });
    }
    Ok(bytes)
}

fn count_record_kinds(records: &[SemanticRecord]) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    for record in records {
        let name = match record.record_kind {
            1 => "identity-declaration",
            2 => "owner-relation",
            3 => "edge-connection",
            4 => "route-occurrence",
            5 => "canonical-geometry-point",
            6 => "access-relation",
            7 => "signal-group-relation",
            8 => "signal-phase-state",
            9 => "gate-occurrence",
            10 => "waiting-zone-occurrence",
            11 => "parking-space-anchors",
            12 => "lane-coverage-occurrence",
            13 => "junction-internal-edge-role",
            _ => "unknown",
        };
        *counts.entry(name.to_owned()).or_default() += 1;
    }
    counts
}

fn encode_source_spans(payload: &mut Vec<u8>, spans: &[SourceSpanRecord]) {
    for span in spans {
        payload.extend_from_slice(&span.source_document_ordinal.to_le_bytes());
        payload.extend_from_slice(&span.start_line.to_le_bytes());
        payload.extend_from_slice(&span.start_column.to_le_bytes());
        payload.extend_from_slice(&span.end_line.to_le_bytes());
        payload.extend_from_slice(&span.end_column.to_le_bytes());
    }
}

fn parse_u64_object(value: &Value) -> Result<BTreeMap<String, u64>, CurrentFixturesError> {
    value
        .as_object()
        .ok_or_else(|| CurrentFixturesError::Missing("count object".into()))?
        .iter()
        .map(|(field, value)| {
            Ok((
                field.clone(),
                value
                    .as_u64()
                    .ok_or_else(|| CurrentFixturesError::Missing(field.clone()))?,
            ))
        })
        .collect()
}

fn compare_exact(path: &str, expected: u64, actual: u64) -> Result<(), CurrentFixturesError> {
    if expected != actual {
        return Err(CurrentFixturesError::Mismatch {
            path: path.to_owned(),
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
    }
    Ok(())
}

fn count(counts: &BTreeMap<String, u64>, field: &str) -> Result<u64, CurrentFixturesError> {
    counts
        .get(field)
        .copied()
        .ok_or_else(|| CurrentFixturesError::Missing(field.to_owned()))
}

fn u32_count<T>(value: T, field: &str) -> Result<u32, CurrentFixturesError>
where
    u32: TryFrom<T>,
{
    u32::try_from(value)
        .map_err(|_| CurrentFixturesError::Contract(format!("{field} does not fit u32")))
}

fn usize_count(value: u64, field: &str) -> Result<usize, CurrentFixturesError> {
    usize::try_from(value)
        .map_err(|_| CurrentFixturesError::Contract(format!("{field} does not fit usize")))
}

fn put_string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
    bytes.extend_from_slice(value.as_bytes());
}

fn lower_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

#[cfg(feature = "fixture-oracle")]
fn describe_template_mismatch(
    producer: &CorridorTemplate,
    independent: &CorridorTemplate,
) -> String {
    if producer.entities != independent.entities {
        let first = producer
            .entities
            .iter()
            .zip(&independent.entities)
            .position(|(left, right)| left != right);
        return format!(
            "entities producer={} independent={} firstMismatch={first:?}",
            producer.entities.len(),
            independent.entities.len()
        );
    }
    if producer.relations != independent.relations {
        let first = producer
            .relations
            .iter()
            .zip(&independent.relations)
            .position(|(left, right)| left != right);
        return format!(
            "relations producer={} independent={} firstMismatch={first:?}",
            producer.relations.len(),
            independent.relations.len()
        );
    }
    let first = producer
        .geometry
        .iter()
        .zip(&independent.geometry)
        .position(|(left, right)| left != right);
    format!(
        "geometry producer={} independent={} firstMismatch={first:?}",
        producer.geometry.len(),
        independent.geometry.len()
    )
}

fn required_array<'a>(value: &'a Value, field: &str) -> Result<&'a [Value], CurrentFixturesError> {
    value
        .get(field)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| CurrentFixturesError::Missing(field.to_owned()))
}

fn required_object<'a>(value: &'a Value, field: &str) -> Result<&'a Value, CurrentFixturesError> {
    value
        .get(field)
        .filter(|item| item.is_object())
        .ok_or_else(|| CurrentFixturesError::Missing(field.to_owned()))
}

fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, CurrentFixturesError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| CurrentFixturesError::Missing(field.to_owned()))
}

fn required_u64(value: &Value, field: &str) -> Result<u64, CurrentFixturesError> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| CurrentFixturesError::Missing(field.to_owned()))
}

fn require_string(value: &Value, field: &str, expected: &str) -> Result<(), CurrentFixturesError> {
    let actual = required_string(value, field)?;
    if actual != expected {
        return Err(CurrentFixturesError::Mismatch {
            path: field.to_owned(),
            expected: expected.to_owned(),
            actual: actual.to_owned(),
        });
    }
    Ok(())
}

fn require_u64(value: &Value, field: &str, expected: u64) -> Result<(), CurrentFixturesError> {
    compare_exact(field, expected, required_u64(value, field)?)
}

fn require_bool(value: &Value, field: &str, expected: bool) -> Result<(), CurrentFixturesError> {
    let actual = value
        .get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| CurrentFixturesError::Missing(field.to_owned()))?;
    if actual != expected {
        return Err(CurrentFixturesError::Mismatch {
            path: field.to_owned(),
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
    }
    Ok(())
}

fn require_string_array(
    value: &Value,
    field: &str,
    expected: &[&str],
) -> Result<(), CurrentFixturesError> {
    let actual = required_array(value, field)?
        .iter()
        .map(|item| item.as_str())
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| CurrentFixturesError::Missing(field.to_owned()))?;
    if actual != expected {
        return Err(CurrentFixturesError::Mismatch {
            path: field.to_owned(),
            expected: expected.join(", "),
            actual: actual.join(", "),
        });
    }
    Ok(())
}

fn require_exact_keys(value: &Value, expected: &[&str]) -> Result<(), CurrentFixturesError> {
    let object = value
        .as_object()
        .ok_or_else(|| CurrentFixturesError::Missing("object".into()))?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(CurrentFixturesError::Mismatch {
            path: "object keys".to_owned(),
            expected: format!("{expected:?}"),
            actual: format!("{actual:?}"),
        });
    }
    Ok(())
}

fn require_exact_string_object(
    parent: &Value,
    field: &str,
    expected: &[(&str, &str)],
) -> Result<(), CurrentFixturesError> {
    let value = required_object(parent, field)?;
    require_exact_keys(
        value,
        &expected.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
    )?;
    for (key, expected_value) in expected {
        require_string(value, key, expected_value)?;
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum CurrentFixturesError {
    #[error(transparent)]
    Corridor(#[from] CorridorError),
    #[error("当前固定样例契约缺少 `{0}`")]
    Missing(String),
    #[error("当前固定样例契约不一致：{path} 应为 {expected}，实际为 {actual}")]
    Mismatch {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("当前固定样例契约错误：{0}")]
    Contract(String),
    #[error("当前固定样例重复制品或记录：{0}")]
    DuplicateArtifact(String),
    #[error("当前固定样例包含不安全路径：{0}")]
    UnsafePath(String),
    #[error("无法读取当前固定样例 {path}: {source}")]
    ReadArtifact {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("当前固定样例 {path} 不是有效 JSON: {source}")]
    InvalidJson {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum CurrentFixturesOracleError {
    #[error(transparent)]
    CurrentFixtures(#[from] CurrentFixturesError),
    #[error(transparent)]
    GeneratorContract(#[from] crate::ManifestContractError),
    #[error(transparent)]
    IdentityContract(#[from] crate::IdentityContractError),
    #[error(transparent)]
    StageContract(#[from] crate::stage::StageContractError),
    #[error(transparent)]
    CorridorOracle(#[from] crate::corridor_oracle::CorridorOracleError),
    #[error("当前生产加载器拒绝固定样例：{0}")]
    ProductionLoader(String),
    #[error("固定样例 {case_id} 的原始 JSON 投影与生产加载器投影不一致：{details}")]
    TemplateMismatch { case_id: String, details: String },
    #[error("固定样例 {0} 的生产者与独立身份/记录流预言机不一致")]
    RecordStreamMismatch(String),
    #[error("固定样例独立预言机契约错误：{0}")]
    Contract(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn all_three_cases_match_frozen_projection_and_stage_shapes() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let summaries = build_current_fixture_summaries(&trusted).expect("fixture summaries");
        assert_eq!(summaries.len(), 3);
        assert_eq!(summaries[0].counts.semantic_output_record, 2_319);
        assert_eq!(summaries[1].counts.semantic_output_record, 57);
        assert_eq!(summaries[2].counts.semantic_output_record, 39);
    }

    #[test]
    fn current_fixture_summary_known_vector_has_exact_frozen_bytes() {
        let path = crate::repository_root().join(
            "research/issue-308-compiler-budget-calibration-research/known-vectors/current-fixtures-summary-v1.json",
        );
        let bytes = std::fs::read(path).expect("current fixture summary known vector");
        assert_eq!(bytes.len(), CURRENT_FIXTURES_KNOWN_VECTOR_BYTE_LENGTH);
        assert_eq!(
            lower_hex(&Sha256::digest(&bytes)),
            CURRENT_FIXTURES_KNOWN_VECTOR_SHA256
        );
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let generated =
            serde_json::to_string_pretty(&build_current_fixtures_known_vectors(&trusted).unwrap())
                .expect("serialize current fixture vectors")
                + "\n";
        assert_eq!(generated.as_bytes(), bytes);
    }

    #[test]
    fn rejects_fixed_case_aggregate_constants_that_violate_stage_formulas() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let mut manifest = trusted.workload_manifest;
        let workload = manifest
            .get_mut("workloads")
            .and_then(Value::as_array_mut)
            .expect("workloads")
            .iter_mut()
            .find(|workload| {
                workload.get("id").and_then(Value::as_str) == Some(CURRENT_FIXTURES_WORKLOAD_ID)
            })
            .expect("current fixtures workload");
        let parking = workload
            .get_mut("cases")
            .and_then(Value::as_array_mut)
            .expect("cases")
            .iter_mut()
            .find(|case| case.get("id").and_then(Value::as_str) == Some("parking-signals-baseline"))
            .expect("parking case");
        parking["projection"]["exactAggregateInputs"]["sourceSpanCount"] = Value::from(116);

        assert!(matches!(
            CurrentFixturesContract::from_manifest(&manifest),
            Err(CurrentFixturesError::Mismatch { path, .. }) if path == "sourceSpanCount"
        ));
    }

    #[test]
    fn rejects_same_length_fixture_byte_drift_before_projection() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let contract =
            CurrentFixturesContract::from_manifest(&trusted.workload_manifest).expect("contract");
        let binding = &contract.cases()[1].files[0];
        let ordinal = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let temporary_root = std::env::temp_dir().join(format!(
            "laneflow-current-fixture-drift-{}-{ordinal}",
            std::process::id()
        ));
        let target = temporary_root.join(&binding.path);
        std::fs::create_dir_all(target.parent().expect("fixture parent")).expect("create parent");
        let mut bytes =
            std::fs::read(crate::repository_root().join(&binding.path)).expect("fixture bytes");
        bytes[0] ^= 1;
        std::fs::write(&target, bytes).expect("write changed fixture");

        assert!(matches!(
            read_bound_file(&temporary_root, binding),
            Err(CurrentFixturesError::Mismatch { path, .. }) if path == binding.path
        ));
        std::fs::remove_dir_all(temporary_root).expect("remove temporary fixture root");
    }

    #[test]
    fn rejects_scenario_manifest_lineage_digest_drift() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let contract =
            CurrentFixturesContract::from_manifest(&trusted.workload_manifest).expect("contract");
        let case = &contract.cases()[0];
        let scenario = case
            .files
            .iter()
            .find(|binding| binding.path.ends_with(".scenario.json"))
            .expect("scenario binding");
        let mut value: Value = serde_json::from_slice(
            &std::fs::read(crate::repository_root().join(&scenario.path)).expect("scenario bytes"),
        )
        .expect("scenario JSON");
        value["traffic"]["digest"] = Value::from(format!("sha256:{}", "0".repeat(64)));

        assert!(matches!(
            validate_scenario_lineage(&value, &case.files),
            Err(CurrentFixturesError::Mismatch { path, .. }) if path == "digest"
        ));
    }

    #[cfg(feature = "fixture-oracle")]
    #[test]
    fn all_three_cases_match_production_loaders_and_independent_record_oracle() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let report =
            verify_current_fixtures_oracle(&trusted).expect("current fixture oracle matrix");
        assert_eq!(report.checked_cases, 3);
        assert_eq!(report.production_loader_cases, 3);
        assert!(report.independent_identity_and_stream_checked);
        assert!(report.scenario_manifest_emits_no_records);
        assert!(report.excluded_from_budget_and_candidate_ranking);
    }
}
