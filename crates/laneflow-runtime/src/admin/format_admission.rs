//! Runtime 的唯一原始格式入口：LFSD 认证及 LFRS verifier、lowering 与编码。
//!
//! wire 类型及字段访问只存在于此模块。LFRS lowering 直接构造局部 staging world，
//! 保持原有校验先后、共同运行时不变量和分配行为；不建立额外的全量解码副本。
//! 只有所有检查成功才返回完整恢复结果，kernel 不接收 wire view。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use laneflow_runtime_snapshot_wire::generated::lane_flow::runtime_snapshot::v5 as wire;
use laneflow_runtime_snapshot_wire::runtime::VerifierOptions;
use laneflow_static_contract::{
    ConflictZoneId, LaneEdgeId, ManeuverGateId, ManeuverPathId, ParkingFacilityId, ParkingSpaceId,
    ParticipantClassId, ParticipantStreamId, StableId128, VehicleProfileId, WaitingZoneId,
    WaitingZoneOrdinal,
};
use laneflow_static_network::SharedNetworkRevision;

use crate::{
    AdmittedRouteRegisterInput, CommittedNetworkSource, InstallError, ManeuverTraversalPhase,
    ManeuverTraversalState, ObservationStateSequence, ParkedVehicleSpawnInput, ParkingTarget,
    ReserveParkingTarget, RouteHandle, TrafficWorld, VehicleHandle, VehicleSpawnInput,
    VehicleStatus, VirtualEntryAnchorSelector, WaitingMembership, WorldConfig,
};
#[cfg(test)]
use crate::{ParkingError, SpawnError};
use crate::{RUNTIME_STATE_VERSION, SNAPSHOT_FORMAT_VERSION};

use super::cutover::{CutoverDescriptorError, SemanticDiffOriginBinding};
use super::snapshot::{
    CapturedManeuverTraversalPhase, CapturedParkingBinding, CapturedParkingTarget, CapturedSnapshot,
};
use super::snapshot_restore::{
    RestoredSnapshot, SnapshotLimitDimension, SnapshotRestoreError, SnapshotRestoreLimits,
};
use laneflow_format::{RegistryCheckedFieldValue, preflight_object_values};
use laneflow_runtime_snapshot_wire::runtime;
use laneflow_static_contract::{
    NETWORK_REVISION_DERIVATION_VERSION, NetworkRevisionId, PortableObjectKind, Sha256Digest,
};
use laneflow_static_network::CanonicalNetworkOrigin;
use sha2::Digest as _;

const MIN_SIZE_PREFIXED_LFRS_BYTES: usize = 12;
const MAX_SCHEMA_TABLE_DEPTH: usize = 6;
const APPARENT_SIZE_MULTIPLIER: usize = 16;
const MICROMETRES_PER_MILLIMETRE: u16 = 1_000;
const ROOT_V5_FIELDS: usize = vtable_field_count(wire::RuntimeSnapshot::VT_CONFLICT_LAG_STATES);
const WORLD_CONFIG_V5_FIELDS: usize =
    vtable_field_count(wire::WorldConfigBinding::VT_FIXED_DELTA_TIME_MS);
const PUBLISHED_SOURCE_V5_FIELDS: usize =
    vtable_field_count(wire::PublishedSourceBinding::VT_NETWORK_REVISION);
const ROUTE_V5_FIELDS: usize = vtable_field_count(wire::SnapshotRoute::VT_EDGES);
const VEHICLE_V5_FIELDS: usize = vtable_field_count(wire::SnapshotVehicle::VT_CONFLICT_RESERVATION);
const PARKING_BINDING_V5_FIELDS: usize =
    vtable_field_count(wire::ParkingBinding::VT_VIRTUAL_ENTRY_PROGRESS_MM);
const MANEUVER_TRAVERSAL_V5_FIELDS: usize =
    vtable_field_count(wire::ManeuverTraversalBinding::VT_PHASE_GATE);
const WAITING_MEMBERSHIP_V5_FIELDS: usize =
    vtable_field_count(wire::WaitingMembershipBinding::VT_ADMISSION_SEQUENCE);
const WAITING_ZONE_STATE_V5_FIELDS: usize =
    vtable_field_count(wire::WaitingZoneState::VT_NEXT_ADMISSION_SEQUENCE);
const CONFLICT_LOCATOR_V5_FIELDS: usize =
    vtable_field_count(wire::ConflictPassageLocatorBinding::VT_CONFLICT_ZONE);
const CONFLICT_ELIGIBILITY_V5_FIELDS: usize =
    vtable_field_count(wire::ConflictEligibilityBinding::VT_FIRST_ELIGIBLE_TICK);
const CONFLICT_PASSAGE_V5_FIELDS: usize =
    vtable_field_count(wire::ConflictPassageBinding::VT_CLEARANCE_PROGRESS_MM);
const CONFLICT_DOWNSTREAM_V5_FIELDS: usize =
    vtable_field_count(wire::ConflictDownstreamIntervalBinding::VT_END_MM);
const CONFLICT_RESERVATION_V5_FIELDS: usize =
    vtable_field_count(wire::ConflictReservationBinding::VT_DOWNSTREAM_INTERVALS);
const CONFLICT_LAG_STATE_V5_FIELDS: usize =
    vtable_field_count(wire::ConflictLagState::VT_REFERENCE_TIME_MS);

const fn vtable_field_count(
    last_field: laneflow_runtime_snapshot_wire::runtime::VOffsetT,
) -> usize {
    (last_field as usize - 4) / 2 + 1
}

/// 保持长度、摘要、结构、base/target 绑定的认证顺序。
pub(super) fn verify_semantic_diff(
    binding: Option<&SemanticDiffOriginBinding>,
    lfsd_bytes: &[u8],
    base_origin: CanonicalNetworkOrigin,
    target_origin: CanonicalNetworkOrigin,
) -> Result<(), CutoverDescriptorError> {
    let Some(binding) = binding else {
        return Err(CutoverDescriptorError::CrossRevisionRequiresSemanticDiff);
    };
    let declared_length = binding.semantic_diff_byte_length().get();
    let actual_length = u64::try_from(lfsd_bytes.len()).map_err(|_| {
        CutoverDescriptorError::SemanticDiffByteLengthMismatch {
            declared: declared_length,
            actual: u64::MAX,
        }
    })?;
    if actual_length != declared_length {
        return Err(CutoverDescriptorError::SemanticDiffByteLengthMismatch {
            declared: declared_length,
            actual: actual_length,
        });
    }
    let digest: [u8; 32] = sha2::Sha256::digest(lfsd_bytes).into();
    if Sha256Digest::from_bytes(digest) != binding.semantic_diff_digest() {
        return Err(CutoverDescriptorError::SemanticDiffDigestMismatch);
    }
    let view = preflight_object_values(
        lfsd_bytes,
        PortableObjectKind::SemanticDiff,
        laneflow_format::FormatLimits::HARD,
    )
    .map_err(|_| CutoverDescriptorError::SemanticDiffStructureInvalid)?
    .registry_view();
    let row = view
        .section(0)
        .and_then(|section| section.table(0))
        .and_then(|table| table.row(0))
        .ok_or(CutoverDescriptorError::SemanticDiffStructureInvalid)?;
    let field = |tag: u16| {
        row.field_by_tag(tag)
            .expect("schema-required LFSD binding field")
            .value()
            .expect("registry-checked LFSD binding value")
    };
    let u8_of = |tag: u16| match field(tag) {
        RegistryCheckedFieldValue::U8(value) => value,
        _ => panic!("LFSD binding field type drift at tag {tag}"),
    };
    let u16_of = |tag: u16| match field(tag) {
        RegistryCheckedFieldValue::U16(value) => value,
        _ => panic!("LFSD binding field type drift at tag {tag}"),
    };
    let u64_of = |tag: u16| match field(tag) {
        RegistryCheckedFieldValue::U64(value) => value,
        _ => panic!("LFSD binding field type drift at tag {tag}"),
    };
    let sha_of = |tag: u16| match field(tag) {
        RegistryCheckedFieldValue::Sha256(value) => value,
        _ => panic!("LFSD binding field type drift at tag {tag}"),
    };
    // 绑定行种类：v1 只承认制品绑定（值 1，沿 lfsd-noop/change-set 冻结值）。
    const SEMANTIC_DIFF_ARTIFACT_BINDING_KIND: u8 = 1;
    let binding_kind = u8_of(1);
    if binding_kind != SEMANTIC_DIFF_ARTIFACT_BINDING_KIND {
        return Err(CutoverDescriptorError::SemanticDiffBindingKindUnsupported {
            actual: binding_kind,
        });
    }
    let base_matches = u16_of(2) == NETWORK_REVISION_DERIVATION_VERSION
        && NetworkRevisionId::from_digest(sha_of(3)) == base_origin.network_revision()
        && sha_of(4) == base_origin.canonical_artifact_digest()
        && u64_of(5) == base_origin.canonical_artifact_byte_length().get();
    if !base_matches {
        return Err(CutoverDescriptorError::SemanticDiffBaseBindingMismatch);
    }
    let target_matches = u16_of(6) == NETWORK_REVISION_DERIVATION_VERSION
        && NetworkRevisionId::from_digest(sha_of(7)) == target_origin.network_revision()
        && sha_of(8) == target_origin.canonical_artifact_digest()
        && u64_of(9) == target_origin.canonical_artifact_byte_length().get();
    if !target_matches {
        return Err(CutoverDescriptorError::SemanticDiffTargetBindingMismatch);
    }
    Ok(())
}

