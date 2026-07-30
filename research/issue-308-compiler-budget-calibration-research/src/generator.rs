use crate::GeneratorContract;
use serde::Serialize;
use std::collections::BTreeMap;

pub const IDENTITY_WORKLOAD_ID: &str = "LF-COMP-ID-v1";
pub const MODULE_GRAPH_KNOWN_VECTOR_SCHEMA: &str =
    "laneflow.compiler-calibration-module-graph-known-vectors";
pub const MODULE_GRAPH_KNOWN_VECTOR_ORDER: &str = "canonical-module-name-utf8-ascending";
#[cfg(test)]
const MODULE_GRAPH_KNOWN_VECTOR_BYTE_LENGTH: usize = 6_545;
#[cfg(test)]
const MODULE_GRAPH_KNOWN_VECTOR_SHA256: &str =
    "abe175a0982c6483619fb65738011c97e7871faf247531f4a46cffb136da41f5";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum GraphProfileId {
    #[serde(rename = "wide-star-v1")]
    WideStar,
    #[serde(rename = "deep-chain-v1")]
    DeepChain,
    #[serde(rename = "shared-fanin-dag-v1")]
    SharedFaninDag,
}

impl GraphProfileId {
    pub const ALL: [Self; 3] = [Self::WideStar, Self::DeepChain, Self::SharedFaninDag];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WideStar => "wide-star-v1",
            Self::DeepChain => "deep-chain-v1",
            Self::SharedFaninDag => "shared-fanin-dag-v1",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SequenceKind {
    Imports,
    Declarations,
    References,
    Relations,
    Geometry,
}

impl SequenceKind {
    fn code(self, contract: &GeneratorContract) -> u8 {
        match self {
            Self::Imports => contract.imports_sequence_kind,
            Self::Declarations => contract.declarations_sequence_kind,
            Self::References => contract.references_sequence_kind,
            Self::Relations => contract.relations_sequence_kind,
            Self::Geometry => contract.geometry_sequence_kind,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpandedModule {
    pub canonical_name: String,
    pub module_seed_ordinal_hex_u64: String,
    pub namespace_id: String,
    pub imports: Vec<String>,
    pub cross_module_references: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpandedModuleGraph {
    pub graph_profile: GraphProfileId,
    pub n: u32,
    pub modules: Vec<ExpandedModule>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleGraphKnownVectorDocument {
    pub schema: &'static str,
    pub schema_version: u32,
    pub source_workload_manifest_sha256: String,
    pub workload_id: &'static str,
    pub generator_version: u32,
    pub base_seed_hex_u64: String,
    pub module_order: &'static str,
    pub vectors: Vec<ExpandedModuleGraph>,
}

pub fn build_module_graph_known_vectors(
    contract: &GeneratorContract,
    workload_manifest_sha256: &str,
) -> Result<ModuleGraphKnownVectorDocument, GeneratorError> {
    let mut vectors = Vec::with_capacity(GraphProfileId::ALL.len() * 2);
    for graph_profile in GraphProfileId::ALL {
        for n in [1, 2] {
            vectors.push(expand_module_graph(
                contract,
                IDENTITY_WORKLOAD_ID,
                graph_profile,
                n,
            )?);
        }
    }

    Ok(ModuleGraphKnownVectorDocument {
        schema: MODULE_GRAPH_KNOWN_VECTOR_SCHEMA,
        schema_version: 1,
        source_workload_manifest_sha256: workload_manifest_sha256.to_owned(),
        workload_id: IDENTITY_WORKLOAD_ID,
        generator_version: contract.generator_version,
        base_seed_hex_u64: format!("{:016x}", contract.base_seed),
        module_order: MODULE_GRAPH_KNOWN_VECTOR_ORDER,
        vectors,
    })
}

pub fn expand_module_graph(
    contract: &GeneratorContract,
    workload_id: &str,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<ExpandedModuleGraph, GeneratorError> {
    if n == 0 {
        return Err(GeneratorError::ScaleMustBePositive);
    }
    let mut modules = BTreeMap::<String, ModuleBuilder>::new();
    insert_module(&mut modules, "root".to_owned(), 0);

    match graph_profile {
        GraphProfileId::WideStar => {
            for unit_index in 0..n {
                let unit = unit_name(unit_index);
                insert_module(&mut modules, unit.clone(), unit_seed_ordinal(unit_index));
                modules
                    .get_mut("root")
                    .expect("root module")
                    .imports
                    .push(unit.clone());
                modules
                    .get_mut("root")
                    .expect("root module")
                    .cross_module_references
                    .push(format!("canonical-first-declaration({unit})"));
            }
        }
        GraphProfileId::DeepChain => {
            for unit_index in 0..n {
                let unit = unit_name(unit_index);
                insert_module(&mut modules, unit, unit_seed_ordinal(unit_index));
            }
            modules
                .get_mut("root")
                .expect("root module")
                .imports
                .push(unit_name(0));
            for unit_index in 0..n.saturating_sub(1) {
                let unit = unit_name(unit_index);
                let next = unit_name(unit_index + 1);
                let module = modules.get_mut(&unit).expect("inserted unit module");
                module.imports.push(next.clone());
                module
                    .cross_module_references
                    .push(format!("canonical-first-declaration({next})"));
            }
        }
        GraphProfileId::SharedFaninDag => {
            insert_module(&mut modules, "shared/common".to_owned(), 1);
            let group_count = n.div_ceil(contract.shared_fanin_group_width);
            for group_index in 0..group_count {
                let group = group_name(group_index);
                insert_module(&mut modules, group.clone(), group_seed_ordinal(group_index));
                modules
                    .get_mut("root")
                    .expect("root module")
                    .imports
                    .push(group);
            }
            for unit_index in 0..n {
                let unit = unit_name(unit_index);
                insert_module(&mut modules, unit.clone(), unit_seed_ordinal(unit_index));
                let group = group_name(unit_index / contract.shared_fanin_group_width);
                modules
                    .get_mut(&group)
                    .expect("inserted group module")
                    .imports
                    .push(unit.clone());
                let unit_module = modules.get_mut(&unit).expect("inserted unit module");
                unit_module.imports.push("shared/common".to_owned());
                unit_module
                    .cross_module_references
                    .push("shared/common::shared-calibration-anchor".to_owned());
            }
        }
    }

    let modules = modules
        .into_values()
        .map(|mut module| {
            permute_in_place(
                &mut module.imports,
                contract,
                SequenceKind::Imports,
                module.seed_ordinal,
            );
            permute_in_place(
                &mut module.cross_module_references,
                contract,
                SequenceKind::References,
                module.seed_ordinal,
            );
            ExpandedModule {
                namespace_id: derive_namespace_id(
                    contract,
                    workload_id,
                    graph_profile.as_str(),
                    &module.canonical_name,
                ),
                canonical_name: module.canonical_name,
                module_seed_ordinal_hex_u64: format!("{:016x}", module.seed_ordinal),
                imports: module.imports,
                cross_module_references: module.cross_module_references,
            }
        })
        .collect();

    Ok(ExpandedModuleGraph {
        graph_profile,
        n,
        modules,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GeneratorError {
    #[error("研究工作负载规模 N 必须至少为 1")]
    ScaleMustBePositive,
}

pub fn permute_in_place<T>(
    values: &mut [T],
    contract: &GeneratorContract,
    sequence_kind: SequenceKind,
    module_seed_ordinal: u64,
) {
    let seed =
        contract.base_seed ^ (u64::from(sequence_kind.code(contract)) << 56) ^ module_seed_ordinal;
    let mut random = SplitMix64::new(seed, contract);
    for index in (1..values.len()).rev() {
        let modulus = u64::try_from(index + 1).expect("sequence length must fit u64");
        let swap_index =
            usize::try_from(random.next_u64() % modulus).expect("swap index must fit usize");
        values.swap(index, swap_index);
    }
}

fn derive_namespace_id(
    contract: &GeneratorContract,
    workload_id: &str,
    graph_profile_id: &str,
    canonical_module_name: &str,
) -> String {
    let preimage = namespace_preimage(
        contract,
        workload_id,
        graph_profile_id,
        canonical_module_name,
    );
    let digest = blake3::hash(&preimage);
    let selected = &digest.as_bytes()[contract.namespace_digest_offset
        ..contract.namespace_digest_offset + contract.namespace_digest_length];
    encode_lower_hex(selected)
}

fn namespace_preimage(
    contract: &GeneratorContract,
    workload_id: &str,
    graph_profile_id: &str,
    canonical_module_name: &str,
) -> Vec<u8> {
    let mut preimage = Vec::with_capacity(
        contract.namespace_domain.len()
            + 1
            + 4
            + 8
            + 4
            + workload_id.len()
            + 4
            + graph_profile_id.len()
            + 4
            + canonical_module_name.len(),
    );
    preimage.extend_from_slice(contract.namespace_domain.as_bytes());
    preimage.push(0);
    preimage.extend_from_slice(&contract.generator_version.to_le_bytes());
    preimage.extend_from_slice(&contract.base_seed.to_le_bytes());
    append_length_prefixed(&mut preimage, workload_id);
    append_length_prefixed(&mut preimage, graph_profile_id);
    append_length_prefixed(&mut preimage, canonical_module_name);
    preimage
}

fn append_length_prefixed(output: &mut Vec<u8>, value: &str) {
    let length = u32::try_from(value.len()).expect("research identifier must fit u32");
    output.extend_from_slice(&length.to_le_bytes());
    output.extend_from_slice(value.as_bytes());
}

fn encode_lower_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn insert_module(
    modules: &mut BTreeMap<String, ModuleBuilder>,
    canonical_name: String,
    seed_ordinal: u64,
) {
    let previous = modules.insert(
        canonical_name.clone(),
        ModuleBuilder {
            canonical_name,
            seed_ordinal,
            imports: Vec::new(),
            cross_module_references: Vec::new(),
        },
    );
    assert!(previous.is_none(), "canonical module names must be unique");
}

fn unit_name(index: u32) -> String {
    format!("unit/{index:08x}")
}

fn group_name(index: u32) -> String {
    format!("group/{index:08x}")
}

fn unit_seed_ordinal(index: u32) -> u64 {
    (2_u64 << 40) | u64::from(index)
}

fn group_seed_ordinal(index: u32) -> u64 {
    (1_u64 << 40) | u64::from(index)
}

#[derive(Clone, Debug)]
struct ModuleBuilder {
    canonical_name: String,
    seed_ordinal: u64,
    imports: Vec<String>,
    cross_module_references: Vec<String>,
}

struct SplitMix64 {
    state: u64,
    increment: u64,
    multiplier_1: u64,
    multiplier_2: u64,
}

impl SplitMix64 {
    fn new(seed: u64, contract: &GeneratorContract) -> Self {
        Self {
            state: seed,
            increment: contract.splitmix64_increment,
            multiplier_1: contract.splitmix64_multiplier_1,
            multiplier_2: contract.splitmix64_multiplier_2,
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(self.increment);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(self.multiplier_1);
        value = (value ^ (value >> 27)).wrapping_mul(self.multiplier_2);
        value ^ (value >> 31)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_repository_contract;

    fn generator_contract() -> GeneratorContract {
        load_repository_contract()
            .expect("frozen contract")
            .generator_contract()
            .expect("generator contract")
    }

    #[test]
    fn splitmix64_matches_public_reference_vector_for_zero_seed() {
        let contract = generator_contract();
        let mut random = SplitMix64::new(0, &contract);

        assert_eq!(random.next_u64(), 0xe220_a839_7b1d_cdaf);
        assert_eq!(random.next_u64(), 0x6e78_9e6a_a1b9_65f4);
        assert_eq!(random.next_u64(), 0x06c4_5d18_8009_454f);
    }

    #[test]
    fn permutation_is_sequence_and_module_scoped() {
        let contract = generator_contract();
        let mut imports = (0_u32..8).collect::<Vec<_>>();
        let mut imports_repeated = imports.clone();
        let mut references = imports.clone();

        permute_in_place(&mut imports, &contract, SequenceKind::Imports, 0);
        permute_in_place(&mut imports_repeated, &contract, SequenceKind::Imports, 0);
        permute_in_place(&mut references, &contract, SequenceKind::References, 0);

        assert_eq!(imports, imports_repeated);
        assert_ne!(imports, references);
        assert_eq!(imports, [5, 0, 2, 6, 3, 1, 4, 7]);
        assert_eq!(references, [6, 5, 7, 4, 0, 3, 1, 2]);
    }

    #[test]
    fn namespace_preimage_and_digest_match_the_published_root_vector() {
        let contract = generator_contract();
        let preimage = namespace_preimage(
            &contract,
            IDENTITY_WORKLOAD_ID,
            GraphProfileId::WideStar.as_str(),
            "root",
        );

        assert_eq!(
            encode_lower_hex(&preimage),
            "4c462d434f4d502d4e414d4553504143452d763100010000000100504d4f43464c0d0000004c462d434f4d502d49442d76310c000000776964652d737461722d763104000000726f6f74"
        );
        assert_eq!(
            derive_namespace_id(
                &contract,
                IDENTITY_WORKLOAD_ID,
                GraphProfileId::WideStar.as_str(),
                "root",
            ),
            "f8edb568537afdedbeb87ab34441b009"
        );
    }

    #[test]
    fn expanding_n_does_not_change_existing_module_identity() {
        let contract = generator_contract();
        for graph_profile in GraphProfileId::ALL {
            let n1 = expand_module_graph(&contract, IDENTITY_WORKLOAD_ID, graph_profile, 1)
                .expect("N=1 graph");
            let n2 = expand_module_graph(&contract, IDENTITY_WORKLOAD_ID, graph_profile, 2)
                .expect("N=2 graph");
            for module in &n1.modules {
                let expanded = n2
                    .modules
                    .iter()
                    .find(|candidate| candidate.canonical_name == module.canonical_name)
                    .expect("existing module must remain");
                assert_eq!(expanded.namespace_id, module.namespace_id);
                assert_eq!(
                    expanded.module_seed_ordinal_hex_u64,
                    module.module_seed_ordinal_hex_u64
                );
            }
        }
    }

    #[test]
    fn expanded_graph_counts_match_frozen_formulas() {
        let contract = generator_contract();
        for n in [1_u32, 2, 65] {
            let wide =
                expand_module_graph(&contract, IDENTITY_WORKLOAD_ID, GraphProfileId::WideStar, n)
                    .expect("wide graph");
            assert_eq!(wide.modules.len(), usize::try_from(n + 1).unwrap());
            assert_eq!(count_imports(&wide), usize::try_from(n).unwrap());
            assert_eq!(count_references(&wide), usize::try_from(n).unwrap());

            let deep = expand_module_graph(
                &contract,
                IDENTITY_WORKLOAD_ID,
                GraphProfileId::DeepChain,
                n,
            )
            .expect("deep graph");
            assert_eq!(deep.modules.len(), usize::try_from(n + 1).unwrap());
            assert_eq!(count_imports(&deep), usize::try_from(n).unwrap());
            assert_eq!(
                count_references(&deep),
                usize::try_from(n.saturating_sub(1)).unwrap()
            );

            let groups = n.div_ceil(contract.shared_fanin_group_width);
            let shared = expand_module_graph(
                &contract,
                IDENTITY_WORKLOAD_ID,
                GraphProfileId::SharedFaninDag,
                n,
            )
            .expect("shared graph");
            assert_eq!(
                shared.modules.len(),
                usize::try_from(n + groups + 2).unwrap()
            );
            assert_eq!(
                count_imports(&shared),
                usize::try_from(2 * n + groups).unwrap()
            );
            assert_eq!(count_references(&shared), usize::try_from(n).unwrap());
        }
    }

    #[test]
    fn zero_scale_is_rejected_without_allocating_a_graph() {
        let contract = generator_contract();

        assert_eq!(
            expand_module_graph(&contract, IDENTITY_WORKLOAD_ID, GraphProfileId::WideStar, 0,),
            Err(GeneratorError::ScaleMustBePositive)
        );
    }

    #[test]
    fn published_n1_n2_vectors_equal_fresh_generation() {
        let trusted = load_repository_contract().expect("frozen contract");
        let contract = trusted.generator_contract().expect("generator contract");
        let generated = build_module_graph_known_vectors(
            &contract,
            &trusted.descriptor.workload_manifest.sha256,
        )
        .expect("known vectors");
        let rendered = format!(
            "{}\n",
            serde_json::to_string_pretty(&generated).expect("serialize generated vectors")
        );

        assert_eq!(
            include_str!("../known-vectors/module-graphs-v1.json"),
            rendered
        );
    }

    #[test]
    fn published_vector_bytes_match_the_readme_binding() {
        use sha2::{Digest, Sha256};

        let bytes = include_bytes!("../known-vectors/module-graphs-v1.json");
        let digest = Sha256::digest(bytes);

        assert_eq!(bytes.len(), MODULE_GRAPH_KNOWN_VECTOR_BYTE_LENGTH);
        assert_eq!(encode_lower_hex(&digest), MODULE_GRAPH_KNOWN_VECTOR_SHA256);
    }

    fn count_imports(graph: &ExpandedModuleGraph) -> usize {
        graph
            .modules
            .iter()
            .map(|module| module.imports.len())
            .sum()
    }

    fn count_references(graph: &ExpandedModuleGraph) -> usize {
        graph
            .modules
            .iter()
            .map(|module| module.cross_module_references.len())
            .sum()
    }
}
