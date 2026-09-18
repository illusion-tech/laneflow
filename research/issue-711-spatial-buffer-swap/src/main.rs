//! #711 独立研究程序：Spatial 位姿批次成功提交的完整路径 A/B 测量。
//!
//! 主测量对象是 `SpatialSession::extract_pose_batch` 完整调用（scratch 准备、全部
//! 输入采样、frame 检查与成功提交）。输入是受检 LFCA fixture 构建的共享根上的
//! `PoseInput` 列表；规模按“记录数”口径，不冒充真实车辆数。停车位探针对同一静态
//! 泊位重复采样，单独标注。fixture 构建、输入准备、验证与摘要输出都在计时窗口外。

use std::hint::black_box;
use std::mem::size_of;
use std::sync::Arc;
use std::time::Instant;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_spatial::{
    CanonicalPoseBatch, FramePlacementToken, PoseInput, PoseRecordId, SpatialError, SpatialSession,
};
use laneflow_static_contract::{CanonicalFrameOrdinal, LaneEdgeOrdinal, ParkingSpaceOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};
use sha2::{Digest, Sha256};

#[cfg(feature = "allocation")]
#[global_allocator]
static ALLOCATOR: &stats_alloc::StatsAlloc<std::alloc::System> = &stats_alloc::INSTRUMENTED_SYSTEM;

const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../../crates/laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
);
/// frame 0 内的三条边；主测量输入全部落在单一 canonical frame 上。
const FRAME0_EDGES: [LaneEdgeOrdinal; 3] = [
    LaneEdgeOrdinal::from_raw(0),
    LaneEdgeOrdinal::from_raw(1),
    LaneEdgeOrdinal::from_raw(2),
];
/// fixture 内唯一的显式停车位；停车探针只对它重复采样。
const PARKING0: ParkingSpaceOrdinal = ParkingSpaceOrdinal::from_raw(0);

const SIZES: &[usize] = &[0, 1, 1_000, 10_000, 100_000];
const WARMUP: usize = 4;
const SAMPLES: usize = 7;
const ITERATIONS: usize = 32;

struct Fixture {
    revision: Arc<SharedNetworkRevision>,
    lengths_mm: Vec<u32>,
    frame: CanonicalFrameOrdinal,
}

fn fixture() -> Fixture {
    let input = check_canonical_network_input(FULL_SPATIAL, FormatLimits::HARD)
        .expect("checked canonical network input");
    let revision = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("shared network revision");
    let lengths_mm: Vec<u32> = FRAME0_EDGES
        .iter()
        .map(|edge| revision.traffic().lane_lengths_millimetres()[edge.index()])
        .collect();
    let frame = revision
        .spatial()
        .and_then(|spatial| spatial.lane_pose())
        .and_then(|network| network.lane_geometry(FRAME0_EDGES[0]))
        .expect("lane geometry")
        .canonical_frame();
    Fixture {
        revision,
        lengths_mm,
        frame,
    }
}

fn bound_session(fixture: &Fixture) -> SpatialSession {
    SpatialSession::bind(Arc::clone(&fixture.revision))
        .expect("bind")
        .expect("session")
}

/// 生成 `records` 条 frame 0 内的车道输入；进度在各自边长内确定性变化。
fn lane_inputs(fixture: &Fixture, records: usize) -> Vec<PoseInput> {
    (0..records)
        .map(|index| {
            let record = u32::try_from(index).expect("record id fits u32");
            let edge_index = (index % FRAME0_EDGES.len()) as u32;
            let length_mm = fixture.lengths_mm[edge_index as usize];
            PoseInput::lane(
                PoseRecordId::new(record),
                FRAME0_EDGES[edge_index as usize],
                record.wrapping_mul(37) % length_mm,
            )
        })
        .collect()
}

