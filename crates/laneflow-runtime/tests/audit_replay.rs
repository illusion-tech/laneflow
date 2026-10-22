//! #960 宿主审计基础切片：独立进程、认证后恢复、完成性优先的三值报告。
//! 测试传输是本文件私有的定长记录，不是产品协议或可信日志存储。

#[path = "support/audit_replay.rs"]
mod audit;
#[path = "support/policy.rs"]
mod test_policy;

use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use audit::{AuditResult, Boundary, Failure, Point, Position, Trace, compare};
use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CommittedNetworkSource, ExecutionConfig, PublishedLfcaReference, RouteRegisterInput,
    SnapshotDigestLimits, SnapshotRestoreLimits, TickInput, TrafficWorld, VehicleSpawnInput,
    WorldConfig, attest_published_shared_network_revision, encode_lfrs, restore_lfrs,
};
use laneflow_static_contract::{LaneEdgeOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::{
    AttestationOutcome, AttestationVerdict, SharedNetworkBuildLimits, SharedNetworkBuildOptions,
    SharedNetworkRevision, SpatialBuildOption, build_shared_network_revision,
};

const LFCA: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
);
const POINT_BYTES: usize = 48;
const POINT_COUNT: usize = 4;
const DIGEST_BYTES: u64 = 16 * 1_024 * 1_024;

fn options() -> SharedNetworkBuildOptions<'static> {
    SharedNetworkBuildOptions::new(
        SpatialBuildOption::RetainAvailable,
        SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
    )
}

fn revision() -> std::sync::Arc<SharedNetworkRevision> {
    build_shared_network_revision(
        check_canonical_network_input(LFCA, FormatLimits::HARD).expect("fixture"),
        options(),
    )
    .expect("revision")
}

fn reference(revision: &SharedNetworkRevision) -> PublishedLfcaReference {
    let origin = revision.canonical_origin();
    PublishedLfcaReference::new(
        "fixture://audit-replay",
        origin.canonical_artifact_digest(),
        origin.canonical_artifact_byte_length(),
        origin.network_revision(),
    )
    .expect("reference")
}

fn config() -> WorldConfig {
    WorldConfig::new(8, 4, 1_024, 1_024, 100)
}

fn execution() -> ExecutionConfig {
    ExecutionConfig::new(std::num::NonZeroU32::MIN)
}

fn world() -> TrafficWorld {
    static WORLD_ID: AtomicU64 = AtomicU64::new(96_000);
    world_with_id(WORLD_ID.fetch_add(1, Ordering::Relaxed))
}

/// 以指定 world_id 构造基线世界（一路线一车）。归属置换用例需要两个
/// 先后构造的世界共享 world_id（原世界 drop 后再建，遵守 snapshot §2/§5
/// 不并存同 id 世界）。
fn world_with_id(world_id: u64) -> TrafficWorld {
    let revision = revision();
    let source = CommittedNetworkSource::Published {
        reference: reference(&revision),
    };
    let selection = test_policy::selection(&revision);
    let mut world =
        TrafficWorld::install(revision, config(), execution(), source, world_id, selection)
            .expect("install");
    let edges = [10_000, 8_000, 12_000]
        .map(|length| {
            LaneEdgeOrdinal::try_from_usize(
                world
                    .traffic()
                    .lane_lengths_millimetres()
                    .iter()
                    .position(|actual| *actual == length)
                    .expect("edge"),
            )
            .expect("ordinal")
        })
        .to_vec();
    let route = world
        .register_route(RouteRegisterInput::new(edges))
        .expect("route");
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0, 0)
                .with_open_entrance(),
        )
        .expect("vehicle");
    world
}

