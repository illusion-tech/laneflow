//! #583 固定研究窗口；墙钟、分配和测试专用阶段诊断复用相同输入。

use super::runtime_types;
#[path = "cutover_scale.rs"]
mod cutover;

use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_static_contract::{
    EntityKind, ManeuverPathOrdinal, ParticipantStreamOrdinal, RightOfWayPolicySetId,
    VehicleProfileOrdinal,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};
use runtime_types::{
    CutoverPreflightLimits, CutoverTransaction, CutoverTransactionLimits, LfcaOriginBinding,
    MigrationPolicyKind, NetworkRevisionCutoverDescriptor, PolicyPin, RouteRegisterInput,
    TickInput, TrafficWorld, VehicleSpawnInput, VehicleStatus, WorldConfig, WorldPolicySelection,
    deterministic_state_digest,
};

const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../../../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
);
pub const JOURNAL_BUDGET: u64 = 64 * 1_024 * 1_024;

#[derive(Clone, Copy, Debug)]
pub enum Scene {
    Lane {
        count: u32,
        edges: u32,
        journal: bool,
    },
    Waiting,
    Conflict,
}

pub const CASES: [Scene; 6] = [
    Scene::Lane {
        count: 1_000,
        edges: 256,
        journal: false,
    },
    Scene::Lane {
        count: 10_000,
        edges: 256,
        journal: false,
    },
    Scene::Lane {
        count: 10_000,
        edges: 16,
        journal: false,
    },
    Scene::Lane {
        count: 10_000,
        edges: 256,
        journal: true,
    },
    Scene::Waiting,
    Scene::Conflict,
];

impl Scene {
    pub fn name(self) -> String {
        match self {
            Self::Lane {
                count,
                edges,
                journal,
            } => format!("lane-{count}-{edges}-journal-{journal}"),
            Self::Waiting => "waiting-membership-1".into(),
            Self::Conflict => "conflict-reservation-retry-2".into(),
        }
    }

    pub fn count(self) -> usize {
        match self {
            Self::Lane { count, .. } => count as usize,
            Self::Waiting => 1,
            Self::Conflict => 2,
        }
    }
}

pub struct Fixtures {
    lanes: cutover::Revisions,
    resources: Arc<SharedNetworkRevision>,
}