/// 末条进度越界的失败输入序列，用于失败与重试场景。
fn failing_lane_inputs(fixture: &Fixture, records: usize) -> Vec<PoseInput> {
    let mut inputs = lane_inputs(fixture, records);
    let last = inputs.len() - 1;
    let edge = FRAME0_EDGES[last % FRAME0_EDGES.len()];
    let length_mm = fixture.lengths_mm[last % fixture.lengths_mm.len()];
    inputs[last] = PoseInput::lane(
        PoseRecordId::new(u32::try_from(last).unwrap()),
        edge,
        length_mm + 1,
    );
    inputs
}

/// 全部采样同一静态泊位的停车探针输入。
fn parking_inputs(records: usize) -> Vec<PoseInput> {
    (0..records)
        .map(|index| {
            PoseInput::parking(
                PoseRecordId::new(u32::try_from(index).expect("record id fits u32")),
                PARKING0,
            )
        })
        .collect()
}

fn token(value: u64) -> FramePlacementToken {
    FramePlacementToken::new(value)
}

/// 批次完整内容的 SHA-256 摘要（header + 记录身份 + 位模式级 pose 分量）。
fn digest_batch(batch: &CanonicalPoseBatch) -> String {
    fn hex(hasher: Sha256) -> String {
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
    let mut hasher = Sha256::new();
    hasher.update(batch.placement_token().raw().to_le_bytes());
    match batch.canonical_frame() {
        Some(frame) => {
            hasher.update([1]);
            hasher.update(frame.raw().to_le_bytes());
        }
        None => hasher.update([0]),
    }
    for record in batch.records() {
        hasher.update(record.record().raw().to_le_bytes());
        let pose = record.pose();
        let position = pose.position();
        let tangent = pose.tangent();
        let up = pose.up();
        for bits in [
            position.x(),
            position.y(),
            position.z(),
            tangent.x(),
            tangent.y(),
            tangent.z(),
            up.x(),
            up.y(),
            up.z(),
        ]
        .map(f32::to_bits)
        {
            hasher.update(bits.to_le_bytes());
        }
    }
    hex(hasher)
}

fn verify_batch(fixture: &Fixture, batch: &CanonicalPoseBatch, records: usize, token_value: u64) {
    assert_eq!(batch.records().len(), records, "record count");
    assert_eq!(batch.placement_token(), token(token_value), "token echo");
    assert_eq!(
        batch.network_revision(),
        Some(fixture.revision.network_revision()),
        "revision"
    );
    let expected_frame = if records == 0 {
        None
    } else {
        Some(fixture.frame)
    };
    assert_eq!(batch.canonical_frame(), expected_frame, "frame");
    for (index, record) in batch.records().iter().enumerate() {
        assert_eq!(
            record.record().raw(),
            u32::try_from(index).expect("record id fits u32"),
            "record id at {index}"
        );
    }
}

/// 计时并计数一个区域；allocation 构建下同时采集分配计数。
fn timed(case: &str, records: usize, sample: usize, iterations: usize, mut op: impl FnMut()) {
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
        "{case},{records},{sample},{iterations},{ns},{allocations},{reallocations},{allocated},{reallocated}"
    );
}

/// 稳定成功路径：暖机后每样本 `ITERATIONS` 次完整调用。
fn run_steady(fixture: &Fixture, records: usize, samples: usize, iterations: usize) {
    let inputs = lane_inputs(fixture, records);
    let mut session = bound_session(fixture);
    let mut output = CanonicalPoseBatch::new();
    session
        .extract_pose_batch(token(1), &inputs, &mut output)
        .expect("verify call");
    verify_batch(fixture, &output, records, 1);
    let mut repeat = CanonicalPoseBatch::new();
    session
        .extract_pose_batch(token(1), &inputs, &mut repeat)
        .expect("repeat call");
    assert_eq!(output, repeat, "deterministic output");
    eprintln!("oracle steady {records} {}", digest_batch(&output));
    drop(repeat);

    for _ in 0..WARMUP {
        session
            .extract_pose_batch(token(2), &inputs, &mut output)
            .expect("warm-up call");
    }
    for sample in 0..samples {
        timed("steady", records, sample, iterations, || {
            for _ in 0..iterations {
                session
                    .extract_pose_batch(token(3), black_box(&inputs), black_box(&mut output))
                    .expect("steady call");
            }
        });
    }
    black_box(&output);
}