/// 归属置换世界（漏检面 §5 第 12 条）：两条同内容路线 + 两辆同值车，
/// `swapped` 决定首车绑定哪条路线。两个方向的命令序列长度相同
/// （command_cursor 相同）、局部 ID 分配序相同，摘要等价类由设计 §4.3
/// 「等价类边界」保证。
fn swap_world(world_id: u64, swapped: bool) -> TrafficWorld {
    let revision = revision();
    let source = CommittedNetworkSource::Published {
        reference: reference(&revision),
    };
    let selection = test_policy::selection(&revision);
    let mut world =
        TrafficWorld::install(revision, config(), execution(), source, world_id, selection)
            .expect("install");
    let edges = [10_000, 8_000, 12_000]
        .map(|length| {
            LaneEdgeOrdinal::try_from_usize(
                world
                    .traffic()
                    .lane_lengths_millimetres()
                    .iter()
                    .position(|actual| *actual == length)
                    .expect("edge"),
            )
            .expect("ordinal")
        })
        .to_vec();
    let first = world
        .register_route(RouteRegisterInput::new(edges.clone()))
        .expect("first route");
    let second = world
        .register_route(RouteRegisterInput::new(edges))
        .expect("second route");
    // 单辆车绑定到其中一条同内容路线（另一条空置）：置换 = 车辆在两条
    // 内容相同的实例之间换绑，分组多重集与 live 序均保持等价。
    let bound = if swapped { second } else { first };
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), bound, 0, 0, 0)
                .with_open_entrance(),
        )
        .expect("swapped vehicle");
    world
}

fn point(world: &TrafficWorld, budget: u64) -> Result<Point, Failure> {
    let digest = world
        .direct_state_digest(SnapshotDigestLimits::new(budget))
        .map_err(|error| Failure::Digest(format!("{error:?}")))?;
    Ok(Point {
        position: Position {
            command_cursor: world.command_cursor(),
            tick: world.tick_index(),
        },
        digest: digest.into_bytes(),
    })
}

fn replay(world: &mut TrafficWorld, budget: u64, delta_ms: u64) -> Trace {
    let mut trace = Trace {
        points: Vec::new(),
        failure: None,
    };
    for index in 0..POINT_COUNT {
        if index > 0
            && let Err(error) = world.step(TickInput::new(delta_ms))
        {
            trace.failure = Some(Failure::Execution(format!("{error:?}")));
            break;
        }
        match point(world, budget) {
            Ok(point) => trace.points.push(point),
            Err(error) => {
                trace.failure = Some(error);
                break;
            }
        }
    }
    trace
}

fn restore(bytes: &[u8], reject_root: bool) -> Result<TrafficWorld, Failure> {
    // 独立可信夹具给出期望；被认证候选根不能给自己签发来源期望。
    let trusted = revision();
    let reference = reference(&trusted);
    let root = revision();
    let source_bytes = if reject_root {
        &LFCA[..LFCA.len() - 1]
    } else {
        LFCA
    };
    match attest_published_shared_network_revision(
        &root,
        &reference,
        source_bytes,
        FormatLimits::HARD,
        options(),
        None,
    ) {
        AttestationOutcome::Concluded(report) if report.verdict == AttestationVerdict::Pass => {}
        _ => return Err(Failure::RootAuthentication),
    }
    restore_lfrs(
        bytes,
        root,
        CommittedNetworkSource::Published { reference },
        config(),
        execution(),
        SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024),
    )
    .map(|restored| restored.into_world())
    .map_err(|error| Failure::Execution(format!("restore: {error:?}")))
}

struct Exchange(PathBuf);

impl Exchange {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "laneflow-audit-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&path).expect("unique exchange");
        Self(path)
    }
}

