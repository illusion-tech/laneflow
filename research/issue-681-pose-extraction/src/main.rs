//! #681 独立研究程序；生产提取为 oracle，候选只存在于本程序。
mod fixture;

use bevy_ecs::{entity::Entity, world::World};
use bevy_math::Vec3;
use bevy_transform::components::Transform;
use laneflow_bevy::{LaneFlowCommittedPoseBatch, LaneFlowSession, LaneFlowSessionConfig};
use laneflow_runtime::{PoseSource as RuntimeSource, TrafficWorld, VehicleHandle, VehicleStatus};
use laneflow_spatial::{
    CanonicalPoseBatch, CanonicalPoseRecord, FramePlacementToken, PoseInput, PoseRecordId,
    SpatialSession,
};
use laneflow_static_contract::{LaneEdgeOrdinal, ParkingSpaceOrdinal};
use std::{
    collections::HashMap, hint::black_box, mem::size_of, num::NonZeroU32, sync::Arc, time::Instant,
};

#[cfg(feature = "allocation")]
#[global_allocator]
static ALLOCATOR: &stats_alloc::StatsAlloc<std::alloc::System> = &stats_alloc::INSTRUMENTED_SYSTEM;

fn measure(name: &str, n: usize, k: usize, iterations: usize, mut operation: impl FnMut()) {
    for _ in 0..4 {
        operation();
    }
    for sample in 0..7 {
        #[cfg(feature = "allocation")]
        let region = stats_alloc::Region::new(ALLOCATOR);
        let started = Instant::now();
        for _ in 0..iterations {
            operation();
        }
        let ns = started.elapsed().as_nanos();
        #[cfg(feature = "allocation")]
        let (allocations, reallocations, bytes, growth) = {
            let s = region.change();
            (
                s.allocations,
                s.reallocations,
                s.bytes_allocated,
                s.bytes_reallocated,
            )
        };
        #[cfg(not(feature = "allocation"))]
        let (allocations, reallocations, bytes, growth) = (0, 0, 0, 0);
        println!(
            "{name},{n},{k},{sample},{iterations},{ns},{allocations},{reallocations},{bytes},{growth}"
        );
    }
}

fn mapped(index: usize, source: RuntimeSource) -> PoseInput {
    let id = PoseRecordId::new(u32::try_from(index).unwrap());
    match source {
        RuntimeSource::Lane { edge, progress_mm } => PoseInput::lane(id, edge, progress_mm),
        RuntimeSource::Parking { space } => PoseInput::parking(id, space),
    }
}

// 只用于研究受选句柄查询的可行性，不成为 Runtime 的第二套生产权威。
fn selected_inputs(world: &TrafficWorld, handles: &[VehicleHandle], output: &mut Vec<PoseInput>) {
    output.clear();
    for (i, handle) in handles.iter().enumerate() {
        let state = world.vehicle(*handle).expect("selected live handle");
        assert_eq!(state.status(), VehicleStatus::Active);
        let edge = world.route_edges(state.route()).unwrap()[state.route_edge_index() as usize];
        output.push(PoseInput::lane(
            PoseRecordId::new(i as u32),
            edge,
            state.progress_mm(),
        ));
    }
}

fn transform(record: &CanonicalPoseRecord) -> Transform {
    let pose = record.pose();
    let p = pose.position();
    let t = pose.tangent();
    let u = pose.up();
    Transform::from_xyz(p.x(), p.y(), p.z()).looking_to(
        Vec3::new(t.x(), t.y(), t.z()),
        Vec3::new(u.x(), u.y(), u.z()),
    )
}