/// 冷启动：每样本用全新 Session 与 output，只计时第一次完整调用。
fn run_cold(fixture: &Fixture, records: usize, samples: usize) {
    let inputs = lane_inputs(fixture, records);
    let mut oracle_session = bound_session(fixture);
    let mut oracle_output = CanonicalPoseBatch::new();
    oracle_session
        .extract_pose_batch(token(9), &inputs, &mut oracle_output)
        .expect("oracle call");
    verify_batch(fixture, &oracle_output, records, 9);
    eprintln!("oracle cold {records} {}", digest_batch(&oracle_output));

    for sample in 0..samples {
        let mut session = bound_session(fixture);
        let mut output = CanonicalPoseBatch::new();
        timed("cold", records, sample, 1, || {
            session
                .extract_pose_batch(token(9), black_box(&inputs), black_box(&mut output))
                .expect("cold call");
        });
        assert_eq!(output, oracle_output, "cold output equals oracle");
    }
}

/// 规模增长：每样本从全新配对出发，连续完成 1k → 10k → 100k 三次完整调用。
fn run_grow(fixture: &Fixture, samples: usize) {
    let small = lane_inputs(fixture, 1_000);
    let medium = lane_inputs(fixture, 10_000);
    let large = lane_inputs(fixture, 100_000);
    let mut oracle_session = bound_session(fixture);
    let mut oracle_output = CanonicalPoseBatch::new();
    oracle_session
        .extract_pose_batch(token(4), &large, &mut oracle_output)
        .expect("oracle call");
    eprintln!("oracle grow 100000 {}", digest_batch(&oracle_output));

    for sample in 0..samples {
        let mut session = bound_session(fixture);
        let mut output = CanonicalPoseBatch::new();
        timed("grow", 100_000, sample, 1, || {
            session
                .extract_pose_batch(token(4), black_box(&small), black_box(&mut output))
                .expect("grow call 1k");
            session
                .extract_pose_batch(token(4), black_box(&medium), black_box(&mut output))
                .expect("grow call 10k");
            session
                .extract_pose_batch(token(4), black_box(&large), black_box(&mut output))
                .expect("grow call 100k");
        });
        assert_eq!(output, oracle_output, "grow ends at the large oracle");
    }
}

/// 规模缩小：在 100k 上暖机后回到 1000 条，验证 backing 不释放、稳态零增长。
fn run_shrink(fixture: &Fixture, samples: usize, iterations: usize) {
    let large = lane_inputs(fixture, 100_000);
    let small = lane_inputs(fixture, 1_000);
    let mut session = bound_session(fixture);
    let mut output = CanonicalPoseBatch::new();
    for _ in 0..WARMUP {
        session
            .extract_pose_batch(token(5), &large, &mut output)
            .expect("warm-up at 100k");
    }
    session
        .extract_pose_batch(token(5), &small, &mut output)
        .expect("first shrink call");
    verify_batch(fixture, &output, 1_000, 5);
    eprintln!("oracle shrink 1000 {}", digest_batch(&output));
    for sample in 0..samples {
        timed("shrink", 1_000, sample, iterations, || {
            for _ in 0..iterations {
                session
                    .extract_pose_batch(token(5), black_box(&small), black_box(&mut output))
                    .expect("shrink call");
            }
        });
    }
}