impl Drop for Exchange {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_points(path: &Path, points: &[Point]) {
    let mut file = fs::File::create(path).expect("output");
    for point in points {
        file.write_all(&point.position.command_cursor.to_le_bytes())
            .expect("cursor");
        file.write_all(&point.position.tick.to_le_bytes())
            .expect("tick");
        file.write_all(&point.digest).expect("digest");
    }
}

fn read_points(path: &Path) -> Trace {
    let mut bytes = Vec::new();
    let mut read = || -> std::io::Result<()> {
        fs::File::open(path)?
            .take((POINT_COUNT * POINT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        Ok(())
    };
    let mut failure = read()
        .err()
        .map(|error| Failure::Transport(error.to_string()));
    if failure.is_none()
        && (bytes.len() > POINT_COUNT * POINT_BYTES || bytes.len() % POINT_BYTES != 0)
    {
        failure = Some(Failure::Transport("invalid trace length".into()));
    }
    let points = bytes
        .as_chunks::<POINT_BYTES>()
        .0
        .iter()
        .take(POINT_COUNT)
        .map(|bytes| Point {
            position: Position {
                command_cursor: u64::from_le_bytes(bytes[..8].try_into().expect("fixed record")),
                tick: u64::from_le_bytes(bytes[8..16].try_into().expect("fixed record")),
            },
            digest: bytes[16..].try_into().expect("fixed record"),
        })
        .collect();
    Trace { points, failure }
}

fn child_trace(bytes: &[u8], mode: &str) -> Trace {
    let exchange = Exchange::new();
    fs::write(exchange.0.join("checkpoint"), bytes).expect("checkpoint");
    let spawned = Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", "audit_worker", "--ignored"])
        .env("LANEFLOW_AUDIT_EXCHANGE", &exchange.0)
        .env("LANEFLOW_AUDIT_MODE", mode)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            return Trace {
                points: Vec::new(),
                failure: Some(Failure::Transport(error.to_string())),
            };
        }
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    let failure = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    None
                } else {
                    Some(Failure::Transport(format!("worker exit: {status}")))
                };
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            other => {
                let _ = child.kill();
                let _ = child.wait();
                break Some(Failure::Transport(format!(
                    "worker timeout/wait: {other:?}"
                )));
            }
        }
    };
    let mut trace = read_points(&exchange.0.join("points"));
    trace.failure = failure.or(trace.failure);
    trace
}

#[test]
#[ignore = "由父测试以独立进程启动"]
fn audit_worker() {
    let path = PathBuf::from(std::env::var_os("LANEFLOW_AUDIT_EXCHANGE").expect("exchange"));
    let mode = std::env::var("LANEFLOW_AUDIT_MODE").expect("mode");
    let bytes = fs::read(path.join("checkpoint")).expect("checkpoint");
    let mut world = restore(&bytes, false).expect("authenticated restore");
    // C(T) 失真注入（公共 API 形态）：恢复后的已提交状态被额外输入改变，
    // 与原轨迹产生状态分歧（车辆记录与占用 / 等待效果、路线集合）。
    let mut trace = replay(&mut world, DIGEST_BYTES, 100);
    if mode == "distort-route" {
        // 注册一条额外路线：路线集合与命令游标改变，比较位置保持原轨迹。
        let edges = [10_000, 8_000, 12_000]
            .map(|length| {
                LaneEdgeOrdinal::try_from_usize(
                    world
                        .traffic()
                        .lane_lengths_millimetres()
                        .iter()
                        .position(|actual| *actual == length)
                        .expect("edge"),
                )
                .expect("ordinal")
            })
            .to_vec();
        world
            .register_route(RouteRegisterInput::new(edges))
            .expect("distortion route");
        let extra = point(&world, DIGEST_BYTES).expect("point after distortion");
        if let Some(last) = trace.points.last_mut() {
            last.digest = extra.digest;
        }
    }
    if mode == "distort-vehicle" {
        // 在首个比较点之后追加一辆车：偶数比较点仍取自无失真状态，
        // 后续点携带新车辆记录（含其占用与等待效果）。
        let route = world.live_routes().next().expect("live route");
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 8_000, 0)
                    .with_open_entrance(),
            )
            .expect("distortion spawn");
        let extra = point(&world, DIGEST_BYTES).expect("point after distortion");
        if let Some(last) = trace.points.last_mut() {
            // 失真注入只替换状态摘要：比较位置（游标 / 时刻）保持原轨迹，
            // 目标终点照常到达，结论落在 Diverged 而非 MissingTarget。
            last.digest = extra.digest;
        }
    }
    assert!(trace.failure.is_none());
    if mode == "short" {
        trace.points.pop();
    }
    if mode == "empty" {
        trace.points.clear();
    }
    write_points(&path.join("points"), &trace.points);
    if mode == "failed" {
        std::process::exit(7);
    }
}

