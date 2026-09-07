use std::{collections::BTreeMap, fs, path::Path, sync::Arc};

use laneflow_compiler::{CompileLimits, derive_canonical_stable_id_v1};
use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CommittedNetworkSource, PolicyPin, PublishedLfcaReference, TrafficWorld, WorldConfig,
    WorldPolicySelection,
};
use laneflow_static_contract::{
    EntityKind, LaneEdgeId, LaneEdgeOrdinal, ParticipantStreamOrdinal, RightOfWayPolicySetId,
    SignalGroupId, SignalGroupOrdinal,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};
use laneflow_urban_generator::{Catalog, Scale};
use serde::{Deserialize, Serialize};

use crate::{Result, checked, invalid, sha256};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct FileDigest {
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Deserialize)]
struct Manifest {
    manifest_version: u32,
    scale: String,
    tiles: u32,
    nominal_individuals: u32,
    fixed_step_ms: u64,
    network_revision: String,
    files: BTreeMap<String, FileDigest>,
}

pub struct Artifacts {
    pub(crate) catalog: Catalog,
    pub(crate) revision: Arc<SharedNetworkRevision>,
    pub(crate) files: BTreeMap<String, FileDigest>,
    pub(crate) manifest_digest: String,
    pub(crate) individuals: u32,
    pub(crate) tiles: u32,
    pub(crate) dt: u64,
    pub(crate) edges: BTreeMap<String, LaneEdgeOrdinal>,
}

impl Artifacts {
    /// Borrows the catalog bound to the loaded source digests.
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Borrows the shared network bound to the loaded source digests.
    pub fn revision(&self) -> &Arc<SharedNetworkRevision> {
        &self.revision
    }

    /// Loads the generator's real checked LFCA. No topology is synthesized by the runner.
    pub fn load(directory: &Path) -> Result<Self> {
        let manifest_bytes = fs::read(directory.join("manifest.toml"))?;
        let manifest: Manifest = toml::from_str(
            std::str::from_utf8(&manifest_bytes).map_err(|e| invalid(e.to_string()))?,
        )?;
        let scale = checked("artifact scale", Scale::parse(&manifest.scale))?;
        if manifest.manifest_version != 1
            || manifest.tiles != scale.tile_count()
            || manifest.nominal_individuals != scale.nominal_individual_count()
            || manifest.fixed_step_ms != scale.fixed_step_ms()
        {
            return Err(invalid("artifact shape differs from its fixed scale"));
        }
        let mut consumed = BTreeMap::new();
        let mut read = |name: &str| -> Result<Vec<u8>> {
            let bytes = fs::read(directory.join(name))?;
            let actual = FileDigest {
                bytes: bytes.len() as u64,
                sha256: sha256(&bytes),
            };
            if manifest.files.get(name) != Some(&actual) {
                return Err(invalid(format!("artifact digest mismatch: {name}")));
            }
            consumed.insert(name.to_owned(), actual);
            Ok(bytes)
        };
        let catalog: Catalog = toml::from_str(
            std::str::from_utf8(&read("routes.toml")?).map_err(|e| invalid(e.to_string()))?,
        )?;
        let _config = read("config.toml")?;
        for name in ["common.lfre", "topology.lfre"] {
            let _ = read(name)?;
        }
        let bytes = read("network.lfca")?;
        let input = checked(
            "LFCA check",
            check_canonical_network_input(bytes.as_slice(), FormatLimits::HARD),
        )?;
        let revision = checked(
            "shared network",
            build_shared_network_revision(
                input,
                SharedNetworkBuildOptions::new(
                    SpatialBuildOption::Omit,
                    SharedNetworkBuildLimits::new(2_147_483_648, 2_147_483_648),
                ),
            ),
        )?;
        if catalog.catalog_version != 1
            || catalog.scale != manifest.scale
            || catalog.network_revision != manifest.network_revision
            || format!(
                "{:x}",
                revision.canonical_origin().network_revision().as_digest()
            ) != manifest.network_revision
        {
            return Err(invalid("catalog and LFCA identity differ"));
        }
        let mut edges = BTreeMap::new();
        for (key, id) in &catalog.edge_ids {
            let id: LaneEdgeId = crate::catalog_id(id)?;
            let ordinal = revision
                .identity()
                .ordinal(id)
                .ok_or_else(|| invalid(format!("unknown edge {key}")))?;
            edges.insert(key.clone(), ordinal);
        }
        Ok(Self {
            catalog,
            revision,
            files: consumed,
            manifest_digest: sha256(&manifest_bytes),
            individuals: manifest.nominal_individuals,
            tiles: manifest.tiles,
            dt: manifest.fixed_step_ms,
            edges,
        })
    }

    pub(crate) fn install(&self) -> Result<TrafficWorld> {
        let mut passages = BTreeMap::new();
        for raw in 0..self
            .revision
            .identity()
            .entity_count(EntityKind::ParticipantStream)
        {
            let stream = self
                .revision
                .conflict()
                .participant_stream(ParticipantStreamOrdinal::from_raw(raw))
                .expect("checked stream");
            let path = self
                .revision
                .traffic()
                .maneuvers()
                .maneuver_path(stream.maneuver_path())
                .expect("checked path");
            passages.insert(path.edges()[1], stream.passages().len() as u64);
        }
        let edge_capacity = self
            .catalog
            .routes
            .iter()
            .map(|r| r.edge_keys.len() as u64)
            .sum();
        let mut conflict_capacity = 0_u64;
        for route in &self.catalog.routes {
            for key in &route.edge_keys {
                let edge = self
                    .edges
                    .get(key)
                    .ok_or_else(|| invalid(format!("unknown route edge {key}")))?;
                conflict_capacity += passages.get(edge).copied().unwrap_or(0);
            }
        }
        let origin = self.revision.canonical_origin();
        let pin: RightOfWayPolicySetId = crate::catalog_id(&self.catalog.policy_id)?;
        let source = checked(
            "source reference",
            PublishedLfcaReference::new(
                "fixture://lf-cn-urban-v1",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            ),
        )?;
        let world = checked(
            "world install",
            TrafficWorld::install(
                self.revision.clone(),
                WorldConfig::new(
                    self.individuals,
                    self.catalog.routes.len() as u32,
                    edge_capacity,
                    conflict_capacity,
                    1,
                    self.dt,
                ),
                CommittedNetworkSource::Published { reference: source },
                544,
                WorldPolicySelection::Pinned(PolicyPin { policy: pin }),
            ),
        )?;
        if world.policy_gap_profiles().is_empty()
            || world
                .policy_gap_profiles()
                .iter()
                .any(|g| g.required_lead_ms() != 5_500 + self.dt || g.required_lag_ms() != 2_500)
        {
            return Err(invalid("urban-conservative-v1 gap differs"));
        }
        Ok(world)
    }

    pub(crate) fn signal_group(&self, key: &str) -> Result<SignalGroupOrdinal> {
        let id = derive_canonical_stable_id_v1(
            EntityKind::SignalGroup,
            &self.catalog.namespace,
            key,
            &CompileLimits::single_network_1m_v2(),
        )
        .map_err(|error| invalid(format!("signal group identity: {error:?}")))?;
        self.revision
            .identity()
            .ordinal(SignalGroupId::from_untyped(id))
            .ok_or_else(|| invalid(format!("unknown signal group {key}")))
    }
}