/// 交替 output：同一 Session 轮流更新两个 output。
fn run_alternate(fixture: &Fixture, samples: usize, iterations: usize) {
    let inputs = lane_inputs(fixture, 10_000);
    let mut session = bound_session(fixture);
    let mut a = CanonicalPoseBatch::new();
    let mut b = CanonicalPoseBatch::new();
    for _ in 0..WARMUP {
        session
            .extract_pose_batch(token(6), &inputs, &mut a)
            .expect("warm a");
        session
            .extract_pose_batch(token(6), &inputs, &mut b)
            .expect("warm b");
    }
    verify_batch(fixture, &a, 10_000, 6);
    verify_batch(fixture, &b, 10_000, 6);
    eprintln!("oracle alternate 10000 {}", digest_batch(&a));
    for sample in 0..samples {
        timed("alternate", 10_000, sample, iterations, || {
            for _ in 0..iterations / 2 {
                session
                    .extract_pose_batch(token(6), black_box(&inputs), black_box(&mut a))
                    .expect("alternate a");
                session
                    .extract_pose_batch(token(6), black_box(&inputs), black_box(&mut b))
                    .expect("alternate b");
            }
        });
    }
    black_box((&a, &b));
}

/// 换入全新 output：Session 暖机后，每次调用使用全新 output。
fn run_fresh_output(fixture: &Fixture, samples: usize) {
    let inputs = lane_inputs(fixture, 10_000);
    let mut session = bound_session(fixture);
    let mut oracle = CanonicalPoseBatch::new();
    session
        .extract_pose_batch(token(7), &inputs, &mut oracle)
        .expect("oracle call");
    verify_batch(fixture, &oracle, 10_000, 7);
    eprintln!("oracle fresh_output 10000 {}", digest_batch(&oracle));
    for _ in 0..WARMUP {
        let mut fresh = CanonicalPoseBatch::new();
        session
            .extract_pose_batch(token(7), &inputs, &mut fresh)
            .expect("warm-up fresh call");
    }
    for sample in 0..samples {
        timed("fresh_output", 10_000, sample, ITERATIONS, || {
            for _ in 0..ITERATIONS {
                let mut fresh = CanonicalPoseBatch::new();
                session
                    .extract_pose_batch(token(7), black_box(&inputs), black_box(&mut fresh))
                    .expect("fresh output call");
                assert_eq!(fresh, oracle, "fresh output equals oracle");
                black_box(&fresh);
            }
        });
    }
}

/// 末条失败：整批在最后一条采样失败，旧输出保持不变。
fn run_fail_last(fixture: &Fixture, records: usize, samples: usize, iterations: usize) {
    let good = lane_inputs(fixture, records);
    let bad = failing_lane_inputs(fixture, records);
    let mut session = bound_session(fixture);
    let mut output = CanonicalPoseBatch::new();
    for _ in 0..WARMUP {
        session
            .extract_pose_batch(token(8), &good, &mut output)
            .expect("warm-up call");
    }
    let snapshot = output.clone();
    for sample in 0..samples {
        timed("fail_last", records, sample, iterations, || {
            for _ in 0..iterations {
                let error = session
                    .extract_pose_batch(token(8), black_box(&bad), black_box(&mut output))
                    .expect_err("batch must fail");
                match error {
                    SpatialError::SharedPoseRecordFailed { input_index, .. } => {
                        assert_eq!(input_index, bad.len() - 1);
                    }
                    other => panic!("unexpected error {other:?}"),
                }
            }
        });
        assert_eq!(output, snapshot, "failure keeps the old output");
    }
}

/// 失败后重试：每迭代先失败再成功，成功结果与 oracle 完全一致。
fn run_retry(fixture: &Fixture, records: usize, samples: usize, iterations: usize) {
    let good = lane_inputs(fixture, records);
    let bad = failing_lane_inputs(fixture, records);
    let mut session = bound_session(fixture);
    let mut output = CanonicalPoseBatch::new();
    session
        .extract_pose_batch(token(2), &good, &mut output)
        .expect("oracle call");
    verify_batch(fixture, &output, records, 2);
    let oracle = output.clone();
    eprintln!("oracle retry {records} {}", digest_batch(&oracle));
    for _ in 0..WARMUP {
        let _ = session.extract_pose_batch(token(3), &bad, &mut output);
        session
            .extract_pose_batch(token(2), &good, &mut output)
            .expect("retry warm-up");
    }
    for sample in 0..samples {
        timed("retry", records, sample, iterations, || {
            for _ in 0..iterations / 2 {
                let _ = session
                    .extract_pose_batch(token(3), black_box(&bad), black_box(&mut output))
                    .expect_err("batch must fail");
                session
                    .extract_pose_batch(token(2), black_box(&good), black_box(&mut output))
                    .expect("retry call");
            }
        });
        assert_eq!(output, oracle, "retry output equals oracle");
    }
}

