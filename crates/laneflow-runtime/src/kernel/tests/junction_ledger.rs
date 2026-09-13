//! 从 integrated run 的 warm-up 快照接续 H/2H/4H，复核摘要并读取现有私有账本。
use std::{fs, io::Write, path::PathBuf};

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

use crate::kernel::conflict::{
    ApproachFrontierCell, conflict_work_counts, reset_conflict_work_counts,
};
use crate::{
    CommittedNetworkSource, PublishedLfcaReference, SnapshotRestoreLimits, TickInput, WorldConfig,
    deterministic_state_digest, restore_lfrs,
};

#[test]
#[ignore = "manual #285 exact-input release-mode logical memory and work-count evidence"]
fn junction_reference_ledger() {
    let required = |key| std::env::var(key).unwrap_or_else(|_| panic!("missing {key}"));
    let lfca = fs::read(required("JUNCTION_LEDGER_LFCA")).unwrap();
    let snapshot = fs::read(required("JUNCTION_LEDGER_SNAPSHOT")).unwrap();
    let vehicles: u32 = required("JUNCTION_LEDGER_VEHICLES").parse().unwrap();
    let cells: u32 = required("JUNCTION_LEDGER_CELLS").parse().unwrap();
    let output = PathBuf::from(required("JUNCTION_LEDGER_OUTPUT"));
    assert!(!output.exists(), "ledger output must be new");
    let root = build_shared_network_revision(
        check_canonical_network_input(lfca.as_slice(), FormatLimits::HARD).unwrap(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(256 * 1024 * 1024, 64 * 1024 * 1024),
        ),
    )
    .unwrap();
    let origin = root.canonical_origin();
    let source = CommittedNetworkSource::Published {
        reference: PublishedLfcaReference::new(
            "scenario://complex-junction-scale",
            origin.canonical_artifact_digest(),
            origin.canonical_artifact_byte_length(),
            origin.network_revision(),
        )
        .unwrap(),
    };
    let restored = restore_lfrs(
        &snapshot,
        root,
        source,
        WorldConfig::new(
            vehicles,
            cells * 11,
            u64::from(cells) * 16_384,
            u64::from(cells) * 65_536,
            1,
            16,
        ),
        SnapshotRestoreLimits::new(256 * 1024 * 1024, 4096),
    )
    .unwrap();
    let mut world = restored.into_world();
    let mut csv = std::io::BufWriter::new(fs::File::create_new(output).unwrap());
    writeln!(csv, "observation_tick,shared_root,binding,committed,derived,scratch,admin,conflict_retained,top_two_cells,top_two_bytes,candidates,passage_visits,frontier_updates,cell_claim_queries,downstream_claim_queries,downstream_interval_visits,collision_rejections,commit_resource_visits,wait_for_nodes,wait_for_edges,wait_for_visits").unwrap();
    let mut peak = [0_u64; 5];
    for tick in 1..=4096 {
        reset_conflict_work_counts();
        world.step(TickInput::new(16)).unwrap();
        let work = conflict_work_counts();
        let memory = world.retained_memory();
        for (peak, bytes) in peak.iter_mut().zip(memory.partitions) {
            *peak = (*peak).max(bytes);
        }
        let cells = world.conflict_read().cell_count();
        let [binding, committed, derived, scratch, admin] = memory.partitions;
        writeln!(csv, "{tick},{},{binding},{committed},{derived},{scratch},{admin},{},{cells},{},{},{},{},{},{},{},{},{},{},{},{}",
            memory.shared_network, world.conflict_retained_logical_bytes(), cells * size_of::<ApproachFrontierCell>(),
            work.candidates, work.visited_passages, work.frontier_updates, work.cell_claim_queries,
            work.downstream_claim_queries, work.downstream_interval_visits, work.collision_rejections,
            work.commit_resource_visits, work.wait_for_nodes, work.wait_for_edges, work.wait_for_visits).unwrap();
        if [1024, 2048, 4096].contains(&tick) {
            eprintln!(
                "junction-ledger checkpoint={tick} digest={:x} peak_partitions={peak:?}",
                deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap()
            );
        }
    }
    csv.flush().unwrap();
    eprintln!(
        "junction-ledger world_owned_peak_partition_sum={} (not process peak; shared root counted separately)",
        peak.iter().sum::<u64>()
    );
}