/// 把不可变快照点编码为 size-prefixed `LFRS` v5。
///
/// 捕获与编码分离：调用方可先在固定步进安全边界调用
/// [`TrafficWorld::capture_snapshot`]，再把本函数放到后台线程。编码只映射已捕获
/// 事实，不重新读取活动 world；输出始终携带 `LFRS` file identifier。
#[must_use]
pub(super) fn encode_lfrs(snapshot: &CapturedSnapshot) -> Vec<u8> {
    let mut fbb = runtime::FlatBufferBuilder::new();

    let world_config = wire::WorldConfigBinding::create(
        &mut fbb,
        &wire::WorldConfigBindingArgs {
            vehicle_capacity: snapshot.config.vehicle_capacity(),
            route_capacity: snapshot.config.route_capacity(),
            route_edge_occurrence_capacity: snapshot.config.route_edge_occurrence_capacity(),
            route_conflict_occurrence_capacity: snapshot
                .config
                .route_conflict_occurrence_capacity(),
            worker_count: snapshot.config.worker_count(),
            fixed_delta_time_ms: snapshot.config.fixed_delta_time_ms(),
        },
    );

    let route_offsets = snapshot
        .routes
        .iter()
        .map(|route| {
            let edges = route
                .edges
                .iter()
                .map(|stable_id| wire::StableId128::new(stable_id.as_bytes()))
                .collect::<Vec<_>>();
            let edges = fbb.create_vector(&edges);
            wire::SnapshotRoute::create(
                &mut fbb,
                &wire::SnapshotRouteArgs {
                    snapshot_route_id: route.snapshot_route_id,
                    edges: Some(edges),
                },
            )
        })
        .collect::<Vec<_>>();
    let routes = fbb.create_vector(&route_offsets);

    let vehicle_offsets = snapshot
        .vehicles
        .iter()
        .map(|vehicle| {
            let profile = wire::StableId128::new(vehicle.profile.as_bytes());
            let class = wire::StableId128::new(vehicle.class.as_bytes());
            let parking = vehicle.parking.map(|binding| {
                let (state, target, target_kind, entry_occurrence, virtual_entry) = match binding {
                    CapturedParkingBinding::Reserved {
                        target,
                        entry_route_occurrence,
                        virtual_entry,
                    } => (
                        wire::ParkingBindingStateKind::Reserved,
                        target,
                        match target {
                            CapturedParkingTarget::ExplicitSpace(_) => {
                                wire::ParkingTargetKind::ExplicitSpace
                            }
                            CapturedParkingTarget::VirtualPool(_) => {
                                wire::ParkingTargetKind::VirtualPool
                            }
                        },
                        entry_route_occurrence,
                        virtual_entry,
                    ),
                    CapturedParkingBinding::Occupied { target } => (
                        wire::ParkingBindingStateKind::Occupied,
                        target,
                        match target {
                            CapturedParkingTarget::ExplicitSpace(_) => {
                                wire::ParkingTargetKind::ExplicitSpace
                            }
                            CapturedParkingTarget::VirtualPool(_) => {
                                wire::ParkingTargetKind::VirtualPool
                            }
                        },
                        0,
                        None,
                    ),
                };
                let target = match target {
                    CapturedParkingTarget::ExplicitSpace(stable)
                    | CapturedParkingTarget::VirtualPool(stable) => {
                        wire::StableId128::new(stable.as_bytes())
                    }
                };
                let virtual_entry_edge =
                    virtual_entry.map(|entry| wire::StableId128::new(entry.lane_edge.as_bytes()));
                wire::ParkingBinding::create(
                    &mut fbb,
                    &wire::ParkingBindingArgs {
                        state,
                        target_kind,
                        target: Some(&target),
                        entry_route_occurrence: entry_occurrence,
                        virtual_entry_edge: virtual_entry_edge.as_ref(),
                        virtual_entry_progress_mm: virtual_entry
                            .map_or(0, |entry| entry.progress_mm),
                    },
                )
            });
            let maneuver_traversal = vehicle.maneuver_traversal.map(|traversal| {
                let maneuver_path = wire::StableId128::new(traversal.maneuver_path.as_bytes());
                let phase_gate = wire::StableId128::new(traversal.phase_gate.as_bytes());
                wire::ManeuverTraversalBinding::create(
                    &mut fbb,
                    &wire::ManeuverTraversalBindingArgs {
                        maneuver_occurrence_index: traversal.maneuver_occurrence_index,
                        maneuver_path: Some(&maneuver_path),
                        phase: match traversal.phase {
                            CapturedManeuverTraversalPhase::PreGate => {
                                wire::ManeuverTraversalPhaseKind::PreGate
                            }
                            CapturedManeuverTraversalPhase::Committed => {
                                wire::ManeuverTraversalPhaseKind::Committed
                            }
                            CapturedManeuverTraversalPhase::Waiting => {
                                wire::ManeuverTraversalPhaseKind::Waiting
                            }
                            CapturedManeuverTraversalPhase::Clearing => {
                                wire::ManeuverTraversalPhaseKind::Clearing
                            }
                        },
                        phase_gate: Some(&phase_gate),
                    },
                )
            });
            let waiting_membership = vehicle.waiting_membership.map(|membership| {
                let waiting_zone = wire::StableId128::new(membership.waiting_zone.as_bytes());
                let entry_gate = wire::StableId128::new(membership.entry_gate.as_bytes());
                let release_gate = wire::StableId128::new(membership.release_gate.as_bytes());
                wire::WaitingMembershipBinding::create(
                    &mut fbb,
                    &wire::WaitingMembershipBindingArgs {
                        waiting_zone: Some(&waiting_zone),
                        maneuver_occurrence_index: membership.maneuver_occurrence_index,
                        entry_gate: Some(&entry_gate),
                        release_gate: Some(&release_gate),
                        admission_sequence: membership.admission_sequence,
                    },
                )
            });
            let conflict_eligibility = vehicle.conflict_eligibility.map(|eligibility| {
                let participant_stream =
                    wire::StableId128::new(eligibility.passage.participant_stream.as_bytes());
                let conflict_zone =
                    wire::StableId128::new(eligibility.passage.conflict_zone.as_bytes());
                let passage = wire::ConflictPassageLocatorBinding::create(
                    &mut fbb,
                    &wire::ConflictPassageLocatorBindingArgs {
                        participant_stream: Some(&participant_stream),
                        conflict_zone: Some(&conflict_zone),
                    },
                );
                let admission_gate = wire::StableId128::new(eligibility.admission_gate.as_bytes());
                wire::ConflictEligibilityBinding::create(
                    &mut fbb,
                    &wire::ConflictEligibilityBindingArgs {
                        maneuver_occurrence_index: eligibility.maneuver_occurrence_index,
                        maneuver_entry_route_edge_index: eligibility
                            .maneuver_entry_route_edge_index,
                        admission_gate: Some(&admission_gate),
                        conflict_occurrence_index: eligibility.conflict_occurrence_index,
                        passage: Some(passage),
                        first_eligible_tick: eligibility.first_eligible_tick,
                    },
                )
            });
            let conflict_reservation = vehicle.conflict_reservation.as_ref().map(|reservation| {
                let passage_offsets = reservation
                    .passages
                    .iter()
                    .map(|row| {
                        let participant_stream =
                            wire::StableId128::new(row.passage.participant_stream.as_bytes());
                        let conflict_zone =
                            wire::StableId128::new(row.passage.conflict_zone.as_bytes());
                        let passage = wire::ConflictPassageLocatorBinding::create(
                            &mut fbb,
                            &wire::ConflictPassageLocatorBindingArgs {
                                participant_stream: Some(&participant_stream),
                                conflict_zone: Some(&conflict_zone),
                            },
                        );
                        wire::ConflictPassageBinding::create(
                            &mut fbb,
                            &wire::ConflictPassageBindingArgs {
                                conflict_occurrence_index: row.conflict_occurrence_index,
                                passage: Some(passage),
                                entry_route_edge_index: row.entry_route_edge_index,
                                entry_progress_mm: row.entry_progress_mm,
                                clearance_route_edge_index: row.clearance_route_edge_index,
                                clearance_progress_mm: row.clearance_progress_mm,
                            },
                        )
                    })
                    .collect::<Vec<_>>();
                let passages = fbb.create_vector(&passage_offsets);
                let downstream_offsets = reservation
                    .downstream_intervals
                    .iter()
                    .map(|row| {
                        let lane_edge = wire::StableId128::new(row.lane_edge.as_bytes());
                        wire::ConflictDownstreamIntervalBinding::create(
                            &mut fbb,
                            &wire::ConflictDownstreamIntervalBindingArgs {
                                lane_edge: Some(&lane_edge),
                                start_mm: row.start_mm,
                                end_mm: row.end_mm,
                            },
                        )
                    })
                    .collect::<Vec<_>>();
                let downstream_intervals = fbb.create_vector(&downstream_offsets);
                let admission_gate = wire::StableId128::new(reservation.admission_gate.as_bytes());
                wire::ConflictReservationBinding::create(
                    &mut fbb,
                    &wire::ConflictReservationBindingArgs {
                        acquired_tick: reservation.acquired_tick,
                        maneuver_occurrence_index: reservation.maneuver_occurrence_index,
                        maneuver_entry_route_edge_index: reservation
                            .maneuver_entry_route_edge_index,
                        admission_gate: Some(&admission_gate),
                        passages: Some(passages),
                        downstream_intervals: Some(downstream_intervals),
                    },
                )
            });
            wire::SnapshotVehicle::create(
                &mut fbb,
                &wire::SnapshotVehicleArgs {
                    snapshot_vehicle_id: vehicle.snapshot_vehicle_id,
                    snapshot_route_id: vehicle.snapshot_route_id,
                    route_edge_index: vehicle.route_edge_index,
                    progress_mm: vehicle.progress_mm,
                    carry_um: vehicle.carry_um,
                    speed_mm_s: vehicle.speed_mm_s,
                    status: encode_vehicle_status(vehicle.status),
                    profile: Some(&profile),
                    class: Some(&class),
                    parking,
                    maneuver_traversal,
                    waiting_membership,
                    conflict_eligibility,
                    conflict_reservation,
                },
            )
        })
        .collect::<Vec<_>>();
    let vehicles = fbb.create_vector(&vehicle_offsets);
    let live_order = fbb.create_vector(&snapshot.live_order);
    let waiting_zone_offsets = snapshot
        .waiting_zones
        .iter()
        .map(|state| {
            let waiting_zone = wire::StableId128::new(state.waiting_zone.as_bytes());
            wire::WaitingZoneState::create(
                &mut fbb,
                &wire::WaitingZoneStateArgs {
                    waiting_zone: Some(&waiting_zone),
                    occupancy: state.occupancy,
                    next_admission_sequence: state.next_admission_sequence,
                },
            )
        })
        .collect::<Vec<_>>();
    let waiting_zones = fbb.create_vector(&waiting_zone_offsets);
    let conflict_lag_offsets = snapshot
        .conflict_lag_states
        .iter()
        .map(|state| {
            let participant_stream =
                wire::StableId128::new(state.passage.participant_stream.as_bytes());
            let conflict_zone = wire::StableId128::new(state.passage.conflict_zone.as_bytes());
            let passage = wire::ConflictPassageLocatorBinding::create(
                &mut fbb,
                &wire::ConflictPassageLocatorBindingArgs {
                    participant_stream: Some(&participant_stream),
                    conflict_zone: Some(&conflict_zone),
                },
            );
            let (reference_kind, reference_time_ms) = match state.reference {
                crate::ConflictLagReference::ActualClear(time) => {
                    (wire::ConflictLagReferenceKind::ActualClear, time)
                }
                crate::ConflictLagReference::CutoverFloor(time) => {
                    (wire::ConflictLagReferenceKind::CutoverFloor, time)
                }
                crate::ConflictLagReference::NoHistory => {
                    unreachable!("NoHistory rows are omitted during capture")
                }
            };
            wire::ConflictLagState::create(
                &mut fbb,
                &wire::ConflictLagStateArgs {
                    passage: Some(passage),
                    reference_kind,
                    reference_time_ms,
                },
            )
        })
        .collect::<Vec<_>>();
    let conflict_lag_states = fbb.create_vector(&conflict_lag_offsets);

    let (source_kind, source_published) = match &snapshot.source {
        CommittedNetworkSource::Published { reference } => {
            let asset_key = fbb.create_string(reference.asset_key());
            let artifact_digest =
                wire::Digest256::new(reference.canonical_artifact_digest().as_bytes());
            let network_revision =
                wire::Digest256::new(reference.network_revision().as_digest().as_bytes());
            let published = wire::PublishedSourceBinding::create(
                &mut fbb,
                &wire::PublishedSourceBindingArgs {
                    asset_key: Some(asset_key),
                    artifact_digest: Some(&artifact_digest),
                    artifact_byte_length: reference.canonical_artifact_byte_length().get(),
                    network_revision: Some(&network_revision),
                },
            );
            (wire::SourceKind::Published, Some(published))
        }
    };

    let origin = snapshot.origin;
    let network_revision = wire::Digest256::new(origin.network_revision().as_digest().as_bytes());
    let artifact_digest = wire::Digest256::new(origin.canonical_artifact_digest().as_bytes());
    let contracts = origin.static_contract_versions();
    let contract_versions = wire::StaticContractVersionSet::new(
        contracts.canonical_format_version(),
        contracts.identity_encoding_version(),
        contracts.identity_registry_revision(),
        contracts.network_revision_derivation_version(),
        contracts.constraint_contract_version(),
        contracts.static_execution_contract_version(),
    );
    let (selection, policy) = match snapshot.policy_selection {
        crate::WorldPolicySelection::NotRequired => {
            (wire::WorldPolicySelectionKind::NotRequired, None)
        }
        crate::WorldPolicySelection::Pinned(pin) => (
            wire::WorldPolicySelectionKind::Pinned,
            Some(wire::StableId128::new(pin.policy.as_untyped().as_bytes())),
        ),
    };
    let world_policy = wire::WorldPolicyBinding::create(
        &mut fbb,
        &wire::WorldPolicyBindingArgs {
            selection,
            policy: policy.as_ref(),
        },
    );
    let root = wire::RuntimeSnapshot::create(
        &mut fbb,
        &wire::RuntimeSnapshotArgs {
            format_version: SNAPSHOT_FORMAT_VERSION,
            runtime_state_version: RUNTIME_STATE_VERSION,
            world_id: snapshot.world_id,
            tick: snapshot.tick,
            time_ms: snapshot.time_ms,
            command_cursor: snapshot.command_cursor,
            event_cursor: snapshot.event_cursor,
            world_config: Some(world_config),
            network_revision: Some(&network_revision),
            lfca_artifact_digest: Some(&artifact_digest),
            lfca_artifact_byte_length: origin.canonical_artifact_byte_length().get(),
            static_contract_versions: Some(&contract_versions),
            source_kind,
            source_published,
            routes: Some(routes),
            vehicles: Some(vehicles),
            live_order: Some(live_order),
            waiting_zones: Some(waiting_zones),
            world_policy: Some(world_policy),
            conflict_lag_states: Some(conflict_lag_states),
        },
    );
    wire::finish_size_prefixed_runtime_snapshot_buffer(&mut fbb, root);
    fbb.finished_data().to_vec()
}

