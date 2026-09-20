//! #712 独立研究程序：借用来源读取与 Adapter 全量缓冲复用的完整路径 A/B。
//!
//! A（before）= main（含 #719，按值来源 Vec）以 `legacy-source` feature 构建；
//! B（after）= 借用迭代器实现默认 feature 构建。两侧 main.rs 字节相同，
//! 只有 `for_each_source` 薄适配按 feature 选择消费写法；`adapter_full` 与
//! 全部验证代码两侧逐字相同。主指标是完整消费来源与完整 Adapter 提取；
//! Transform 转换为产品链路补充观察。fixture 构建、数据集准备、对拍与摘要
//! 输出都不进入计时区间。

use std::hint::black_box;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Instant;

use laneflow_bevy::{LaneFlowCommittedPoseBatch, LaneFlowSession, LaneFlowSessionConfig};
use laneflow_compiler::{
    CanonicalFrameInput, CanonicalPoint3F32Input, CompilationUnitBuilder, CompileLimits, Compiler,
    IidmVehicleProfileInput, LaneEdgeGeometryInput, LaneEdgeInput, LaneEdgeReference,
    ParkingFacilityInput, ParkingLaneAnchorInput, ParkingSpaceGeometryInput, ParkingSpaceInput,
    ParticipantClassInput, ParticipantClassReference, PortableDiffBase, PortableEmissionProvenance,
    SourceModuleHeader, SourceModuleHeaderInput, SyntheticModuleBuilder, VehicleProfileInput,
    emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_runtime::{
    CommittedNetworkSource, ParkedVehicleSpawnInput, ParkingTarget, PublishedLfcaReference,
    RouteRegisterInput, TrafficWorld, VehicleHandle, VehicleSpawnInput, VehicleStatus, WorldConfig,
    WorldPolicySelection,
};
use laneflow_spatial::SpatialSession;
use laneflow_static_contract::VehicleProfileOrdinal;
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};
use sha2::{Digest, Sha256};

#[cfg(feature = "allocation")]
#[global_allocator]
static ALLOCATOR: &stats_alloc::StatsAlloc<std::alloc::System> = &stats_alloc::INSTRUMENTED_SYSTEM;

const NAMESPACE: &str = "research/issue-712";
const PROFILE: VehicleProfileOrdinal = VehicleProfileOrdinal::from_raw(0);
const EDGE_COUNT: usize = 512;
const EDGE_LENGTH_M: f64 = 2_000.0;
const EDGE_LENGTH_MM: u32 = 2_000_000;
const EXPLICIT_SPACES: usize = 128;
const WARMUP: usize = 4;
const SAMPLES: usize = 7;
const ITERATIONS: usize = 32;

/// 消费全部已提交来源。A 侧走按值 Vec 的 as_slice；B 侧直接消费借用迭代器。
#[cfg(feature = "legacy-source")]
fn for_each_source<F>(world: &TrafficWorld, mut consume: F)
where
    F: FnMut(VehicleHandle, laneflow_runtime::PoseSource),
{
    for (vehicle, source) in world.committed_pose_sources().as_slice() {
        consume(*vehicle, *source);
    }
}

#[cfg(not(feature = "legacy-source"))]
fn for_each_source<F>(world: &TrafficWorld, mut consume: F)
where
    F: FnMut(VehicleHandle, laneflow_runtime::PoseSource),
{
    for (vehicle, source) in world.committed_pose_sources() {
        consume(vehicle, source);
    }
}