impl Fixtures {
    pub fn new() -> Self {
        let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD).unwrap();
        Self {
            lanes: cutover::revisions(),
            resources: build_shared_network_revision(
                input,
                SharedNetworkBuildOptions::new(
                    SpatialBuildOption::Omit,
                    SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
                ),
            )
            .unwrap(),
        }
    }

    pub fn world(&self, scene: Scene) -> Harness {
        match scene {
            Scene::Lane {
                count,
                edges,
                journal,
            } => {
                let mut world = cutover::world(&self.lanes, count, edges, edges);
                for _ in 0..32 {
                    world.step(TickInput::new(100)).unwrap();
                }
                let transaction = journal.then(|| {
                    let descriptor = NetworkRevisionCutoverDescriptor::new(
                        LfcaOriginBinding::from_canonical_origin(
                            *self.lanes.base.canonical_origin(),
                        ),
                        LfcaOriginBinding::from_canonical_origin(
                            *self.lanes.target.canonical_origin(),
                        ),
                        Some(self.lanes.diff_binding),
                        MigrationPolicyKind::CrossRevisionDirect,
                        world.world_binding(),
                    );
                    world
                        .prepare_cross_revision_cutover(
                            Arc::clone(&self.lanes.target),
                            cutover::source(&self.lanes.target),
                            &descriptor,
                            &self.lanes.diff,
                            &CutoverPreflightLimits::new(16 * 1_024 * 1_024),
                            &CutoverTransactionLimits {
                                max_journal_bytes: JOURNAL_BUDGET,
                                ..Default::default()
                            },
                        )
                        .unwrap()
                });
                for _ in 0..8 {
                    world.step(TickInput::new(100)).unwrap();
                }
                Harness {
                    world,
                    scene,
                    delta_ms: 100,
                    steps: 64,
                    transaction,
                }
            }
            Scene::Waiting | Scene::Conflict => {
                let policy = RightOfWayPolicySetId::from_untyped(
                    laneflow_compiler::derive_canonical_stable_id_v1(
                        EntityKind::RightOfWayPolicySet,
                        "runtime-fixture-policy",
                        "fixture-policy",
                        &laneflow_compiler::CompileLimits::p100_initial_v1(),
                    )
                    .unwrap(),
                );
                let mut world = TrafficWorld::install(
                    Arc::clone(&self.resources),
                    WorldConfig::new(8, 4, 1_024, 1_024, 1, 4),
                    cutover::source(&self.resources),
                    583,
                    WorldPolicySelection::Pinned(PolicyPin { policy }),
                )
                .unwrap();
                let paths: Vec<_> = match scene {
                    Scene::Waiting => vec![ManeuverPathOrdinal::from_raw(0)],
                    Scene::Conflict => [0, 1]
                        .map(|raw| {
                            self.resources
                                .conflict()
                                .participant_stream(ParticipantStreamOrdinal::from_raw(raw))
                                .unwrap()
                                .maneuver_path()
                        })
                        .to_vec(),
                    Scene::Lane { .. } => unreachable!(),
                };
                for path in paths {
                    let edges = self
                        .resources
                        .traffic()
                        .maneuvers()
                        .maneuver_path(path)
                        .unwrap()
                        .edges()
                        .to_vec();
                    let boundary =
                        self.resources.traffic().lane_lengths_millimetres()[edges[0].index()];
                    let route = world
                        .register_route(RouteRegisterInput::new(edges))
                        .unwrap();
                    let progress = boundary - u32::from(matches!(scene, Scene::Waiting));
                    world
                        .spawn_vehicle(VehicleSpawnInput::new(
                            VehicleProfileOrdinal::from_raw(0),
                            route,
                            0,
                            progress,
                            8_000,
                        ))
                        .unwrap();
                }
                // 与既有 Waiting / Conflict 稳态分配测试相同的有限窗口。
                let warm = if matches!(scene, Scene::Waiting) {
                    1
                } else {
                    9
                };
                for _ in 0..warm {
                    world.step(TickInput::new(4)).unwrap();
                }
                Harness {
                    world,
                    scene,
                    delta_ms: 4,
                    steps: 16,
                    transaction: None,
                }
            }
        }
    }
}

pub struct Harness {
    pub world: TrafficWorld,
    pub scene: Scene,
    pub delta_ms: u64,
    pub steps: usize,
    transaction: Option<CutoverTransaction>,
}

impl Harness {
    pub fn step(&mut self) {
        std::hint::black_box(self.world.step(TickInput::new(self.delta_ms)).unwrap());
    }

    pub fn validate(&self) {
        assert_eq!(self.world.live_vehicles().len(), self.scene.count());
        assert!(self.world.live_vehicles().iter().all(|handle| {
            self.world.vehicle(*handle).unwrap().status() == VehicleStatus::Active
        }));
        match self.scene {
            Scene::Waiting => assert!(self.world.live_vehicles().iter().any(|handle| {
                self.world
                    .vehicle(*handle)
                    .unwrap()
                    .waiting_membership()
                    .is_some()
            })),
            Scene::Conflict => {
                assert_eq!(
                    self.world
                        .live_vehicles()
                        .iter()
                        .filter(|handle| self.world.conflict_reservation(**handle).is_some())
                        .count(),
                    1
                );
                assert!(
                    self.world
                        .latest_conflict_decisions()
                        .iter()
                        .any(|decision| {
                            matches!(
                                decision.outcome(),
                                runtime_types::ConflictDecisionOutcome::NoGrant(_)
                            )
                        })
                );
            }
            Scene::Lane { journal, .. } => {
                assert_eq!(self.world.migration_journal_stats().is_some(), journal);
                if journal {
                    assert!(self.journal_bytes() < JOURNAL_BUDGET);
                }
            }
        }
    }

    pub fn journal_bytes(&self) -> u64 {
        self.world
            .migration_journal_stats()
            .map_or(0, |stats| stats.written_bytes)
    }

    pub fn digest(&self) -> String {
        format!(
            "{:x}",
            deterministic_state_digest(&self.world.capture_snapshot().unwrap()).unwrap()
        )
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(transaction) = self.transaction.take() {
            transaction.abandon(&mut self.world).unwrap();
        }
    }
}