const fn encode_vehicle_status(status: VehicleStatus) -> wire::VehicleStatusKind {
    match status {
        VehicleStatus::Active => wire::VehicleStatusKind::Active,
        VehicleStatus::Parked => wire::VehicleStatusKind::Parked,
        VehicleStatus::Completed => wire::VehicleStatusKind::Completed,
    }
}

/// verifier-first 读取 `LFRS`，核对目标根/来源/配置并原子构造一个新 world。
///
/// fresh restore 从 [`crate::WorldGeneration::INITIAL`] 建立新观测 stream；调用方不得让
/// 同一 `world_id` 的旧 world 或旧 session 与返回值并存。任一失败只丢弃局部 staging，
/// 不返回半恢复 world。
pub(super) fn restore_lfrs(
    bytes: &[u8],
    revision: Arc<SharedNetworkRevision>,
    source: CommittedNetworkSource,
    target_config: WorldConfig,
    limits: SnapshotRestoreLimits,
) -> Result<RestoredSnapshot, SnapshotRestoreError> {
    let root = verify_lfrs(bytes, limits)?;
    validate_bindings(root, revision.as_ref(), &source, target_config, limits)?;

    // 路线重编译必须得到完整实际冲突出现项总数，才能同时校验
    // 快照与目标容量。这个 staging world 不对外发布，也不按该上限预分配内存。
    let staging_config = WorldConfig::new(
        target_config.vehicle_capacity(),
        target_config.route_capacity(),
        target_config.route_edge_occurrence_capacity(),
        u64::MAX,
        target_config.worker_count(),
        target_config.fixed_delta_time_ms(),
    );
    let mut world = TrafficWorld::install(
        revision,
        staging_config,
        source,
        root.world_id(),
        decode_world_policy(root.world_policy())?,
    )
    .map_err(SnapshotRestoreError::Install)?;
    let root_revision = root
        .network_revision()
        .expect("binding validation requires network_revision");
    let contracts = root
        .static_contract_versions()
        .expect("binding validation requires static_contract_versions");

    let mut route_map = BTreeMap::new();
    for route in root.routes() {
        let snapshot_route_id = route.snapshot_route_id();
        if snapshot_route_id == 0 {
            return Err(SnapshotRestoreError::ZeroRouteId);
        }
        if route_map.contains_key(&snapshot_route_id) {
            return Err(SnapshotRestoreError::DuplicateRouteId { snapshot_route_id });
        }
        let stable_edges = route
            .edges()
            .iter()
            .map(|stable_id| StableId128::from_bytes(stable_id.0))
            .collect::<Vec<_>>();
        let handle = world
            .register_admitted_route(AdmittedRouteRegisterInput::new(
                laneflow_static_contract::NetworkRevisionId::from_digest(
                    laneflow_static_contract::Sha256Digest::from_bytes(root_revision.0),
                ),
                contracts.network_revision_derivation_version(),
                stable_edges,
            ))
            .map_err(|error| SnapshotRestoreError::Route {
                snapshot_route_id,
                error,
            })?;
        route_map.insert(snapshot_route_id, handle);
    }
    let snapshot_config = root.world_config();
    validate_state_count(
        SnapshotLimitDimension::RouteConflictOccurrences,
        world.committed.live_route_conflict_occurrence_count,
        snapshot_config.route_conflict_occurrence_capacity(),
        target_config.route_conflict_occurrence_capacity(),
    )?;
    world.binding.config = target_config;
    // 在私有 staging 恢复已提交时钟；Waiting phase 保留历史归因，不按当前信号重解释。
    world.committed.tick_index = root.tick();
    world.committed.time_ms = root.time_ms();
    world.committed.event_cursor = root.event_cursor();
    world.refresh_signals();

    let vehicle_rows = root.vehicles();
    let mut vehicle_map = BTreeMap::new();
    // 非 Active 先恢复，Active 最后恢复；每辆车都以最终状态一次提交，
    // 因此 Completed 不会被临时当成 Active，Active 的 carry 也参与提交前 3A 校验。
    for active_pass in [false, true] {
        for vehicle in vehicle_rows {
            let status = decode_vehicle_status(vehicle.snapshot_vehicle_id(), vehicle.status())?;
            if (status == VehicleStatus::Active) != active_pass {
                continue;
            }
            restore_vehicle(&mut world, vehicle, status, &route_map, &mut vehicle_map)?;
        }
    }

    let mut seen_live = BTreeSet::new();
    let mut live_order = Vec::with_capacity(root.live_order().len());
    for snapshot_vehicle_id in root.live_order() {
        let Some(handle) = vehicle_map.get(&snapshot_vehicle_id).copied() else {
            return Err(SnapshotRestoreError::UnknownLiveOrderVehicle {
                snapshot_vehicle_id,
            });
        };
        if !seen_live.insert(snapshot_vehicle_id) {
            return Err(SnapshotRestoreError::DuplicateLiveOrderVehicle {
                snapshot_vehicle_id,
            });
        }
        live_order.push(handle);
    }
    if seen_live.len() != vehicle_map.len() {
        return Err(SnapshotRestoreError::IncompleteLiveOrder);
    }

    world.committed.live_order = live_order;
    world.rebuild_active_order();
    restore_waiting_aggregate(&mut world, root)?;
    restore_conflict_aggregate(&mut world, root, &vehicle_map)?;
    world.committed.observation_state_sequence = ObservationStateSequence::INITIAL;
    world.committed.command_cursor = root.command_cursor();
    world.committed.event_cursor = root.event_cursor();
    world.workspace.next_states.clear();
    world.refresh_signals();
    world
        .rebuild_occupancy_index()
        .map_err(SnapshotRestoreError::Occupancy)?;

    Ok(RestoredSnapshot {
        world,
        routes: route_map.into_iter().collect(),
        vehicles: vehicle_map.into_iter().collect(),
    })
}