fn compile_revision() -> Arc<SharedNetworkRevision> {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: NAMESPACE,
            source_document_key: "issue-712-borrowed-sources.document",
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x77; 32],
            frontend_options_digest: [0x27; 32],
            random_seed: Some(712),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .expect("source header");
    let mut module = SyntheticModuleBuilder::new(header, &limits).expect("synthetic module");
    module
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "road-user",
            extends: None,
        })
        .expect("class")
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "car",
            participant_class: ParticipantClassReference::local("road-user"),
            iidm: IidmVehicleProfileInput {
                length_meters: 4.5,
                desired_speed_meters_per_second: 13.75,
                min_gap_meters: 2.0,
                time_headway_seconds: 1.4,
                max_acceleration_meters_per_second_squared: 1.8,
                comfortable_deceleration_meters_per_second_squared: 2.0,
                emergency_deceleration_meters_per_second_squared: 4.5,
            },
        })
        .expect("profile");
    let edge_keys: Vec<&'static str> = (0..EDGE_COUNT)
        .map(|index| -> &'static str { Box::leak(format!("edge-{index}").into_boxed_str()) })
        .collect();
    for key in &edge_keys {
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: key,
                length_meters: EDGE_LENGTH_M,
                speed_limit_meters_per_second: 30.0,
                successors: &[],
            })
            .expect("lane edge");
    }
    fn anchor(edge: &str, progress: f64) -> ParkingLaneAnchorInput<'_> {
        ParkingLaneAnchorInput {
            lane_edge: LaneEdgeReference::local(edge),
            progress_meters: progress,
        }
    }
    let _ = &edge_keys;
    module
        .add_parking_facility(ParkingFacilityInput {
            parking_facility_key: "facility",
            virtual_capacity: 200_000,
            virtual_entries: &[anchor("edge-0", 20.0)],
            virtual_exits: &[anchor("edge-0", 30.0)],
        })
        .expect("facility");
    let space_keys: Vec<&'static str> = (0..EXPLICIT_SPACES)
        .map(|index| -> &'static str { Box::leak(format!("space-{index}").into_boxed_str()) })
        .collect();
    for (index, space_key) in space_keys.iter().enumerate() {
        module
            .add_parking_space(ParkingSpaceInput {
                parking_space_key: space_key,
                parking_facility: Some(laneflow_compiler::ParkingFacilityReference::local(
                    "facility",
                )),
                entry: anchor(edge_keys[index % 8], 1_900.0),
                exit: anchor(edge_keys[index % 8], 1_950.0),
                geometry: ParkingSpaceGeometryInput {
                    lateral_offset_meters: -3.0,
                    heading_offset_radians: 0.25,
                    length_meters: 5.5,
                    width_meters: 2.6,
                },
            })
            .expect("parking space");
    }
    let mut geometries = Vec::new();
    let mut point_pairs = Vec::new();
    // 平行车道几何：每条车道是沿 X 的直线段，Z 按车道序号错开；全部坐标
    // 保持在 canonical 点分量闭区间内。
    for (index, key) in edge_keys.iter().enumerate() {
        let z = (index as f64) * 3.0;
        point_pairs.push((
            [
                CanonicalPoint3F32Input {
                    x: 0.0,
                    y: 0.0,
                    z: z as f32,
                },
                CanonicalPoint3F32Input {
                    x: EDGE_LENGTH_M as f32,
                    y: 0.0,
                    z: z as f32,
                },
            ],
            LaneEdgeReference::local(key),
        ));
    }
    for (points, edge) in &point_pairs {
        geometries.push(LaneEdgeGeometryInput {
            lane_edge: *edge,
            centerline_points: points,
        });
    }
    module
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "frame",
            lane_edge_geometries: &geometries,
        })
        .expect("canonical frame");
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().expect("finished module"))
        .expect("compilation module");
    let output = Compiler::new()
        .compile(unit.build().expect("compilation unit"))
        .expect("compiled output");
    let provenance =
        PortableEmissionProvenance::try_new("laneflow-issue-712-v1").expect("provenance");
    let candidate = emit_portable_candidate(
        &output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .expect("portable candidate");
    let checked = check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .expect("checked bundle");
    build_shared_network_revision(
        checked.canonical_network_input(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(2 * 1_024 * 1_024 * 1_024, 64 * 1_024 * 1_024),
        ),
    )
    .expect("shared revision")
}

fn install(revision: &Arc<SharedNetworkRevision>) -> TrafficWorld {
    let origin = revision.canonical_origin();
    TrafficWorld::install(
        Arc::clone(revision),
        WorldConfig::new(200_000, 1_024, 65_536, 1_024, 100),
        laneflow_runtime::ExecutionConfig::new(NonZeroU32::MIN),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://issue-712-borrowed-sources",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("fixture key"),
        },
        712,
        WorldPolicySelection::NotRequired,
    )
    .expect("install")
}

fn edge_ordinal(
    revision: &SharedNetworkRevision,
    index: usize,
) -> laneflow_static_contract::LaneEdgeOrdinal {
    let stable = laneflow_compiler::derive_canonical_stable_id_v1(
        laneflow_static_contract::EntityKind::LaneEdge,
        NAMESPACE,
        &format!("edge-{index}"),
        &CompileLimits::p100_initial_v1(),
    )
    .expect("edge stable id");
    revision
        .identity()
        .ordinal(laneflow_static_contract::LaneEdgeId::from_untyped(stable))
        .expect("edge ordinal")
}