fn exercise(count: u32, smoke: bool) {
    let root = fixture::revision();
    eprintln!("fixture n={count} origin={:?}", root.canonical_origin());
    let world = fixture::world(&root, count);
    let mut adapter = LaneFlowSession::new(
        world,
        SpatialSession::bind(Arc::clone(&root)).unwrap(),
        LaneFlowSessionConfig::new(NonZeroU32::new(1).unwrap()),
    )
    .unwrap();
    let before = adapter.world().capture_snapshot().unwrap();
    let mut oracle = LaneFlowCommittedPoseBatch::default();
    adapter
        .extract_committed_pose_batch(FramePlacementToken::new(1), &mut oracle)
        .unwrap();
    let n = count as usize;
    assert_eq!(oracle.vehicles(), adapter.world().live_vehicles());
    let iterations = if smoke { 2 } else { 32 };
    measure("runtime_sources", n, n, iterations, || {
        black_box(adapter.world().committed_pose_sources());
    });
    let sources = adapter.world().committed_pose_sources();
    let mut inputs: Vec<_> = sources
        .as_slice()
        .iter()
        .enumerate()
        .map(|(i, (_, source))| mapped(i, *source))
        .collect();
    let mut spatial = SpatialSession::bind(Arc::clone(&root)).unwrap().unwrap();
    let mut batch = CanonicalPoseBatch::new();
    spatial
        .extract_pose_batch(FramePlacementToken::new(1), &inputs, &mut batch)
        .unwrap();
    assert_eq!(&batch, oracle.batch());
    measure("spatial_full", n, n, iterations, || {
        spatial
            .extract_pose_batch(FramePlacementToken::new(1), black_box(&inputs), &mut batch)
            .unwrap();
        black_box(&batch);
    });
    let mut copied = Vec::new();
    measure("record_copy_kernel", n, n, iterations, || {
        copied.clear();
        copied.extend_from_slice(black_box(batch.records()));
        black_box(&copied);
    });
    let mut other = copied.clone();
    measure("record_swap_kernel", n, n, iterations, || {
        std::mem::swap(black_box(&mut copied), black_box(&mut other));
        black_box(&copied);
    });
    let mut full = LaneFlowCommittedPoseBatch::default();
    measure("adapter_full", n, n, iterations, || {
        adapter
            .extract_committed_pose_batch(FramePlacementToken::new(1), &mut full)
            .unwrap();
        black_box(&full);
    });
    assert_eq!(full.batch(), oracle.batch());
    assert_eq!(full.vehicles(), oracle.vehicles());
    for divisor in [1, 10, 100] {
        let k = n / divisor;
        let handles = &oracle.vehicles()[..k];
        let mut selected = Vec::new();
        let mut product = CanonicalPoseBatch::new();
        selected_inputs(adapter.world(), handles, &mut selected);
        spatial
            .extract_pose_batch(FramePlacementToken::new(1), &selected, &mut product)
            .unwrap();
        assert_eq!(product.records(), &oracle.batch().records()[..k]);
        measure("product_selected_probe", n, k, iterations, || {
            selected_inputs(adapter.world(), black_box(handles), &mut selected);
            spatial
                .extract_pose_batch(FramePlacementToken::new(1), &selected, &mut product)
                .unwrap();
            black_box(&product);
        });
        let mut converted = Vec::new();
        measure("transform_full_before_select", n, k, iterations, || {
            converted.clear();
            converted.extend(black_box(oracle.batch().records()).iter().map(transform));
            black_box(&converted[..k]);
        });
        measure("transform_selected", n, k, iterations, || {
            converted.clear();
            converted.extend(black_box(product.records()).iter().map(transform));
            black_box(&converted);
        });
        binding_probe(&mut adapter, handles, n, iterations);
    }
    assert_eq!(adapter.world().capture_snapshot().unwrap(), before);
    // 末条无效输入验证完整输出（含 header/token）保持；随后正常重试。
    inputs.push(PoseInput::lane(
        PoseRecordId::new(count),
        LaneEdgeOrdinal::from_raw(0),
        u32::MAX,
    ));
    let previous = batch.clone();
    assert!(
        spatial
            .extract_pose_batch(FramePlacementToken::new(2), &inputs, &mut batch)
            .is_err()
    );
    assert_eq!(batch, previous);
    inputs.pop();
    spatial
        .extract_pose_batch(FramePlacementToken::new(1), &inputs, &mut batch)
        .unwrap();
    assert_eq!(batch, previous);
    parking_probe(&root, iterations);
    eprintln!(
        "oracle-ok n={n} full=selected@100%,10%,1% unchanged-world=true late-failure-retry=true"
    );
}