fn decode_conflict_locator(
    world: &TrafficWorld,
    binding: wire::ConflictPassageLocatorBinding<'_>,
) -> Result<(crate::ConflictPassageLocator, crate::ConflictPassageAddress), ()> {
    let stream_stable = binding.participant_stream().ok_or(())?;
    let zone_stable = binding.conflict_zone().ok_or(())?;
    let stream_id = ParticipantStreamId::from_untyped(StableId128::from_bytes(stream_stable.0));
    let zone_id = ConflictZoneId::from_untyped(StableId128::from_bytes(zone_stable.0));
    let identity = world.binding.revision.identity();
    let stream = identity.ordinal(stream_id).ok_or(())?;
    let zone = identity.ordinal(zone_id).ok_or(())?;
    let address = world
        .conflict_read()
        .unique_address(zone, stream)
        .ok_or(())?;
    let locator = crate::ConflictPassageLocator::new(stream_id, zone_id);
    (world.conflict_passage_locator(address) == Some(locator))
        .then_some((locator, address))
        .ok_or(())
}

fn route_position_um(
    world: &TrafficWorld,
    route: RouteHandle,
    route_edge_index: u32,
    progress_mm: u32,
    carry_um: u16,
) -> Option<u128> {
    let edges = world.route_edges(route)?;
    let index = usize::try_from(route_edge_index).ok()?;
    let edge = *edges.get(index)?;
    let lengths = world.binding.revision.traffic().lane_lengths_millimetres();
    if progress_mm > *lengths.get(edge.index())? || carry_um >= MICROMETRES_PER_MILLIMETRE {
        return None;
    }
    let prefix_mm = edges[..index].iter().try_fold(0_u128, |sum, edge| {
        sum.checked_add(u128::from(*lengths.get(edge.index())?))
    })?;
    prefix_mm
        .checked_add(u128::from(progress_mm))?
        .checked_mul(u128::from(MICROMETRES_PER_MILLIMETRE))?
        .checked_add(u128::from(carry_um))
}

fn restore_conflict_aggregate(
    world: &mut TrafficWorld,
    root: wire::RuntimeSnapshot<'_>,
    vehicle_map: &BTreeMap<u64, VehicleHandle>,
) -> Result<(), SnapshotRestoreError> {
    let capacity = usize::try_from(world.binding.config.vehicle_capacity())
        .map_err(|_| SnapshotRestoreError::InvalidConflictHistory)?;
    let mut eligibility = Vec::new();
    eligibility
        .try_reserve_exact(capacity)
        .map_err(|_| SnapshotRestoreError::InvalidConflictHistory)?;
    eligibility.resize(capacity, None);
    world.committed.conflict_eligibility = eligibility;

    for vehicle in root.vehicles() {
        let snapshot_vehicle_id = vehicle.snapshot_vehicle_id();
        let handle = *vehicle_map.get(&snapshot_vehicle_id).ok_or(
            SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            },
        )?;
        if vehicle.conflict_eligibility().is_some() && vehicle.conflict_reservation().is_some() {
            return Err(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            });
        }
        if let Some(binding) = vehicle.conflict_eligibility() {
            let state = *world.vehicle_state(handle).ok_or(
                SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                },
            )?;
            if state.status != VehicleStatus::Active
                || world.conflict_reservation(handle).is_some()
                || binding.first_eligible_tick() > root.tick()
            {
                return Err(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                });
            }
            let (stable_locator, _) =
                decode_conflict_locator(world, binding.passage()).map_err(|_| {
                    SnapshotRestoreError::InvalidConflictAuthority {
                        snapshot_vehicle_id,
                    }
                })?;
            let locator = world
                .conflict_passage_occurrence_locator(
                    state.route,
                    binding.conflict_occurrence_index(),
                )
                .filter(|locator| {
                    locator.stable_locator() == stable_locator
                        && locator.maneuver_occurrence_index()
                            == binding.maneuver_occurrence_index()
                })
                .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                })?;
            let compiled = world.compiled_route(state.route).ok_or(
                SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                },
            )?;
            let maneuver = compiled
                .maneuvers
                .get(locator.maneuver_occurrence_index() as usize)
                .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                })?;
            let gate = compiled
                .hop_gate
                .get(locator.admission_gate_hop() as usize)
                .copied()
                .flatten()
                .and_then(|gate| world.binding.revision.identity().stable_id(gate))
                .map(|gate| *gate.as_untyped());
            if maneuver.entry_route_edge_index != binding.maneuver_entry_route_edge_index()
                || gate
                    != binding
                        .admission_gate()
                        .map(|gate| StableId128::from_bytes(gate.0))
            {
                return Err(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                });
            }
            let eligibility = crate::ConflictEligibilityState::update(
                None,
                locator,
                true,
                binding.first_eligible_tick(),
            )
            .expect("true predicate creates eligibility");
            if !world.conflict_eligibility_authority_valid(&state, eligibility) {
                return Err(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                });
            }
            world.committed.conflict_eligibility[handle.index() as usize] = Some(eligibility);
        }

        let Some(binding) = vehicle.conflict_reservation() else {
            continue;
        };
        let state =
            *world
                .vehicle_state(handle)
                .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                })?;
        let traversal =
            vehicle
                .maneuver_traversal()
                .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                })?;
        if state.status != VehicleStatus::Active
            || state.waiting_membership.is_some()
            || binding.acquired_tick() > root.tick()
            || traversal.phase().0 != wire::ManeuverTraversalPhaseKind::Clearing.0
            || traversal.maneuver_occurrence_index() != binding.maneuver_occurrence_index()
            || binding.passages().is_empty()
            || binding.downstream_intervals().is_empty()
        {
            return Err(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            });
        }
        let compiled = world.compiled_route(state.route).ok_or(
            SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            },
        )?;
        let maneuver = compiled
            .maneuvers
            .get(binding.maneuver_occurrence_index() as usize)
            .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        let path_stable = world
            .binding
            .revision
            .identity()
            .stable_id(maneuver.path)
            .map(|path| *path.as_untyped());
        let admission_gate =
            binding
                .admission_gate()
                .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                })?;
        let admission_gate_stable = StableId128::from_bytes(admission_gate.0);
        let admission_hop = (maneuver.entry_route_edge_index..maneuver.exit_route_edge_index)
            .find(|hop| {
                compiled
                    .hop_gate
                    .get(*hop as usize)
                    .copied()
                    .flatten()
                    .and_then(|gate| world.binding.revision.identity().stable_id(gate))
                    .is_some_and(|gate| *gate.as_untyped() == admission_gate_stable)
            })
            .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        if maneuver.entry_route_edge_index != binding.maneuver_entry_route_edge_index()
            || traversal
                .maneuver_path()
                .map(|path| StableId128::from_bytes(path.0))
                != path_stable
            || traversal
                .phase_gate()
                .map(|gate| StableId128::from_bytes(gate.0))
                != Some(admission_gate_stable)
        {
            return Err(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            });
        }

        let first_occurrence = binding.passages().get(0).conflict_occurrence_index();
        let passage_count = u32::try_from(binding.passages().len()).map_err(|_| {
            SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            }
        })?;
        let gate_range = compiled
            .conflict_gate_ranges
            .get(admission_hop as usize)
            .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        if gate_range.start != first_occurrence || gate_range.len != passage_count {
            return Err(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            });
        }
        let passage_range = crate::ConflictPassageRange::new(
            state.route,
            binding.maneuver_occurrence_index(),
            admission_hop,
            first_occurrence,
            passage_count,
        )
        .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
            snapshot_vehicle_id,
        })?;
        let gate_edge = compiled.edges.get(admission_hop as usize).copied().ok_or(
            SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            },
        )?;
        let gate_progress_mm = world
            .binding
            .revision
            .traffic()
            .lane_lengths_millimetres()
            .get(gate_edge.index())
            .copied()
            .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        let gate_um = route_position_um(world, state.route, admission_hop, gate_progress_mm, 0)
            .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        let front_um = route_position_um(
            world,
            state.route,
            state.route_edge_index,
            state.progress_mm,
            state.carry_um,
        )
        .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
            snapshot_vehicle_id,
        })?;
        if front_um < gate_um {
            return Err(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            });
        }
        let tail_um = i128::try_from(front_um).map_err(|_| {
            SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            }
        })? - i128::from(state.length_mm) * i128::from(MICROMETRES_PER_MILLIMETRE);
        let mut restored_cells = Vec::new();
        restored_cells
            .try_reserve_exact(binding.passages().len())
            .map_err(|_| SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        for (offset, row) in binding.passages().iter().enumerate() {
            if row.conflict_occurrence_index()
                != first_occurrence
                    .checked_add(u32::try_from(offset).map_err(|_| {
                        SnapshotRestoreError::InvalidConflictAuthority {
                            snapshot_vehicle_id,
                        }
                    })?)
                    .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                        snapshot_vehicle_id,
                    })?
            {
                return Err(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                });
            }
            let (stable_locator, address) =
                decode_conflict_locator(world, row.passage()).map_err(|_| {
                    SnapshotRestoreError::InvalidConflictAuthority {
                        snapshot_vehicle_id,
                    }
                })?;
            world
                .conflict_passage_occurrence_locator(state.route, row.conflict_occurrence_index())
                .filter(|locator| {
                    locator.stable_locator() == stable_locator
                        && locator.maneuver_occurrence_index()
                            == binding.maneuver_occurrence_index()
                        && locator.admission_gate_hop() == admission_hop
                })
                .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                })?;
            let occurrence = compiled
                .conflicts
                .get(row.conflict_occurrence_index() as usize)
                .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                })?;
            if occurrence.entry.route_edge_index != row.entry_route_edge_index()
                || occurrence.entry.progress_mm != row.entry_progress_mm()
                || occurrence.clearance.route_edge_index != row.clearance_route_edge_index()
                || occurrence.clearance.progress_mm != row.clearance_progress_mm()
            {
                return Err(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                });
            }
            let entry_um = route_position_um(
                world,
                state.route,
                occurrence.entry.route_edge_index,
                occurrence.entry.progress_mm,
                0,
            )
            .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
            let clearance_um = route_position_um(
                world,
                state.route,
                occurrence.clearance.route_edge_index,
                occurrence.clearance.progress_mm,
                0,
            )
            .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
            let entered = front_um >= entry_um;
            let cleared = tail_um
                >= i128::try_from(clearance_um).map_err(|_| {
                    SnapshotRestoreError::InvalidConflictAuthority {
                        snapshot_vehicle_id,
                    }
                })?;
            restored_cells.push(crate::kernel::conflict::RestoredConflictCell {
                address,
                occupant: entered && !cleared,
                cleared,
            });
        }
        restored_cells.sort_unstable_by_key(|cell| cell.address);
        if restored_cells
            .windows(2)
            .any(|pair| pair[0].address == pair[1].address)
        {
            return Err(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            });
        }

        let mut downstream = Vec::new();
        downstream
            .try_reserve_exact(binding.downstream_intervals().len())
            .map_err(|_| SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        let mut previous_wire_key = None;
        for row in binding.downstream_intervals() {
            let lane_edge =
                row.lane_edge()
                    .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                        snapshot_vehicle_id,
                    })?;
            let stable = StableId128::from_bytes(lane_edge.0);
            let wire_key = (stable, row.start_mm(), row.end_mm());
            if previous_wire_key.is_some_and(|previous| previous >= wire_key) {
                return Err(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                });
            }
            previous_wire_key = Some(wire_key);
            let edge = world
                .binding
                .revision
                .identity()
                .ordinal(LaneEdgeId::from_untyped(stable))
                .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                })?;
            let interval = crate::DownstreamInterval::new(edge, row.start_mm(), row.end_mm())
                .filter(|interval| {
                    interval.end_mm()
                        <= world.binding.revision.traffic().lane_lengths_millimetres()[edge.index()]
                })
                .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                    snapshot_vehicle_id,
                })?;
            downstream.push(interval);
        }
        downstream.sort_unstable();
        if downstream.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            });
        }
        let mut expected_downstream = Vec::new();
        let downstream_plan = world
            .reservation_downstream_claim_plan(passage_range, state.length_mm)
            .map_err(|_| SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        expected_downstream
            .try_reserve_exact(downstream_plan.raw_interval_capacity())
            .map_err(|_| SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        world
            .derive_reservation_downstream_claims_from_plan(
                downstream_plan,
                &mut expected_downstream,
            )
            .map_err(|_| SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        if downstream != expected_downstream {
            return Err(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            });
        }
        let follower_min_gap_mm = world
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(state.profile)
            .map(|profile| profile.min_gap_mm())
            .ok_or(SnapshotRestoreError::InvalidConflictAuthority {
                snapshot_vehicle_id,
            })?;
        crate::kernel::conflict::ConflictWrite::new(
            &mut world.committed.conflict,
            &mut world.derived.conflict,
            &mut world.workspace.conflict,
        )
        .restore_reservation(
            handle,
            crate::kernel::conflict::RestoredConflictReservation {
                follower_min_gap_mm,
                acquired_tick: binding.acquired_tick(),
                passage_range,
                cells: &restored_cells,
                downstream: &downstream,
            },
        )
        .map_err(|_| SnapshotRestoreError::InvalidConflictAuthority {
            snapshot_vehicle_id,
        })?;
        world.committed.vehicles[handle.index() as usize]
            .state
            .as_mut()
            .expect("restored vehicle exists")
            .maneuver_traversal = Some(ManeuverTraversalState {
            route: state.route,
            maneuver_occurrence_index: binding.maneuver_occurrence_index(),
            phase: ManeuverTraversalPhase::Clearing {
                admission_gate_hop: admission_hop,
            },
        });
    }

    let mut previous_locator = None;
    for row in root.conflict_lag_states() {
        let (locator, address) = decode_conflict_locator(world, row.passage())
            .map_err(|_| SnapshotRestoreError::InvalidConflictHistory)?;
        let key = (
            *locator.participant_stream_stable_id().as_untyped(),
            *locator.conflict_zone_stable_id().as_untyped(),
        );
        if previous_locator.is_some_and(|previous| previous >= key) {
            return Err(SnapshotRestoreError::InvalidConflictHistory);
        }
        previous_locator = Some(key);
        let time = row.reference_time_ms();
        if time > root.time_ms() {
            return Err(SnapshotRestoreError::InvalidConflictHistory);
        }
        let reference = if row.reference_kind().0 == wire::ConflictLagReferenceKind::ActualClear.0 {
            crate::ConflictLagReference::ActualClear(time)
        } else if row.reference_kind().0 == wire::ConflictLagReferenceKind::CutoverFloor.0 {
            crate::ConflictLagReference::CutoverFloor(time)
        } else {
            return Err(SnapshotRestoreError::InvalidConflictHistory);
        };
        crate::kernel::conflict::ConflictWrite::new(
            &mut world.committed.conflict,
            &mut world.derived.conflict,
            &mut world.workspace.conflict,
        )
        .restore_lag_reference(address, reference)
        .map_err(|_| SnapshotRestoreError::InvalidConflictHistory)?;
    }
    world.normalize_conflict_eligibility();
    if !world.conflict_state_valid() {
        return Err(SnapshotRestoreError::InvalidConflictHistory);
    }
    Ok(())
}