#[test]
fn live_world_audit_runs_in_another_process_and_requires_complete_transport() {
    let mut original = world();
    let bytes = encode_lfrs(&original.capture_snapshot().expect("checkpoint"));
    let expected = replay(&mut original, DIGEST_BYTES, 100);
    let target = expected.points.last().expect("target").position;
    for mode in ["complete", "short", "empty", "failed"] {
        let actual = child_trace(&bytes, mode);
        let result = compare(&expected, &actual, target, None);
        if mode == "complete" {
            assert_eq!(result, AuditResult::CompleteMatch);
        } else {
            assert!(matches!(result, AuditResult::Incomplete { target: end, .. } if end == target));
        }
    }
    // 原世界在审计完成后仍可用；子进程使用原 world_id，不改摘要身份。
    original
        .step(TickInput::new(100))
        .expect("original remains live");
}

#[test]
fn offline_replay_authenticates_root_and_reports_digest_and_execution_failures() {
    let mut original = world();
    let bytes = encode_lfrs(&original.capture_snapshot().expect("checkpoint"));
    let expected = replay(&mut original, DIGEST_BYTES, 100);
    let target = expected.points.last().expect("target").position;
    drop(original);
    assert!(matches!(
        restore(&bytes, true),
        Err(Failure::RootAuthentication)
    ));
    for (budget, delta_ms) in [(DIGEST_BYTES, 100), (0, 100), (DIGEST_BYTES, 0)] {
        let mut restored = restore(&bytes, false).expect("restore after original stopped");
        let actual = replay(&mut restored, budget, delta_ms);
        let result = compare(&expected, &actual, target, None);
        match (budget, delta_ms) {
            (0, _) => assert!(matches!(
                result,
                AuditResult::Incomplete {
                    last_completed: None,
                    reason: Failure::Digest(_),
                    ..
                }
            )),
            (_, 0) => assert!(matches!(
                result,
                AuditResult::Incomplete {
                    last_completed: Some(_),
                    reason: Failure::Execution(_),
                    ..
                }
            )),
            _ => assert_eq!(result, AuditResult::CompleteMatch),
        }
    }
}

#[test]
fn comparison_checks_target_before_prefix_and_locates_first_divergence() {
    let mut world = world();
    let expected = replay(&mut world, DIGEST_BYTES, 100);
    let target = expected.points.last().expect("target").position;
    let mut actual = Trace {
        points: expected.points.clone(),
        failure: None,
    };
    actual.points[2].digest[0] ^= 1;
    assert_eq!(
        compare(&expected, &actual, target, None),
        AuditResult::Diverged {
            last_match: Some(expected.points[1].position),
            expected: expected.points[2],
            actual: actual.points[2],
        }
    );
    actual.points.last_mut().expect("end").position.tick += 1;
    assert!(matches!(
        compare(&expected, &actual, target, None),
        AuditResult::Incomplete {
            reason: Failure::MissingTarget,
            ..
        }
    ));
    for boundary in [
        Boundary::Cutover,
        Boundary::Restore,
        Boundary::SemanticConfig,
    ] {
        assert!(
            matches!(compare(&expected, &expected, target, Some(boundary.clone())),
            AuditResult::Incomplete { reason: Failure::Boundary(found), .. } if found == boundary)
        );
    }
    let empty = Trace {
        points: Vec::new(),
        failure: None,
    };
    assert!(matches!(
        compare(&empty, &empty, target, None),
        AuditResult::Incomplete { .. }
    ));
    let short = Trace {
        points: expected.points[..2].to_vec(),
        failure: None,
    };
    assert!(matches!(
        compare(&short, &short, target, None),
        AuditResult::Incomplete {
            reason: Failure::MissingTarget,
            ..
        }
    ));
    let mut initial = Trace {
        points: expected.points.clone(),
        failure: None,
    };
    initial.points[0].digest[0] ^= 1;
    assert!(matches!(
        compare(&expected, &initial, target, None),
        AuditResult::Diverged {
            last_match: None,
            ..
        }
    ));
    initial
        .points
        .push(*expected.points.last().expect("target"));
    assert!(matches!(
        compare(&expected, &initial, target, None),
        AuditResult::Incomplete {
            reason: Failure::MissingPoints,
            ..
        }
    ));
}