fn lane_routes(
    world: &mut TrafficWorld,
    revision: &Arc<SharedNetworkRevision>,
) -> Vec<laneflow_runtime::RouteHandle> {
    (0..EDGE_COUNT)
        .map(|index| {
            world
                .register_route(RouteRegisterInput::new(vec![edge_ordinal(revision, index)]))
                .expect("lane route")
        })
        .collect()
}

/// 数据集形态：全 Active / 混合停车 / 高 Completed / 稀疏可表现。
struct Dataset {
    name: String,
    session: LaneFlowSession,
    presentable: usize,
}

fn spawn_active(
    world: &mut TrafficWorld,
    routes: &[laneflow_runtime::RouteHandle],
    slot: usize,
) -> VehicleHandle {
    let per_lane = (EDGE_LENGTH_MM / 10_000) as usize;
    let lane = slot / per_lane;
    let progress = u32::try_from(slot % per_lane).expect("progress fits u32") * 10_000;
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            PROFILE,
            routes[lane],
            0,
            progress,
            0,
        ))
        .expect("spawn active")
}

fn spawn_virtual(
    world: &mut TrafficWorld,
    routes: &[laneflow_runtime::RouteHandle],
    slot: usize,
) -> VehicleHandle {
    let route = routes[slot % routes.len()];
    world
        .spawn_parked_vehicle(
            ParkedVehicleSpawnInput::new(PROFILE, route, 0, 20_000 + slot as u32),
            ParkingTarget::VirtualPool(laneflow_static_contract::ParkingFacilityOrdinal::from_raw(
                0,
            )),
        )
        .expect("spawn virtual parked")
        .vehicle
}

fn spawn_explicit(
    world: &mut TrafficWorld,
    routes: &[laneflow_runtime::RouteHandle],
    space_index: usize,
    entry_progress_mm: u32,
) -> VehicleHandle {
    let route = routes[space_index % 8];
    world
        .spawn_parked_vehicle(
            ParkedVehicleSpawnInput::new(PROFILE, route, 0, entry_progress_mm),
            ParkingTarget::ExplicitSpace(laneflow_static_contract::ParkingSpaceOrdinal::from_raw(
                space_index as u32,
            )),
        )
        .expect("spawn explicit parked")
        .vehicle
}

/// 测前显式核验各状态数量；不满足目标形态时终止，而不是带错名继续测量。
fn count_status(world: &TrafficWorld, status: VehicleStatus) -> usize {
    world
        .live_vehicles()
        .iter()
        .filter(|handle| world.vehicle(**handle).map(|state| state.status()) == Some(status))
        .count()
}

#[allow(clippy::too_many_arguments)]
fn assert_dataset_shape(
    world: &TrafficWorld,
    label: &str,
    active: usize,
    explicit_parked: usize,
    virtual_parked: usize,
    completed: usize,
) {
    assert_eq!(
        count_status(world, VehicleStatus::Active),
        active,
        "{label}: Active count"
    );
    assert_eq!(
        count_status(world, VehicleStatus::Parked),
        explicit_parked + virtual_parked,
        "{label}: Parked count"
    );
    assert_eq!(
        count_status(world, VehicleStatus::Completed),
        completed,
        "{label}: Completed count"
    );
}