fn restore_waiting_aggregate(
    world: &mut TrafficWorld,
    root: wire::RuntimeSnapshot<'_>,
) -> Result<(), SnapshotRestoreError> {
    let mut rows = vec![None; world.committed.waiting_zones.len()];
    for row in root.waiting_zones() {
        let zone = row
            .waiting_zone()
            .and_then(|stable| {
                world
                    .binding
                    .revision
                    .identity()
                    .ordinal(WaitingZoneId::from_untyped(StableId128::from_bytes(
                        stable.0,
                    )))
            })
            .ok_or(SnapshotRestoreError::InvalidWaitingZoneState)?;
        if (row.occupancy() == 0 && row.next_admission_sequence() == 0)
            || rows[zone.index()]
                .replace((row.occupancy(), row.next_admission_sequence()))
                .is_some()
        {
            return Err(SnapshotRestoreError::InvalidWaitingZoneState);
        }
    }

    let mut members = Vec::new();
    members
        .try_reserve_exact(world.committed.live_order.len())
        .map_err(|_| SnapshotRestoreError::WaitingInvariantViolation)?;
    for vehicle in world.committed.live_order.iter().copied() {
        if let Some(membership) = world
            .vehicle_state(vehicle)
            .and_then(|state| state.waiting_membership)
        {
            members.push((
                membership.waiting_zone.index(),
                membership.admission_sequence,
                vehicle,
                membership,
            ));
        }
    }
    members.sort_by_key(|(zone, sequence, vehicle, _)| {
        (*zone, *sequence, vehicle.index(), vehicle.generation())
    });
    if members
        .windows(2)
        .any(|pair| pair[0].0 == pair[1].0 && pair[0].1 == pair[1].1)
    {
        return Err(SnapshotRestoreError::WaitingInvariantViolation);
    }

    // 成员已经按 zone 排序；单调游标只消费当前组，计数总成本为 O(zones + members)。
    let mut member_cursor = 0;
    for (zone_index, state) in world.committed.waiting_zones.iter_mut().enumerate() {
        let group_start = member_cursor;
        while members
            .get(member_cursor)
            .is_some_and(|member| member.0 == zone_index)
        {
            member_cursor += 1;
        }
        let member_count = member_cursor - group_start;
        let Some((occupancy, next_admission_sequence)) = rows[zone_index] else {
            if member_count != 0 {
                return Err(SnapshotRestoreError::WaitingInvariantViolation);
            }
            continue;
        };
        let zone = WaitingZoneOrdinal::from_raw(
            u32::try_from(zone_index)
                .map_err(|_| SnapshotRestoreError::WaitingInvariantViolation)?,
        );
        let max_occupancy = world
            .binding
            .revision
            .traffic()
            .relations()
            .waiting_zone(zone)
            .ok_or(SnapshotRestoreError::InvalidWaitingZoneState)?
            .max_occupancy();
        if usize::try_from(occupancy).ok() != Some(member_count)
            || occupancy > max_occupancy
            || (member_count != 0 && next_admission_sequence == 0)
        {
            return Err(SnapshotRestoreError::WaitingInvariantViolation);
        }
        state.next_admission_sequence = next_admission_sequence;
    }

    for (_, sequence, vehicle, membership) in members {
        let next =
            world.committed.waiting_zones[membership.waiting_zone.index()].next_admission_sequence;
        if sequence >= next {
            return Err(SnapshotRestoreError::WaitingInvariantViolation);
        }
        world.append_waiting_member(vehicle, membership);
    }
    if !world.waiting_state_valid() || !world.waiting_snapshot_storage_valid() {
        return Err(SnapshotRestoreError::WaitingInvariantViolation);
    }
    world.rebuild_waiting_member_rows();
    world
        .prepare_waiting_dependencies(false)
        .map_err(SnapshotRestoreError::WaitingDependencyRebuild)?;
    Ok(())
}

fn verify_lfrs<'a>(
    bytes: &'a [u8],
    limits: SnapshotRestoreLimits,
) -> Result<wire::RuntimeSnapshot<'a>, SnapshotRestoreError> {
    let byte_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if byte_len > limits.max_wire_bytes() {
        return Err(limit_error(
            SnapshotLimitDimension::WireBytes,
            limits.max_wire_bytes(),
            byte_len,
        ));
    }
    if bytes.len() < MIN_SIZE_PREFIXED_LFRS_BYTES {
        return Err(SnapshotRestoreError::TruncatedFraming);
    }
    let declared = u32::from_le_bytes(
        bytes[..4]
            .try_into()
            .expect("minimum framing includes size prefix"),
    );
    let actual = u64::try_from(bytes.len() - 4).unwrap_or(u64::MAX);
    if u64::from(declared) != actual {
        return Err(SnapshotRestoreError::SizePrefixMismatch {
            declared: u64::from(declared),
            actual,
        });
    }
    if !wire::runtime_snapshot_size_prefixed_buffer_has_identifier(bytes) {
        return Err(SnapshotRestoreError::FileIdentifierMismatch);
    }

    // canonical FlatBuffers 中每个 table object 至少占一个 4-byte soffset；以实际
    // caller-bounded wire 长度给 verifier 线性表预算，既覆盖 Conflict locator/
    // reservation 的可变嵌套表，也会拒绝利用共享子树造成超线性重复访问的输入。
    let max_tables = bytes.len() / std::mem::size_of::<u32>();
    let max_apparent_size = bytes
        .len()
        .checked_mul(APPARENT_SIZE_MULTIPLIER)
        .ok_or_else(|| {
            limit_error(
                SnapshotLimitDimension::VerifierBudget,
                usize::MAX as u64,
                u64::MAX,
            )
        })?;
    let options = VerifierOptions {
        max_depth: MAX_SCHEMA_TABLE_DEPTH,
        max_tables,
        max_apparent_size,
        ignore_missing_null_terminator: false,
    };
    wire::size_prefixed_root_as_runtime_snapshot_with_opts(&options, bytes)
        .map_err(|_| SnapshotRestoreError::InvalidFlatbuffer)
}

