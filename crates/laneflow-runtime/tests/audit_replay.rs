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
    let revision = revision();
    let source = CommittedNetworkSource::Published {
        reference: reference(&revision),
    };
    let selection = test_policy::selection(&revision);
    let mut world = TrafficWorld::install(
        revision,
        config(),
        execution(),
        source,
        WORLD_ID.fetch_add(1, Ordering::Relaxed),
        selection,
    )
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
    let mut trace = replay(&mut world, DIGEST_BYTES, 100);
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
