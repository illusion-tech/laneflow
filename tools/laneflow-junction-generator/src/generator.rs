//! 复杂路口场景编排：拓扑编制 → catalog → 编译 → LFCA 发射。

use laneflow_compiler::{PortableDiffBase, PortableEmissionProvenance, emit_portable_candidate};
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_scenario::complex_junction::MIN_SPAWN_SLOT_COUNT;

use crate::Error;
use crate::catalog::{build_catalog, validate_catalog};
use crate::compile::{compile_junction, emit_lfca};
use crate::config::JunctionConfig;
use crate::topology::{COMPILER_BUILD_ID, build_topology};

pub struct GeneratedScenario {
    catalog: Vec<u8>,
    lfca: Vec<u8>,
    counts: ScenarioCounts,
    compilation: laneflow_compiler::CompilationOutput,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScenarioCounts {
    pub edges: usize,
    pub movements: usize,
    pub maneuver_paths: usize,
    pub maneuver_gates: usize,
    pub stop_lines: usize,
    pub waiting_zones: usize,
    pub conflict_zones: usize,
    pub streams: usize,
    pub signal_groups: usize,
    pub controllers: usize,
    pub phases: usize,
    pub routes: usize,
    pub portals: usize,
    pub spawn_slots: usize,
}

impl GeneratedScenario {
    pub fn catalog_bytes(&self) -> &[u8] {
        &self.catalog
    }

    pub fn lfca_bytes(&self) -> &[u8] {
        &self.lfca
    }

    pub fn emit_portable_sidecars(&self) -> Result<(Vec<u8>, Vec<u8>), Error> {
        let provenance =
            PortableEmissionProvenance::try_new(COMPILER_BUILD_ID).map_err(|error| {
                Error::Validation {
                    stage: "portable provenance",
                    message: format!("{error:?}"),
                }
            })?;
        let candidate = emit_portable_candidate(
            &self.compilation,
            &provenance,
            FormatLimits::HARD,
            PortableDiffBase::Genesis,
        )
        .map_err(|error| Error::Validation {
            stage: "emit LFCA",
            message: format!("{error:?}"),
        })?;
        check_post_emission_bundle(
            candidate.canonical_artifact().bytes(),
            candidate.source_map().bytes(),
            candidate.semantic_diff().bytes(),
            candidate.expected_semantic_diff_base(),
            FormatLimits::HARD,
        )
        .map_err(|error| Error::Validation {
            stage: "post-emission",
            message: format!("{error:?}"),
        })?;
        if candidate.canonical_artifact().bytes() != self.lfca.as_slice() {
            return Err(Error::Validation {
                stage: "portable provenance",
                message: "re-emitted LFCA does not match generate() LFCA".to_owned(),
            });
        }
        Ok((
            candidate.source_map().bytes().to_vec(),
            candidate.semantic_diff().bytes().to_vec(),
        ))
    }

    pub const fn counts(&self) -> ScenarioCounts {
        self.counts
    }

    pub fn lir(&self) -> &laneflow_compiler::ValidatedCanonicalLir {
        self.compilation.lir()
    }
}

pub fn generate(config: &JunctionConfig) -> Result<GeneratedScenario, Error> {
    config.validate()?;
    let topology = build_topology(config)?;
    let catalog = build_catalog(config, &topology)?;
    validate_catalog(&catalog, &topology, config)?;
    let compilation = compile_junction(config, &topology)?;

    let mut catalog_text = toml::to_string_pretty(&catalog)?;
    while catalog_text.ends_with(['\r', '\n']) {
        catalog_text.pop();
    }
    catalog_text.push('\n');
    let catalog_bytes = catalog_text.into_bytes();

    let counts = ScenarioCounts {
        edges: topology.edges.len(),
        movements: topology.movements.len(),
        maneuver_paths: topology.paths.len(),
        maneuver_gates: topology.gates.len(),
        stop_lines: topology.stop_lines.len(),
        waiting_zones: 1,
        conflict_zones: topology.zones.len(),
        streams: topology.streams.len(),
        signal_groups: crate::topology::SIGNAL_GROUPS.len(),
        controllers: 1,
        phases: topology.phases.len(),
        routes: topology.routes.len(),
        portals: topology.portals.len(),
        spawn_slots: catalog.spawn_slots.len(),
    };
    if counts.spawn_slots < MIN_SPAWN_SLOT_COUNT {
        return Err(Error::Config(format!(
            "configuration yields {} spawn slots; at least {MIN_SPAWN_SLOT_COUNT} are required",
            counts.spawn_slots
        )));
    }

    let lfca = emit_lfca(&compilation)?;
    Ok(GeneratedScenario {
        catalog: catalog_bytes,
        lfca,
        counts,
        compilation,
    })
}