fn dataset(
    name: &str,
    revision: &Arc<SharedNetworkRevision>,
    shape: &str,
    total: usize,
) -> Dataset {
    let mut world = install(revision);
    let routes = lane_routes(&mut world, revision);
    let presentable = match shape {
        "all_active" => {
            for slot in 0..total {
                spawn_active(&mut world, &routes, slot);
            }
            assert_dataset_shape(&world, &format!("all_active({total})"), total, 0, 0, 0);
            total
        }
        "mixed_parking" => {
            let active = total / 2;
            for slot in 0..active {
                spawn_active(&mut world, &routes, slot);
            }
            for space_index in 0..EXPLICIT_SPACES {
                spawn_explicit(&mut world, &routes, space_index, 1_900_000);
            }
            for slot in 0..total - active - EXPLICIT_SPACES {
                spawn_virtual(&mut world, &routes, slot);
            }
            assert_dataset_shape(
                &world,
                &format!("mixed_parking({total})"),
                active,
                EXPLICIT_SPACES,
                total - active - EXPLICIT_SPACES,
                0,
            );
            active + EXPLICIT_SPACES
        }
        "high_completed" => {
            // 目标：≥90% 车辆 Completed、其余 Active。分批把车辆生成在各自
            // 车道末端（每车道至多一辆在途，避免重叠），一拍内到达路终完成；
            // 完成后路线终点占用释放，同车道可继续复用。数据集准备与状态
            // 断言都在计时区间外。
            let completed_target = total - total / 10;
            let mut completed = 0;
            while completed < completed_target {
                for route in routes
                    .iter()
                    .take((completed_target - completed).min(routes.len()))
                {
                    world
                        .spawn_vehicle(VehicleSpawnInput::new(
                            PROFILE,
                            *route,
                            0,
                            EDGE_LENGTH_MM,
                            0,
                        ))
                        .expect("spawn at lane end");
                }
                world
                    .step(laneflow_runtime::TickInput::new(100))
                    .expect("complete batch");
                completed = count_status(&world, VehicleStatus::Completed);
            }
            let active = total - completed;
            for slot in 0..active {
                spawn_active(&mut world, &routes, slot);
            }
            assert_dataset_shape(
                &world,
                &format!("high_completed({total})"),
                active,
                0,
                0,
                completed,
            );
            active
        }
        "sparse_presentable" => {
            for slot in 0..100 {
                spawn_active(&mut world, &routes, slot);
            }
            for slot in 0..total - 100 {
                spawn_virtual(&mut world, &routes, slot);
            }
            assert_dataset_shape(
                &world,
                &format!("sparse_presentable({total})"),
                100,
                0,
                total - 100,
                0,
            );
            100
        }
        other => panic!("unknown dataset shape {other}"),
    };
    let spatial = SpatialSession::bind(Arc::clone(revision))
        .expect("bind")
        .expect("spatial");
    Dataset {
        name: name.to_string(),
        session: LaneFlowSession::new(
            world,
            Some(spatial),
            LaneFlowSessionConfig::new(NonZeroU32::new(64).expect("non-zero")),
        )
        .expect("session"),
        presentable,
    }
}

