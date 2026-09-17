//! 独立过滤运行的资源计量探针；配合 Windows 只读栈采样脚本。

use super::*;
use crate::admin::cutover::tests::transaction_tests::world_with_vehicle;
use stats_alloc::INSTRUMENTED_SYSTEM;
use std::path::Path;
use std::sync::Barrier;
use std::time::{Duration, Instant};

fn wait_ack(directory: &Path, phase: &str) {
    let deadline = Instant::now() + Duration::from_secs(45);
    let acknowledgement = directory.join(format!("{phase}.ack"));
    while !acknowledgement.exists() {
        assert!(
            Instant::now() < deadline,
            "native stack probe acknowledgement timeout"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn phase(
    directory: &Path,
    name: &str,
    heap_base: &stats_alloc::Stats,
    plan_bytes: usize,
    registration_bytes: usize,
) {
    let heap = INSTRUMENTED_SYSTEM.stats();
    let net = (heap.bytes_allocated as i128 - heap_base.bytes_allocated as i128)
        - (heap.bytes_deallocated as i128 - heap_base.bytes_deallocated as i128);
    let marker = format!(
        "{{\"pid\":{},\"phase\":\"{name}\",\"observed_heap_net_bytes_since_baseline\":{net},\"active_plan_bytes\":{plan_bytes},\"thread_registration_bytes\":{registration_bytes},\"requested_stack_bytes_per_worker\":{WORKER_STACK_BYTES}}}",
        std::process::id()
    );
    let temporary = directory.join(format!("{name}.tmp"));
    std::fs::write(&temporary, marker).unwrap();
    std::fs::rename(temporary, directory.join(format!("{name}.json"))).unwrap();
    wait_ack(directory, name);
}

#[test]
#[ignore = "需独立进程与 Windows tests/support/measure_execution_stacks.ps1 采样"]
fn execution_resource_memory_probe() {
    let _lock = RESOURCE_TEST_LOCK.lock().unwrap();
    let directory = std::env::var_os("LANEFLOW_EXECUTION_PROBE_DIR").expect("probe directory");
    let directory = Path::new(&directory);
    std::fs::create_dir_all(directory).unwrap();
    let (mut world, _, _) = world_with_vehicle(true);
    let heap_base = INSTRUMENTED_SYSTEM.stats();
    phase(
        directory,
        "baseline",
        &heap_base,
        world.execution.active_plan.retained_bytes(),
        0,
    );
    let config = ExecutionConfig::new(std::num::NonZeroU32::new(4).unwrap());
    world.execution = WorldExecution::start_private(config, &world.state);
    let plan_bytes = world.execution.active_plan.retained_bytes();
    let registration_bytes = match &world.execution.resources {
        ExecutionResources::Pool(resources) => {
            resources._workers.0.capacity() * size_of::<JoinHandle<()>>()
        }
        ExecutionResources::Caller => panic!("private worker resources"),
    };
    phase(
        directory,
        "idle",
        &heap_base,
        plan_bytes,
        registration_bytes,
    );
    let barrier = Barrier::new(4);
    world.execution.run(&mut world.state, |state, resources| {
        let mut output = [0_u64; 4];
        resources.for_each_chunk(state.read_view(), &mut output, 1, |view, index, chunk| {
            chunk[0] = view.committed.live_order.len() as u64;
            barrier.wait();
            if index == 0 {
                phase(
                    directory,
                    "busy",
                    &heap_base,
                    plan_bytes,
                    registration_bytes,
                );
            } else {
                wait_ack(directory, "busy");
            }
        });
        assert_eq!(output, [1; 4]);
    });
    drop(world);
    assert_eq!(LIVE_WORKERS.load(std::sync::atomic::Ordering::SeqCst), 0);
    phase(directory, "dropped", &heap_base, 0, 0);
}