fn validate_bindings(
    root: wire::RuntimeSnapshot<'_>,
    revision: &SharedNetworkRevision,
    source: &CommittedNetworkSource,
    target_config: WorldConfig,
    limits: SnapshotRestoreLimits,
) -> Result<(), SnapshotRestoreError> {
    if root.format_version() != SNAPSHOT_FORMAT_VERSION {
        return Err(SnapshotRestoreError::UnsupportedFormatVersion {
            actual: root.format_version(),
        });
    }
    if root.runtime_state_version() != RUNTIME_STATE_VERSION {
        return Err(SnapshotRestoreError::UnsupportedRuntimeStateVersion {
            actual: root.runtime_state_version(),
        });
    }
    validate_closed_v5_tables(root)?;
    let network_revision = root
        .network_revision()
        .ok_or(SnapshotRestoreError::MissingField {
            field: "network_revision",
        })?;
    if network_revision.0 != *revision.network_revision().as_digest().as_bytes() {
        return Err(SnapshotRestoreError::NetworkRevisionMismatch);
    }
    root.lfca_artifact_digest()
        .ok_or(SnapshotRestoreError::MissingField {
            field: "lfca_artifact_digest",
        })?;
    let contracts = root
        .static_contract_versions()
        .ok_or(SnapshotRestoreError::MissingField {
            field: "static_contract_versions",
        })?;
    let target_contracts = revision.canonical_origin().static_contract_versions();
    if contracts.canonical_format_version() != target_contracts.canonical_format_version()
        || contracts.identity_encoding_version() != target_contracts.identity_encoding_version()
        || contracts.identity_registry_revision() != target_contracts.identity_registry_revision()
        || contracts.network_revision_derivation_version()
            != target_contracts.network_revision_derivation_version()
        || contracts.constraint_contract_version() != target_contracts.constraint_contract_version()
        || contracts.static_execution_contract_version()
            != target_contracts.static_execution_contract_version()
    {
        return Err(SnapshotRestoreError::StaticContractVersionsMismatch);
    }
    if source.network_revision() != revision.network_revision() {
        return Err(SnapshotRestoreError::Install(
            InstallError::SourceRevisionMismatch {
                source_revision: source.network_revision(),
                installed_revision: revision.network_revision(),
            },
        ));
    }

    if root.source_kind() != wire::SourceKind::Published {
        return Err(SnapshotRestoreError::UnsupportedSourceKind {
            actual: root.source_kind().0,
        });
    }
    let published = root
        .source_published()
        .ok_or(SnapshotRestoreError::MissingField {
            field: "source_published",
        })?;
    if published.asset_key().is_empty() {
        return Err(SnapshotRestoreError::EmptyAssetKey);
    }
    let asset_key_len = u64::try_from(published.asset_key().len()).unwrap_or(u64::MAX);
    if asset_key_len > limits.max_asset_key_bytes() {
        return Err(limit_error(
            SnapshotLimitDimension::AssetKeyBytes,
            limits.max_asset_key_bytes(),
            asset_key_len,
        ));
    }
    published
        .artifact_digest()
        .ok_or(SnapshotRestoreError::MissingField {
            field: "source_published.artifact_digest",
        })?;
    let source_revision =
        published
            .network_revision()
            .ok_or(SnapshotRestoreError::MissingField {
                field: "source_published.network_revision",
            })?;
    if source_revision.0 != network_revision.0 {
        return Err(SnapshotRestoreError::SnapshotSourceRevisionMismatch);
    }

    let snapshot_config = root.world_config();
    if snapshot_config.fixed_delta_time_ms() != target_config.fixed_delta_time_ms() {
        return Err(SnapshotRestoreError::FixedDeltaTimeMismatch {
            snapshot: snapshot_config.fixed_delta_time_ms(),
            target: target_config.fixed_delta_time_ms(),
        });
    }
    validate_capacity_not_smaller(
        SnapshotLimitDimension::Vehicles,
        u64::from(snapshot_config.vehicle_capacity()),
        u64::from(target_config.vehicle_capacity()),
    )?;
    validate_capacity_not_smaller(
        SnapshotLimitDimension::Routes,
        u64::from(snapshot_config.route_capacity()),
        u64::from(target_config.route_capacity()),
    )?;
    validate_capacity_not_smaller(
        SnapshotLimitDimension::RouteEdgeOccurrences,
        snapshot_config.route_edge_occurrence_capacity(),
        target_config.route_edge_occurrence_capacity(),
    )?;
    validate_capacity_not_smaller(
        SnapshotLimitDimension::RouteConflictOccurrences,
        snapshot_config.route_conflict_occurrence_capacity(),
        target_config.route_conflict_occurrence_capacity(),
    )?;
    let expected_time = root
        .tick()
        .checked_mul(snapshot_config.fixed_delta_time_ms())
        .ok_or(SnapshotRestoreError::InvalidClock)?;
    if expected_time != root.time_ms() {
        return Err(SnapshotRestoreError::InvalidClock);
    }

    let route_count = u64::try_from(root.routes().len()).unwrap_or(u64::MAX);
    validate_state_count(
        SnapshotLimitDimension::Routes,
        route_count,
        u64::from(snapshot_config.route_capacity()),
        u64::from(target_config.route_capacity()),
    )?;
    let vehicle_count = u64::try_from(root.vehicles().len()).unwrap_or(u64::MAX);
    validate_state_count(
        SnapshotLimitDimension::Vehicles,
        vehicle_count,
        u64::from(snapshot_config.vehicle_capacity()),
        u64::from(target_config.vehicle_capacity()),
    )?;
    if root.live_order().len() != root.vehicles().len() {
        return Err(SnapshotRestoreError::IncompleteLiveOrder);
    }
    let mut occurrence_count = 0_u64;
    for route in root.routes() {
        occurrence_count = occurrence_count
            .checked_add(u64::try_from(route.edges().len()).unwrap_or(u64::MAX))
            .ok_or_else(|| {
                limit_error(
                    SnapshotLimitDimension::RouteEdgeOccurrences,
                    snapshot_config.route_edge_occurrence_capacity(),
                    u64::MAX,
                )
            })?;
    }
    validate_state_count(
        SnapshotLimitDimension::RouteEdgeOccurrences,
        occurrence_count,
        snapshot_config.route_edge_occurrence_capacity(),
        target_config.route_edge_occurrence_capacity(),
    )?;
    Ok(())
}

fn decode_world_policy(
    binding: wire::WorldPolicyBinding<'_>,
) -> Result<crate::WorldPolicySelection, SnapshotRestoreError> {
    match (binding.selection(), binding.policy()) {
        (wire::WorldPolicySelectionKind::NotRequired, None) => {
            Ok(crate::WorldPolicySelection::NotRequired)
        }
        (wire::WorldPolicySelectionKind::Pinned, Some(id)) => {
            Ok(crate::WorldPolicySelection::Pinned(crate::PolicyPin {
                policy: laneflow_static_contract::RightOfWayPolicySetId::from_untyped(
                    StableId128::from_bytes(id.0),
                ),
            }))
        }
        _ => Err(SnapshotRestoreError::InvalidPolicyBinding),
    }
}

fn validate_closed_v5_tables(root: wire::RuntimeSnapshot<'_>) -> Result<(), SnapshotRestoreError> {
    validate_table_field_count("RuntimeSnapshot", root._tab, ROOT_V5_FIELDS)?;
    validate_table_field_count(
        "WorldPolicyBinding",
        root.world_policy()._tab,
        vtable_field_count(wire::WorldPolicyBinding::VT_POLICY),
    )?;
    validate_table_field_count(
        "WorldConfigBinding",
        root.world_config()._tab,
        WORLD_CONFIG_V5_FIELDS,
    )?;
    if let Some(published) = root.source_published() {
        validate_table_field_count(
            "PublishedSourceBinding",
            published._tab,
            PUBLISHED_SOURCE_V5_FIELDS,
        )?;
    }
    for route in root.routes() {
        validate_table_field_count("SnapshotRoute", route._tab, ROUTE_V5_FIELDS)?;
    }
    for vehicle in root.vehicles() {
        validate_table_field_count("SnapshotVehicle", vehicle._tab, VEHICLE_V5_FIELDS)?;
        if let Some(parking) = vehicle.parking() {
            validate_table_field_count("ParkingBinding", parking._tab, PARKING_BINDING_V5_FIELDS)?;
        }
        if let Some(traversal) = vehicle.maneuver_traversal() {
            validate_table_field_count(
                "ManeuverTraversalBinding",
                traversal._tab,
                MANEUVER_TRAVERSAL_V5_FIELDS,
            )?;
        }
        if let Some(membership) = vehicle.waiting_membership() {
            validate_table_field_count(
                "WaitingMembershipBinding",
                membership._tab,
                WAITING_MEMBERSHIP_V5_FIELDS,
            )?;
        }
        if let Some(eligibility) = vehicle.conflict_eligibility() {
            validate_table_field_count(
                "ConflictEligibilityBinding",
                eligibility._tab,
                CONFLICT_ELIGIBILITY_V5_FIELDS,
            )?;
            validate_conflict_locator_table(eligibility.passage())?;
        }
        if let Some(reservation) = vehicle.conflict_reservation() {
            validate_table_field_count(
                "ConflictReservationBinding",
                reservation._tab,
                CONFLICT_RESERVATION_V5_FIELDS,
            )?;
            for passage in reservation.passages() {
                validate_table_field_count(
                    "ConflictPassageBinding",
                    passage._tab,
                    CONFLICT_PASSAGE_V5_FIELDS,
                )?;
                validate_conflict_locator_table(passage.passage())?;
            }
            for downstream in reservation.downstream_intervals() {
                validate_table_field_count(
                    "ConflictDownstreamIntervalBinding",
                    downstream._tab,
                    CONFLICT_DOWNSTREAM_V5_FIELDS,
                )?;
            }
        }
    }
    for state in root.waiting_zones() {
        validate_table_field_count("WaitingZoneState", state._tab, WAITING_ZONE_STATE_V5_FIELDS)?;
    }
    for state in root.conflict_lag_states() {
        validate_table_field_count("ConflictLagState", state._tab, CONFLICT_LAG_STATE_V5_FIELDS)?;
        validate_conflict_locator_table(state.passage())?;
    }
    Ok(())
}