/// 完整提取输出的 SHA-256 摘要：车辆句柄序列、全部记录（身份 + 位模式级
/// pose）、批次 header（修订、frame、placement token）、完整消费上下文
/// （world id + world generation）与两个序列的长度。
fn digest_output(output: &LaneFlowCommittedPoseBatch) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"v2");
    hasher.update((output.vehicles().len() as u64).to_le_bytes());
    hasher.update((output.batch().records().len() as u64).to_le_bytes());
    for vehicle in output.vehicles() {
        hasher.update(format!("{vehicle:?}").as_bytes());
        hasher.update(b"|");
    }
    for record in output.batch().records() {
        hasher.update(record.record().raw().to_le_bytes());
        let pose = record.pose();
        for value in [
            pose.position().x(),
            pose.position().y(),
            pose.position().z(),
            pose.tangent().x(),
            pose.tangent().y(),
            pose.tangent().z(),
            pose.up().x(),
            pose.up().y(),
            pose.up().z(),
        ]
        .map(f32::to_bits)
        {
            hasher.update(value.to_le_bytes());
        }
    }
    hasher.update(format!("{:?}", output.batch().network_revision()).as_bytes());
    hasher.update(b"|");
    hasher.update(format!("{:?}", output.batch().canonical_frame()).as_bytes());
    hasher.update(b"|");
    hasher.update(output.batch().placement_token().raw().to_le_bytes());
    hasher.update(b"|");
    hasher.update(output.context().world_id().to_le_bytes());
    hasher.update(b"|");
    hasher.update(format!("{:?}", output.context().world_generation()).as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 有序完整来源序列的 SHA-256 摘要（句柄 + 来源判别与全部字段）。
fn digest_sources(sources: &[(VehicleHandle, laneflow_runtime::PoseSource)]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"sources-v2");
    hasher.update((sources.len() as u64).to_le_bytes());
    for (vehicle, source) in sources {
        hasher.update(format!("{vehicle:?}").as_bytes());
        hasher.update(b"|");
        match source {
            laneflow_runtime::PoseSource::Lane { edge, progress_mm } => {
                hasher.update(b"L");
                hasher.update(edge.raw().to_le_bytes());
                hasher.update(progress_mm.to_le_bytes());
            }
            laneflow_runtime::PoseSource::Parking { space } => {
                hasher.update(b"P");
                hasher.update(space.raw().to_le_bytes());
            }
        }
        hasher.update(b";");
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 收集完整来源序列（A 侧 as_slice / B 侧迭代器；仅正确性取证与断言使用）。
#[cfg(feature = "legacy-source")]
fn collected_sources(world: &TrafficWorld) -> Vec<(VehicleHandle, laneflow_runtime::PoseSource)> {
    world.committed_pose_sources().as_slice().to_vec()
}

#[cfg(not(feature = "legacy-source"))]
fn collected_sources(world: &TrafficWorld) -> Vec<(VehicleHandle, laneflow_runtime::PoseSource)> {
    world.committed_pose_sources().collect()
}

/// 稳态成功场景逐样本断言：指定稳态窗口内零分配/零重分配
/// （仅 allocation 构建；冷启动与 fresh-output 生命周期另行解释）。
macro_rules! assert_steady_sample {
    ($stats:expr, $case:expr, $sample:expr) => {
        #[cfg(all(feature = "allocation", not(feature = "legacy-source")))]
        assert!(
            $stats.0 == 0 && $stats.1 == 0,
            "steady {} sample {} must not allocate (got {} alloc / {} realloc)",
            $case,
            $sample,
            $stats.0,
            $stats.1
        );
    };
}

fn timed(
    case: &str,
    dataset: &str,
    sample: usize,
    iterations: usize,
    mut op: impl FnMut(),
) -> (u64, u64) {
    #[cfg(feature = "allocation")]
    let region = stats_alloc::Region::new(ALLOCATOR);
    let started = Instant::now();
    op();
    let ns = started.elapsed().as_nanos();
    #[cfg(feature = "allocation")]
    let (allocations, reallocations, allocated, reallocated) = {
        let stats = region.change();
        (
            stats.allocations,
            stats.reallocations,
            stats.bytes_allocated,
            stats.bytes_reallocated,
        )
    };
    #[cfg(not(feature = "allocation"))]
    let (allocations, reallocations, allocated, reallocated) = (0, 0, 0, 0);
    println!(
        "{case},{dataset},{sample},{iterations},{ns},{allocations},{reallocations},{allocated},{reallocated}"
    );
    (allocations as u64, reallocations as u64)
}

/// 指标 1：完整消费来源迭代器（含成员判定与遍历）。
fn run_source_full(dataset: &mut Dataset, samples: usize, iterations: usize) {
    let expected = dataset.presentable;
    let sources = collected_sources(dataset.session.world());
    assert_eq!(sources.len(), expected, "presentable source count");
    eprintln!(
        "oracle source {} {}",
        dataset.name,
        digest_sources(&sources)
    );
    for _ in 0..WARMUP {
        for_each_source(dataset.session.world(), |_, _| {});
    }
    for sample in 0..samples {
        let _stats = timed("source_full", &dataset.name, sample, iterations, || {
            for _ in 0..iterations {
                for_each_source(dataset.session.world(), |vehicle, source| {
                    black_box((vehicle, source));
                });
            }
        });
        assert_steady_sample!(_stats, "source_full", sample);
    }
}

/// 指标 2：完整 Adapter 提取（配对、单遍候选构建、Spatial 采样、成功提交）。
/// 计时循环使用 token 3；参考结果在计时外以同 token 准备，每个计时样本
/// 结束后校验该样本留下的实际 output（全部字段，v2 摘要 + 显式字段断言）。
fn run_adapter_full(dataset: &mut Dataset, samples: usize, iterations: usize) {
    let mut output = LaneFlowCommittedPoseBatch::new();
    dataset
        .session
        .extract_committed_pose_batch(laneflow_spatial::FramePlacementToken::new(1), &mut output)
        .expect("verify extract");
    assert_eq!(output.vehicles().len(), dataset.presentable);
    assert_eq!(output.batch().records().len(), dataset.presentable);
    // reference 在最终暖机之前准备：reference 是全新 output，若放在暖机之后
    // 会把 Session 的暖 backing 交换给 reference、接回空 backing，使第一个
    // 计时窗口重新分配（该轮换行为由 capacity 测试覆盖，不属于稳态测量）。
    let mut reference = LaneFlowCommittedPoseBatch::new();
    dataset
        .session
        .extract_committed_pose_batch(
            laneflow_spatial::FramePlacementToken::new(3),
            &mut reference,
        )
        .expect("reference extract with the measured token");
    let reference_digest = digest_output(&reference);
    for _ in 0..WARMUP {
        dataset
            .session
            .extract_committed_pose_batch(
                laneflow_spatial::FramePlacementToken::new(2),
                &mut output,
            )
            .expect("final warm-up on the measured output");
    }

    for sample in 0..samples {
        let _stats = timed("adapter_full", &dataset.name, sample, iterations, || {
            for _ in 0..iterations {
                dataset
                    .session
                    .extract_committed_pose_batch(
                        laneflow_spatial::FramePlacementToken::new(3),
                        black_box(&mut output),
                    )
                    .expect("adapter extract");
            }
        });
        assert_steady_sample!(_stats, "adapter_full", sample);
        // 计时外校验：本样本最后一次实际提取留下的 output 与参考全字段一致。
        assert_eq!(
            digest_output(&output),
            reference_digest,
            "post-sample output"
        );
        assert_eq!(output.vehicles().len(), dataset.presentable);
        assert_eq!(output.batch().records().len(), dataset.presentable);
        assert_eq!(
            output.batch().placement_token(),
            laneflow_spatial::FramePlacementToken::new(3)
        );
        assert!(output.batch().canonical_frame().is_some());
    }
    eprintln!("oracle adapter {} {}", dataset.name, reference_digest);
    black_box(&output);
}

/// 指标 3（补充）：提取后的 Transform 转换，产品链路观察。
fn run_transform_convert(dataset: &mut Dataset, samples: usize, iterations: usize) {
    let mut output = LaneFlowCommittedPoseBatch::new();
    dataset
        .session
        .extract_committed_pose_batch(laneflow_spatial::FramePlacementToken::new(1), &mut output)
        .expect("extract for transform");
    let mut transforms: Vec<bevy_math::Vec3> = Vec::with_capacity(output.vehicles().len());
    for _ in 0..WARMUP {
        transforms.clear();
        for record in output.batch().records() {
            let position = record.pose().position();
            transforms.push(bevy_math::Vec3::new(
                position.x(),
                position.y(),
                position.z(),
            ));
        }
    }
    for sample in 0..samples {
        timed(
            "transform_convert",
            &dataset.name,
            sample,
            iterations,
            || {
                for _ in 0..iterations {
                    transforms.clear();
                    for record in black_box(output.batch().records()) {
                        let position = record.pose().position();
                        transforms.push(bevy_math::Vec3::new(
                            position.x(),
                            position.y(),
                            position.z(),
                        ));
                    }
                    black_box(&transforms);
                }
            },
        );
    }
}

/// 生命周期：冷启动（全新 Session+output 的首次提取）。
fn run_cold(revision: &Arc<SharedNetworkRevision>, samples: usize) {
    let mut probe = dataset("cold_probe", revision, "all_active", 1_000);
    let mut reference = LaneFlowCommittedPoseBatch::new();
    probe
        .session
        .extract_committed_pose_batch(
            laneflow_spatial::FramePlacementToken::new(9),
            &mut reference,
        )
        .expect("cold oracle");
    eprintln!("oracle cold cold {}", digest_output(&reference));
    for sample in 0..samples {
        let mut fresh = dataset("cold_probe", revision, "all_active", 1_000);
        let mut output = LaneFlowCommittedPoseBatch::new();
        timed("cold", "cold_probe", sample, 1, || {
            fresh
                .session
                .extract_committed_pose_batch(
                    laneflow_spatial::FramePlacementToken::new(9),
                    black_box(&mut output),
                )
                .expect("cold extract");
        });
        assert_eq!(digest_output(&output), digest_output(&reference));
    }
}

/// 生命周期：换入全新 output（计时内保持创建—提取—释放生命周期）；
/// 计时后另做正确性重放（同调用序列、逐次校验），明确不是计时结果本身。
fn run_fresh_output(dataset: &mut Dataset, samples: usize, iterations: usize) {
    for _ in 0..WARMUP {
        let mut fresh = LaneFlowCommittedPoseBatch::new();
        dataset
            .session
            .extract_committed_pose_batch(laneflow_spatial::FramePlacementToken::new(7), &mut fresh)
            .expect("warm-up fresh output");
    }
    let mut reference = LaneFlowCommittedPoseBatch::new();
    dataset
        .session
        .extract_committed_pose_batch(
            laneflow_spatial::FramePlacementToken::new(7),
            &mut reference,
        )
        .expect("reference extract");
    let reference_digest = digest_output(&reference);
    for sample in 0..samples {
        timed("fresh_output", &dataset.name, sample, iterations, || {
            for _ in 0..iterations {
                let mut fresh = LaneFlowCommittedPoseBatch::new();
                dataset
                    .session
                    .extract_committed_pose_batch(
                        laneflow_spatial::FramePlacementToken::new(7),
                        black_box(&mut fresh),
                    )
                    .expect("fresh output extract");
                black_box(&fresh);
            }
        });
    }
    // 正确性重放：不进入计时，逐次校验每个临时 output 与参考一致。
    for _ in 0..iterations {
        let mut fresh = LaneFlowCommittedPoseBatch::new();
        dataset
            .session
            .extract_committed_pose_batch(laneflow_spatial::FramePlacementToken::new(7), &mut fresh)
            .expect("replay fresh output");
        assert_eq!(digest_output(&fresh), reference_digest, "replay output");
    }
}

/// 生命周期：两个 output 交替使用；计时结束后分别校验 A、B 与参考一致。
fn run_alternate(dataset: &mut Dataset, samples: usize, iterations: usize) {
    let token = laneflow_spatial::FramePlacementToken::new(8);
    // reference 先填充，再对真正计时的 A/B 配对完成最终暖机。
    let mut reference = LaneFlowCommittedPoseBatch::new();
    dataset
        .session
        .extract_committed_pose_batch(token, &mut reference)
        .expect("reference extract");
    let reference_digest = digest_output(&reference);
    let mut a = LaneFlowCommittedPoseBatch::new();
    let mut b = LaneFlowCommittedPoseBatch::new();
    for _ in 0..WARMUP {
        dataset
            .session
            .extract_committed_pose_batch(token, &mut a)
            .expect("warm a");
        dataset
            .session
            .extract_committed_pose_batch(token, &mut b)
            .expect("warm b");
    }

    for sample in 0..samples {
        let _stats = timed("alternate", &dataset.name, sample, iterations, || {
            for _ in 0..iterations / 2 {
                dataset
                    .session
                    .extract_committed_pose_batch(token, black_box(&mut a))
                    .expect("alternate a");
                dataset
                    .session
                    .extract_committed_pose_batch(token, black_box(&mut b))
                    .expect("alternate b");
            }
        });
        assert_steady_sample!(_stats, "alternate", sample);
    }
    // 计时外校验：A、B 都与参考全字段一致。
    assert_eq!(digest_output(&a), reference_digest, "post-sample output a");
    assert_eq!(digest_output(&b), reference_digest, "post-sample output b");
    eprintln!("oracle alternate {} {}", dataset.name, reference_digest);
    black_box((&a, &b));
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let smoke = args.iter().any(|arg| arg == "--smoke");
    let (samples, iterations, scales) = if smoke {
        (2, 2, vec![("all_active", 1_000_usize)])
    } else {
        (
            SAMPLES,
            ITERATIONS,
            vec![
                ("all_active", 10_000),
                ("all_active", 100_000),
                ("mixed_parking", 10_000),
                ("mixed_parking", 100_000),
                ("high_completed", 10_000),
                ("sparse_presentable", 10_000),
            ],
        )
    };
    eprintln!(
        "features legacy_source={} allocation={} edge_count={} spaces={}",
        cfg!(feature = "legacy-source"),
        cfg!(feature = "allocation"),
        EDGE_COUNT,
        EXPLICIT_SPACES
    );
    println!(
        "case,dataset,sample,iterations,ns,allocations,reallocations,allocated_bytes,reallocated_bytes"
    );

    let revision = compile_revision();
    for (shape, total) in scales {
        let name = format!("{shape}_{total}");
        let mut set = dataset(&name, &revision, shape, total);
        eprintln!(
            "dataset {name} presentable={} live={}",
            set.presentable,
            set.session.world().live_vehicles().len()
        );
        run_source_full(&mut set, samples, iterations);
        run_adapter_full(&mut set, samples, iterations);
        run_transform_convert(&mut set, samples, iterations);
        if shape == "all_active" && !smoke {
            run_fresh_output(&mut set, samples, iterations);
            run_alternate(&mut set, samples, iterations);
        }
    }
    if !smoke {
        run_cold(&revision, samples);
    }
}