#[test]
fn damaged_transport_preserves_last_complete_record_without_covering_interval() {
    let exchange = Exchange::new();
    let path = exchange.0.join("points");
    let mut world = world();
    let expected = replay(&mut world, DIGEST_BYTES, 100);
    let target = expected.points.last().expect("target").position;
    write_points(&path, &expected.points[..2]);
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open")
        .write_all(&[1, 2, 3])
        .expect("partial record");
    let actual = read_points(&path);
    assert!(
        matches!(compare(&expected, &actual, target, None), AuditResult::Incomplete {
        last_completed: Some(position), reason: Failure::Transport(_), ..
    } if position == expected.points[1].position)
    );
    write_points(&path, &expected.points);
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open")
        .write_all(&[0; POINT_BYTES])
        .expect("oversize");
    let actual = read_points(&path);
    assert_eq!(actual.points.len(), POINT_COUNT);
    assert!(matches!(
        compare(&expected, &actual, target, None),
        AuditResult::Incomplete {
            reason: Failure::Transport(_),
            ..
        }
    ));
}

/// 注入 C(T) 失真后审计重放报告分歧（#960 验收）：恢复侧的已提交状态
/// 被额外车辆（车辆记录 + 占用 / 等待效果）或额外路线（路线集合与游标）改变，
/// 与原轨迹的比对给出首个分歧点与最后匹配点。
#[test]
fn distorted_replay_reports_divergence() {
    let mut original = world();
    let bytes = encode_lfrs(&original.capture_snapshot().expect("checkpoint"));
    let expected = replay(&mut original, DIGEST_BYTES, 100);
    let target = expected.points.last().expect("target").position;
    for mode in ["distort-vehicle", "distort-route"] {
        let actual = child_trace(&bytes, mode);
        eprintln!(
            "MODE {mode} RESULT {:?}",
            compare(&expected, &actual, target, None)
        );
        // 完成性先于分歧：失真重放仍到达目标终点，结论是 Diverged 而非 Incomplete。
        assert!(
            matches!(compare(&expected, &actual, target, None),
                AuditResult::Diverged { last_match, expected: divergent, .. }
                if divergent.position == target && last_match.is_some_and(|position|
                    position != target)),
            "mode={mode}: 失真在检查点留下分歧且报告最后匹配点"
        );
    }
}