fn binding_probe(
    adapter: &mut LaneFlowSession,
    handles: &[VehicleHandle],
    n: usize,
    iterations: usize,
) {
    let k = handles.len();
    let mut ecs = World::new();
    let mut bindings: HashMap<VehicleHandle, Entity> = HashMap::with_capacity(k);
    for handle in handles {
        // 每组独立建宿主世界，前一组结束时已解除全部研究绑定。
        assert!(adapter.vehicle_entity(*handle).is_none());
        let entity = ecs.spawn(Transform::IDENTITY).id();
        adapter.bind_vehicle_entity(*handle, entity).unwrap();
        bindings.insert(*handle, entity);
    }
    measure("binding_full_live_map", n, k, iterations, || {
        let live: HashMap<_, _> = adapter
            .world()
            .live_vehicles()
            .iter()
            .enumerate()
            .map(|(i, handle)| (*handle, i))
            .collect();
        for (handle, entity) in &bindings {
            assert!(live.contains_key(handle));
            assert_eq!(adapter.vehicle_entity(*handle), Some(*entity));
        }
        black_box(&live);
    });
    measure("binding_retained_lookup", n, k, iterations, || {
        for (handle, entity) in &bindings {
            assert!(adapter.world().vehicle(*handle).is_some());
            assert_eq!(adapter.vehicle_entity(*handle), Some(*entity));
        }
    });
    let mut visible = true;
    measure("binding_visibility_toggle", n, k, iterations, || {
        for entity in bindings.values() {
            if visible {
                ecs.entity_mut(*entity).remove::<Transform>();
            } else {
                ecs.entity_mut(*entity).insert(Transform::IDENTITY);
            }
        }
        visible = !visible;
        black_box(&ecs);
    });
    for handle in handles {
        adapter.unbind_vehicle(*handle).unwrap();
    }
}

fn parking_probe(root: &Arc<laneflow_static_network::SharedNetworkRevision>, iterations: usize) {
    let mut spatial = SpatialSession::bind(Arc::clone(root)).unwrap().unwrap();
    let inputs: Vec<_> = (0..fixture::PARKING)
        .map(|i| PoseInput::parking(PoseRecordId::new(i), ParkingSpaceOrdinal::from_raw(i)))
        .collect();
    let mut batch = CanonicalPoseBatch::new();
    spatial
        .extract_pose_batch(FramePlacementToken::new(1), &inputs, &mut batch)
        .unwrap();
    let cache = batch.clone();
    let n = inputs.len();
    measure("parking_resample", n, n, iterations * 16, || {
        spatial
            .extract_pose_batch(FramePlacementToken::new(1), &inputs, &mut batch)
            .unwrap();
        black_box(&batch);
    });
    assert_eq!(batch, cache);
    let mut records = Vec::new();
    measure("parking_cached_copy_kernel", n, n, iterations * 16, || {
        records.clear();
        records.extend_from_slice(black_box(cache.records()));
        black_box(&records);
    });
    assert_eq!(records, cache.records());
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let count: u32 = args.get(1).expect("count argument").parse().unwrap();
    assert!((100..=100_000).contains(&count));
    eprintln!(
        "sizes source={} input={} record={} handle={} transform={} allocation={}",
        size_of::<(VehicleHandle, RuntimeSource)>(),
        size_of::<PoseInput>(),
        size_of::<CanonicalPoseRecord>(),
        size_of::<VehicleHandle>(),
        size_of::<Transform>(),
        cfg!(feature = "allocation")
    );
    println!(
        "case,n,k,sample,iterations,ns,allocations,reallocations,allocated_bytes,reallocated_bytes"
    );
    exercise(count, args.iter().any(|a| a == "--smoke"));
}