/// 停车探针：同一静态泊位重复采样，不冒充大量真实停车车辆。
fn run_parking(fixture: &Fixture, records: usize, samples: usize, iterations: usize) {
    let inputs = parking_inputs(records);
    let mut session = bound_session(fixture);
    let mut output = CanonicalPoseBatch::new();
    session
        .extract_pose_batch(token(11), &inputs, &mut output)
        .expect("verify call");
    verify_batch(fixture, &output, records, 11);
    let mut repeat = CanonicalPoseBatch::new();
    session
        .extract_pose_batch(token(11), &inputs, &mut repeat)
        .expect("repeat call");
    assert_eq!(output, repeat, "deterministic parking output");
    eprintln!("oracle parking {records} {}", digest_batch(&output));
    drop(repeat);
    for _ in 0..WARMUP {
        session
            .extract_pose_batch(token(12), &inputs, &mut output)
            .expect("warm-up call");
    }
    for sample in 0..samples {
        timed("parking", records, sample, iterations, || {
            for _ in 0..iterations {
                session
                    .extract_pose_batch(token(13), black_box(&inputs), black_box(&mut output))
                    .expect("parking call");
            }
        });
    }
    black_box(&output);
}

/// records backing 建立成本：全新配对连续三次调用达到稳态，
/// 区域累计 allocated bytes 即两侧 records backing 的建立字节。
fn run_retained_build(fixture: &Fixture, records: usize) {
    let inputs = lane_inputs(fixture, records);
    let mut session = bound_session(fixture);
    let mut output = CanonicalPoseBatch::new();
    timed("retained_build", records, 0, 3, || {
        for round in 0..3 {
            session
                .extract_pose_batch(
                    token(20 + round),
                    black_box(&inputs),
                    black_box(&mut output),
                )
                .expect("retained build call");
        }
    });
    verify_batch(fixture, &output, records, 22);
    black_box(&output);
}

fn main() {
    let smoke = std::env::args().any(|arg| arg == "--smoke");
    let (sizes, samples, iterations) = if smoke {
        (&[0_usize, 1, 1_000][..], 2, 2)
    } else {
        (SIZES, SAMPLES, ITERATIONS)
    };
    eprintln!(
        "size_of CanonicalPoseRecord={} PoseInput={} allocation={}",
        size_of::<laneflow_spatial::CanonicalPoseRecord>(),
        size_of::<PoseInput>(),
        cfg!(feature = "allocation")
    );
    println!(
        "case,records,sample,iterations,ns,allocations,reallocations,allocated_bytes,reallocated_bytes"
    );

    let fixture = fixture();
    for records in sizes {
        run_steady(&fixture, *records, samples, iterations);
    }
    if smoke {
        run_fail_last(&fixture, 1_000, samples, iterations);
        run_retry(&fixture, 1_000, samples, iterations);
        run_parking(&fixture, 1_000, samples, iterations);
        return;
    }
    run_cold(&fixture, 10_000, samples);
    run_grow(&fixture, samples);
    run_shrink(&fixture, samples, iterations);
    run_alternate(&fixture, samples, iterations);
    run_fresh_output(&fixture, samples);
    run_fail_last(&fixture, 10_000, samples, iterations);
    run_fail_last(&fixture, 100_000, samples, iterations);
    run_retry(&fixture, 10_000, samples, iterations);
    run_retry(&fixture, 100_000, samples, iterations);
    run_parking(&fixture, 10_000, samples, iterations);
    run_parking(&fixture, 100_000, samples, iterations);
    run_retained_build(&fixture, 10_000);
    run_retained_build(&fixture, 100_000);
}