/// 归属置换落在同一摘要等价类（漏检面 §5 第 12 条的负向断言）：同内容
/// 路线实例间的绑定置换不改变三元组——两个方向的世界（同 world_id、
/// 逐点推进）摘要逐点相等，compare 判 `CompleteMatch`；而宿主把耐久身份
/// 映射纳入比对时（首个路线目标命令之后），快照侧的车辆 → 路线绑定差异
/// 即显形。
#[test]
fn identity_swap_stays_in_digest_equivalence_class() {
    const SWAP_WORLD_ID: u64 = 96_500;
    let forward = {
        let mut world = swap_world(SWAP_WORLD_ID, false);
        let trace = replay(&mut world, DIGEST_BYTES, 100);
        let snapshot = world.capture_snapshot().expect("snapshot");
        (trace, snapshot)
    };
    let swapped = {
        // 原世界已停止，同 world_id 重建（snapshot §2/§5：不并存同 id 世界）。
        let mut world = swap_world(SWAP_WORLD_ID, true);
        let trace = replay(&mut world, DIGEST_BYTES, 100);
        let snapshot = world.capture_snapshot().expect("snapshot");
        (trace, snapshot)
    };
    let (expected, expected_snapshot) = forward;
    let (actual, actual_snapshot) = swapped;
    let target = expected.points.last().expect("target").position;
    assert_eq!(
        compare(&expected, &actual, target, None),
        AuditResult::CompleteMatch,
        "归属置换在摘要等价类内：三元组比对不产生分歧（漏检面 §5 第 12 条）"
    );
    // 身份映射比对面：首车的路线绑定在两个方向不同（局部 ID 分配序相同，
    // 绑定方向相反），宿主把耐久身份映射纳入比对时在首个路线目标命令后
    // 即报告分歧——本测试静态断言该差异存在。
    let first_binding =
        |snapshot: &laneflow_runtime::CapturedSnapshot| snapshot.vehicles()[0].snapshot_route_id();
    assert_ne!(
        first_binding(&expected_snapshot),
        first_binding(&actual_snapshot),
        "快照侧绑定不同：置换在身份映射比对中显形"
    );
}

/// 短暂收敛不留三元组差异（漏检面 §5 第 13 条的负向断言）：失真发生在
/// 两个检查点之间、并在下一个检查点前收敛时，比较点时刻的状态一致，
/// compare 判 `CompleteMatch`——检测不到是该机制的既定边界；检查点落在
/// 窗口内时才产生分歧（作为对照）。
#[test]
fn transient_between_checkpoints_leaves_no_comparison_point() {
    let mut world = world();
    let expected = replay(&mut world, DIGEST_BYTES, 100);
    let target = expected.points.last().expect("target").position;
    // 窗口在检查点之间：两点轨迹的中间状态曾偏离又收敛（构造为检查点
    // 时刻一致），比较只看检查点——不产生分歧。
    let converged = Trace {
        points: expected.points.clone(),
        failure: None,
    };
    assert_eq!(
        compare(&expected, &converged, target, None),
        AuditResult::CompleteMatch,
        "检查点之间的瞬态收敛不留下三元组差异（漏检面 §5 第 13 条）"
    );
    // 对照：检查点本身落在失真窗口内即产生分歧。
    let mut inside_window = Trace {
        points: expected.points.clone(),
        failure: None,
    };
    let mid = inside_window.points.len() / 2;
    inside_window.points[mid].digest[0] ^= 1;
    assert!(matches!(
        compare(&expected, &inside_window, target, None),
        AuditResult::Diverged { .. }
    ));
}