fn validate_conflict_locator_table(
    locator: wire::ConflictPassageLocatorBinding<'_>,
) -> Result<(), SnapshotRestoreError> {
    validate_table_field_count(
        "ConflictPassageLocatorBinding",
        locator._tab,
        CONFLICT_LOCATOR_V5_FIELDS,
    )
}

fn validate_table_field_count(
    table_name: &'static str,
    table: laneflow_runtime_snapshot_wire::runtime::Table<'_>,
    supported: usize,
) -> Result<(), SnapshotRestoreError> {
    let actual = table.vtable().num_fields();
    if actual > supported {
        return Err(SnapshotRestoreError::UnknownTableFields {
            table: table_name,
            supported,
            actual,
        });
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum DecodedParkingBinding {
    Reserved(ReserveParkingTarget),
    Occupied(ParkingTarget),
}

#[derive(Clone, Copy, Default)]
struct DecodedWaitingAuthority {
    traversal: Option<ManeuverTraversalState>,
    membership: Option<WaitingMembership>,
}

fn decode_waiting_authority(
    world: &TrafficWorld,
    vehicle: wire::SnapshotVehicle<'_>,
    status: VehicleStatus,
    route: RouteHandle,
) -> Result<DecodedWaitingAuthority, SnapshotRestoreError> {
    let snapshot_vehicle_id = vehicle.snapshot_vehicle_id();
    if status != VehicleStatus::Active {
        return if vehicle.maneuver_traversal().is_none()
            && vehicle.waiting_membership().is_none()
            && vehicle.conflict_eligibility().is_none()
            && vehicle.conflict_reservation().is_none()
        {
            Ok(DecodedWaitingAuthority::default())
        } else {
            Err(SnapshotRestoreError::InvalidWaitingAuthority {
                snapshot_vehicle_id,
            })
        };
    }

    let Some(binding) = vehicle.maneuver_traversal() else {
        return if vehicle.waiting_membership().is_none() && vehicle.conflict_reservation().is_none()
        {
            Ok(DecodedWaitingAuthority::default())
        } else {
            Err(SnapshotRestoreError::InvalidWaitingAuthority {
                snapshot_vehicle_id,
            })
        };
    };
    let clearing = binding.phase().0 == wire::ManeuverTraversalPhaseKind::Clearing.0;
    if clearing
        && (vehicle.waiting_membership().is_some() || vehicle.conflict_reservation().is_none())
    {
        return Err(SnapshotRestoreError::InvalidConflictAuthority {
            snapshot_vehicle_id,
        });
    }
    if !clearing && vehicle.conflict_reservation().is_some() {
        return Err(SnapshotRestoreError::InvalidConflictAuthority {
            snapshot_vehicle_id,
        });
    }
    let compiled =
        world
            .compiled_route(route)
            .ok_or(SnapshotRestoreError::InvalidWaitingAuthority {
                snapshot_vehicle_id,
            })?;
    let path = binding
        .maneuver_path()
        .and_then(|stable| {
            world
                .binding
                .revision
                .identity()
                .ordinal(ManeuverPathId::from_untyped(StableId128::from_bytes(
                    stable.0,
                )))
        })
        .ok_or(SnapshotRestoreError::InvalidWaitingAuthority {
            snapshot_vehicle_id,
        })?;
    let phase_gate = binding
        .phase_gate()
        .and_then(|stable| {
            world
                .binding
                .revision
                .identity()
                .ordinal(ManeuverGateId::from_untyped(StableId128::from_bytes(
                    stable.0,
                )))
        })
        .ok_or(SnapshotRestoreError::InvalidWaitingAuthority {
            snapshot_vehicle_id,
        })?;
    let anchor = world
        .resolve_maneuver_anchor(
            route,
            crate::kernel::world::ManeuverOccurrenceAnchor::OccurrenceIndex(
                binding.maneuver_occurrence_index(),
            ),
            path,
            phase_gate,
        )
        .ok_or(SnapshotRestoreError::InvalidWaitingAuthority {
            snapshot_vehicle_id,
        })?;
    let phase_hop = anchor.gate_hop;
    let phase = if binding.phase().0 == wire::ManeuverTraversalPhaseKind::PreGate.0 {
        ManeuverTraversalPhase::PreGate {
            next_gate_hop: phase_hop,
        }
    } else if binding.phase().0 == wire::ManeuverTraversalPhaseKind::Committed.0 {
        ManeuverTraversalPhase::Committed {
            last_crossed_gate_hop: phase_hop,
        }
    } else if binding.phase().0 == wire::ManeuverTraversalPhaseKind::Waiting.0 {
        ManeuverTraversalPhase::Waiting {
            release_gate_hop: phase_hop,
        }
    } else if clearing {
        ManeuverTraversalPhase::Clearing {
            admission_gate_hop: phase_hop,
        }
    } else {
        return Err(SnapshotRestoreError::InvalidWaitingAuthority {
            snapshot_vehicle_id,
        });
    };
    let traversal = ManeuverTraversalState {
        route,
        maneuver_occurrence_index: anchor.occurrence_index,
        phase,
    };

    let membership = match vehicle.waiting_membership() {
        None => None,
        Some(binding) => {
            if binding.maneuver_occurrence_index() != traversal.maneuver_occurrence_index {
                return Err(SnapshotRestoreError::InvalidWaitingAuthority {
                    snapshot_vehicle_id,
                });
            }
            let zone = binding
                .waiting_zone()
                .and_then(|stable| {
                    world
                        .binding
                        .revision
                        .identity()
                        .ordinal(WaitingZoneId::from_untyped(StableId128::from_bytes(
                            stable.0,
                        )))
                })
                .ok_or(SnapshotRestoreError::InvalidWaitingAuthority {
                    snapshot_vehicle_id,
                })?;
            let entry_gate = binding
                .entry_gate()
                .and_then(|stable| {
                    world
                        .binding
                        .revision
                        .identity()
                        .ordinal(ManeuverGateId::from_untyped(StableId128::from_bytes(
                            stable.0,
                        )))
                })
                .ok_or(SnapshotRestoreError::InvalidWaitingAuthority {
                    snapshot_vehicle_id,
                })?;
            let release_gate = binding
                .release_gate()
                .and_then(|stable| {
                    world
                        .binding
                        .revision
                        .identity()
                        .ordinal(ManeuverGateId::from_untyped(StableId128::from_bytes(
                            stable.0,
                        )))
                })
                .ok_or(SnapshotRestoreError::InvalidWaitingAuthority {
                    snapshot_vehicle_id,
                })?;
            let occurrence = compiled
                .waiting
                .iter()
                .find(|occurrence| {
                    occurrence.maneuver_index == traversal.maneuver_occurrence_index
                        && occurrence.zone == zone
                        && compiled
                            .hop_gate
                            .get(occurrence.entry_hop as usize)
                            .copied()
                            .flatten()
                            == Some(entry_gate)
                        && compiled
                            .hop_gate
                            .get(occurrence.release_hop as usize)
                            .copied()
                            .flatten()
                            == Some(release_gate)
                })
                .ok_or(SnapshotRestoreError::InvalidWaitingAuthority {
                    snapshot_vehicle_id,
                })?;
            Some(WaitingMembership {
                waiting_zone: zone,
                admission_sequence: binding.admission_sequence(),
                release_hop: occurrence.release_hop,
            })
        }
    };
    Ok(DecodedWaitingAuthority {
        traversal: Some(traversal),
        membership,
    })
}

fn decode_parking_binding(
    world: &TrafficWorld,
    vehicle: wire::SnapshotVehicle<'_>,
    status: VehicleStatus,
) -> Result<Option<DecodedParkingBinding>, SnapshotRestoreError> {
    let snapshot_vehicle_id = vehicle.snapshot_vehicle_id();
    let Some(binding) = vehicle.parking() else {
        return if status == VehicleStatus::Parked {
            Err(SnapshotRestoreError::ParkingStatusMismatch {
                snapshot_vehicle_id,
            })
        } else {
            Ok(None)
        };
    };
    let state = if binding.state() == wire::ParkingBindingStateKind::Reserved {
        wire::ParkingBindingStateKind::Reserved
    } else if binding.state() == wire::ParkingBindingStateKind::Occupied {
        wire::ParkingBindingStateKind::Occupied
    } else {
        return Err(SnapshotRestoreError::InvalidParkingBindingState {
            snapshot_vehicle_id,
            actual: binding.state().0,
        });
    };
    let target_wire = binding.target().ok_or(SnapshotRestoreError::MissingField {
        field: "vehicles.parking.target",
    })?;
    let identity = world.binding.revision.identity();
    let target = if binding.target_kind() == wire::ParkingTargetKind::ExplicitSpace {
        ParkingTarget::ExplicitSpace(
            identity
                .ordinal(ParkingSpaceId::from_untyped(StableId128::from_bytes(
                    target_wire.0,
                )))
                .ok_or(SnapshotRestoreError::UnknownParkingSpace {
                    snapshot_vehicle_id,
                })?,
        )
    } else if binding.target_kind() == wire::ParkingTargetKind::VirtualPool {
        ParkingTarget::VirtualPool(
            identity
                .ordinal(ParkingFacilityId::from_untyped(StableId128::from_bytes(
                    target_wire.0,
                )))
                .ok_or(SnapshotRestoreError::UnknownParkingFacility {
                    snapshot_vehicle_id,
                })?,
        )
    } else {
        return Err(SnapshotRestoreError::InvalidParkingTargetKind {
            snapshot_vehicle_id,
            actual: binding.target_kind().0,
        });
    };

    match (state, target, status) {
        (
            wire::ParkingBindingStateKind::Reserved,
            ParkingTarget::ExplicitSpace(space),
            VehicleStatus::Active,
        ) => {
            if binding.virtual_entry_edge().is_some() || binding.virtual_entry_progress_mm() != 0 {
                return Err(SnapshotRestoreError::InvalidParkingBindingShape {
                    snapshot_vehicle_id,
                });
            }
            Ok(Some(DecodedParkingBinding::Reserved(
                ReserveParkingTarget::ExplicitSpace {
                    space,
                    entry_route_occurrence: binding.entry_route_occurrence(),
                },
            )))
        }
        (
            wire::ParkingBindingStateKind::Reserved,
            ParkingTarget::VirtualPool(facility),
            VehicleStatus::Active,
        ) => {
            let entry_wire = binding.virtual_entry_edge().ok_or(
                SnapshotRestoreError::InvalidParkingBindingShape {
                    snapshot_vehicle_id,
                },
            )?;
            let entry_edge = identity
                .ordinal(LaneEdgeId::from_untyped(StableId128::from_bytes(
                    entry_wire.0,
                )))
                .ok_or(SnapshotRestoreError::UnknownVirtualParkingEntry {
                    snapshot_vehicle_id,
                })?;
            let view = world
                .binding
                .revision
                .traffic()
                .relations()
                .parking_facility(facility)
                .ok_or(SnapshotRestoreError::UnknownParkingFacility {
                    snapshot_vehicle_id,
                })?;
            let selector = view
                .virtual_entries()
                .iter()
                .position(|anchor| {
                    anchor.lane_edge() == entry_edge
                        && anchor.progress_mm() == binding.virtual_entry_progress_mm()
                })
                .ok_or(SnapshotRestoreError::UnknownVirtualParkingEntry {
                    snapshot_vehicle_id,
                })?;
            Ok(Some(DecodedParkingBinding::Reserved(
                ReserveParkingTarget::VirtualPool {
                    facility,
                    entry_anchor: VirtualEntryAnchorSelector::from_raw(
                        u32::try_from(selector).expect("virtual entry selector fits u32"),
                    ),
                    entry_route_occurrence: binding.entry_route_occurrence(),
                },
            )))
        }
        (wire::ParkingBindingStateKind::Occupied, target, VehicleStatus::Parked) => {
            if binding.entry_route_occurrence() != 0
                || binding.virtual_entry_edge().is_some()
                || binding.virtual_entry_progress_mm() != 0
            {
                return Err(SnapshotRestoreError::InvalidParkingBindingShape {
                    snapshot_vehicle_id,
                });
            }
            Ok(Some(DecodedParkingBinding::Occupied(target)))
        }
        _ => Err(SnapshotRestoreError::ParkingStatusMismatch {
            snapshot_vehicle_id,
        }),
    }
}

fn restore_vehicle(
    world: &mut TrafficWorld,
    vehicle: wire::SnapshotVehicle<'_>,
    status: VehicleStatus,
    route_map: &BTreeMap<u64, RouteHandle>,
    vehicle_map: &mut BTreeMap<u64, VehicleHandle>,
) -> Result<(), SnapshotRestoreError> {
    let snapshot_vehicle_id = vehicle.snapshot_vehicle_id();
    if snapshot_vehicle_id == 0 {
        return Err(SnapshotRestoreError::ZeroVehicleId);
    }
    if vehicle_map.contains_key(&snapshot_vehicle_id) {
        return Err(SnapshotRestoreError::DuplicateVehicleId {
            snapshot_vehicle_id,
        });
    }
    let snapshot_route_id = vehicle.snapshot_route_id();
    let Some(route) = route_map.get(&snapshot_route_id).copied() else {
        return Err(SnapshotRestoreError::UnknownRouteReference {
            snapshot_vehicle_id,
            snapshot_route_id,
        });
    };
    if vehicle.carry_um() >= MICROMETRES_PER_MILLIMETRE {
        return Err(SnapshotRestoreError::CarryOutOfRange {
            snapshot_vehicle_id,
            actual: vehicle.carry_um(),
        });
    }
    let parking = decode_parking_binding(world, vehicle, status)?;
    if status != VehicleStatus::Active && (vehicle.speed_mm_s() != 0 || vehicle.carry_um() != 0) {
        return Err(SnapshotRestoreError::InvalidInactiveMotion {
            snapshot_vehicle_id,
        });
    }

    let profile = vehicle
        .profile()
        .ok_or(SnapshotRestoreError::MissingField {
            field: "vehicles.profile",
        })?;
    let class = vehicle.class().ok_or(SnapshotRestoreError::MissingField {
        field: "vehicles.class",
    })?;
    let identity = world.binding.revision.identity();
    let profile = identity
        .ordinal(VehicleProfileId::from_untyped(StableId128::from_bytes(
            profile.0,
        )))
        .ok_or(SnapshotRestoreError::UnknownVehicleProfile {
            snapshot_vehicle_id,
        })?;
    let class = identity
        .ordinal(ParticipantClassId::from_untyped(StableId128::from_bytes(
            class.0,
        )))
        .ok_or(SnapshotRestoreError::UnknownParticipantClass {
            snapshot_vehicle_id,
        })?;
    let profile_view = world
        .binding
        .revision
        .traffic()
        .relations()
        .vehicle_profile(profile)
        .expect("identity ordinal resolves profile row");
    if profile_view.class() != class {
        return Err(SnapshotRestoreError::ProfileClassMismatch {
            snapshot_vehicle_id,
        });
    }
    let waiting = decode_waiting_authority(world, vehicle, status, route)?;

    if status == VehicleStatus::Completed {
        let edges = world
            .route_edges(route)
            .expect("restored route handle remains live");
        let index = usize::try_from(vehicle.route_edge_index()).unwrap_or(usize::MAX);
        let at_route_end = edges.get(index).is_some_and(|edge| {
            index + 1 == edges.len()
                && vehicle.progress_mm()
                    == world.binding.revision.traffic().lane_lengths_millimetres()[edge.index()]
        });
        if !at_route_end {
            return Err(SnapshotRestoreError::InvalidCompletedState {
                snapshot_vehicle_id,
            });
        }
    }

    let handle = if let Some(DecodedParkingBinding::Occupied(target)) = parking {
        world
            .spawn_parked_vehicle(
                ParkedVehicleSpawnInput::new(
                    profile,
                    route,
                    vehicle.route_edge_index(),
                    vehicle.progress_mm(),
                ),
                target,
            )
            .map_err(|error| SnapshotRestoreError::Parking {
                snapshot_vehicle_id,
                error,
            })?
            .vehicle
    } else {
        world
            .restore_unparked_vehicle(
                VehicleSpawnInput::new(
                    profile,
                    route,
                    vehicle.route_edge_index(),
                    vehicle.progress_mm(),
                    vehicle.speed_mm_s(),
                ),
                vehicle.carry_um(),
                status,
                waiting.traversal,
                waiting.membership,
                vehicle.conflict_eligibility().is_some()
                    || vehicle.conflict_reservation().is_some(),
            )
            .map_err(|error| SnapshotRestoreError::Vehicle {
                snapshot_vehicle_id,
                error,
            })?
    };
    match status {
        VehicleStatus::Active => {
            if let Some(DecodedParkingBinding::Reserved(target)) = parking {
                world.reserve_parking(handle, target).map_err(|error| {
                    SnapshotRestoreError::Parking {
                        snapshot_vehicle_id,
                        error,
                    }
                })?;
            }
        }
        VehicleStatus::Parked => {}
        VehicleStatus::Completed => {}
    }
    if !world
        .vehicle_state(handle)
        .copied()
        .is_some_and(|state| world.restored_waiting_authority_valid(state))
    {
        return Err(SnapshotRestoreError::InvalidWaitingAuthority {
            snapshot_vehicle_id,
        });
    }
    vehicle_map.insert(snapshot_vehicle_id, handle);
    Ok(())
}

const fn decode_vehicle_status(
    snapshot_vehicle_id: u64,
    status: wire::VehicleStatusKind,
) -> Result<VehicleStatus, SnapshotRestoreError> {
    if status.0 == wire::VehicleStatusKind::Active.0 {
        Ok(VehicleStatus::Active)
    } else if status.0 == wire::VehicleStatusKind::Parked.0 {
        Ok(VehicleStatus::Parked)
    } else if status.0 == wire::VehicleStatusKind::Completed.0 {
        Ok(VehicleStatus::Completed)
    } else {
        Err(SnapshotRestoreError::InvalidVehicleStatus {
            snapshot_vehicle_id,
            actual: status.0,
        })
    }
}

const fn validate_capacity_not_smaller(
    dimension: SnapshotLimitDimension,
    snapshot: u64,
    target: u64,
) -> Result<(), SnapshotRestoreError> {
    if target < snapshot {
        return Err(SnapshotRestoreError::TargetCapacitySmaller {
            dimension,
            snapshot,
            target,
        });
    }
    Ok(())
}

const fn validate_state_count(
    dimension: SnapshotLimitDimension,
    actual: u64,
    snapshot_limit: u64,
    target_limit: u64,
) -> Result<(), SnapshotRestoreError> {
    if actual > snapshot_limit {
        return Err(limit_error(dimension, snapshot_limit, actual));
    }
    if actual > target_limit {
        return Err(limit_error(dimension, target_limit, actual));
    }
    Ok(())
}

const fn limit_error(
    dimension: SnapshotLimitDimension,
    limit: u64,
    actual: u64,
) -> SnapshotRestoreError {
    SnapshotRestoreError::LimitExceeded {
        dimension,
        limit,
        actual,
    }
}

#[cfg(test)]
#[path = "tests/format_admission.rs"]
pub(crate) mod tests;