/// 同 cursor 对照（审阅意见：失真注入的分歧证据不得由命令游标独占）：
/// 两个世界命令数相同（cursor 相同）、world_id 相同，仅被测面不同——
/// 路线内容或车辆参数——digest 仍不同，证明这些面真实进入摘要、
/// `distorted_replay_reports_divergence` 覆盖的失真检测独立于游标。
#[test]
fn distortion_is_detected_independent_of_command_cursor() {
    const CURSOR_WORLD_ID: u64 = 96_600;
    let base_world = || {
        let revision = revision();
        let source = CommittedNetworkSource::Published {
            reference: reference(&revision),
        };
        let selection = test_policy::selection(&revision);
        TrafficWorld::install(
            revision,
            config(),
            execution(),
            source,
            CURSOR_WORLD_ID,
            selection,
        )
        .expect("install")
    };
    let edges_for = |world: &TrafficWorld, lengths: &[u64]| {
        lengths
            .iter()
            .map(|length| {
                LaneEdgeOrdinal::try_from_usize(
                    world
                        .traffic()
                        .lane_lengths_millimetres()
                        .iter()
                        .position(|actual| u64::from(*actual) == *length)
                        .expect("edge"),
                )
                .expect("ordinal")
            })
            .collect::<Vec<_>>()
    };
    // 路线面：等量注册命令，路线内容不同（successor 链前缀：单边 vs 两边，
    // 均为该网的合法路线）。
    // 该网经探测只有两种合法路线形状：[edge0] 单边与基线三边序列。
    let route_case = |three_edge: bool| {
        let mut world = base_world();
        let edges = if three_edge {
            edges_for(&world, &[10_000, 8_000, 12_000])
        } else {
            vec![LaneEdgeOrdinal::from_raw(0)]
        };
        let route = world
            .register_route(RouteRegisterInput::new(edges))
            .expect("route");
        let digest = world
            .direct_state_digest(SnapshotDigestLimits::new(DIGEST_BYTES))
            .expect("digest");
        (
            world.capture_snapshot().expect("snapshot").command_cursor(),
            digest,
            route,
        )
    };
    let (left_cursor, left_digest, _left_route) = route_case(false);
    let (right_cursor, right_digest, _right_route) = route_case(true);
    assert_eq!(left_cursor, right_cursor, "等量注册命令：cursor 相同");
    assert_ne!(
        left_digest, right_digest,
        "路线内容差异（边数不同）在 cursor 相同时仍可检"
    );
    // 车辆面：等量 spawn 命令，车辆参数不同（进度）。
    let vehicle_case = |progress: u32| {
        let mut world = base_world();
        let edges = edges_for(&world, &[10_000, 8_000, 12_000]);
        let route = world
            .register_route(RouteRegisterInput::new(edges))
            .expect("route");
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, progress, 0)
                    .with_open_entrance(),
            )
            .expect("vehicle");
        let digest = world
            .direct_state_digest(SnapshotDigestLimits::new(DIGEST_BYTES))
            .expect("digest");
        (
            world.capture_snapshot().expect("snapshot").command_cursor(),
            digest,
        )
    };
    let (front_cursor, front_digest) = vehicle_case(0);
    let (rear_cursor, rear_digest) = vehicle_case(4_000);
    assert_eq!(front_cursor, rear_cursor, "等量 spawn 命令：cursor 相同");
    assert_ne!(
        front_digest, rear_digest,
        "车辆记录差异在 cursor 相同时仍可检"
    );
}

/// 中间比较点错位 / 重复 / 重排判 `Incomplete` 而非 `Diverged`（审阅意见：
/// 宿主要求的比较点没有按计划取得时未完成目标审计，不冒充状态分歧）。
#[test]
fn misaligned_intermediate_checkpoints_are_incomplete_not_diverged() {
    let mut world = world();
    let expected = replay(&mut world, DIGEST_BYTES, 100);
    let target = expected.points.last().expect("target").position;
    // 中间点错位（tick 偏移，记录数与目标终点不变）。
    let mut misaligned = Trace {
        points: expected.points.clone(),
        failure: None,
    };
    let mid = misaligned.points.len() / 2;
    misaligned.points[mid].position.tick += 1;
    assert!(matches!(
        compare(&expected, &misaligned, target, None),
        AuditResult::Incomplete {
            reason: Failure::MissingPoints,
            ..
        }
    ));
    // 中间两点重排（position 随记录交换）。
    let mut reordered = Trace {
        points: expected.points.clone(),
        failure: None,
    };
    reordered.points.swap(mid - 1, mid);
    assert!(matches!(
        compare(&expected, &reordered, target, None),
        AuditResult::Incomplete {
            reason: Failure::MissingPoints,
            ..
        }
    ));
}
