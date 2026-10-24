//! #963：真实认证与在途 step 的宿主停止边界；本模块中的调度与输出策略仅为测试宿主。

use std::{
    num::NonZeroU32,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
    thread,
    time::Duration,
};

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_spatial::{
    CanonicalPoseBatch, FramePlacementToken, PoseInput, PoseRecordId, SpatialSession,
};
use laneflow_static_contract::{LaneEdgeOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::{
    AttestationCause, AttestationOutcome, AttestationVerdict, Disposition, OperationFailureKind,
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

use crate::{
    CommittedNetworkSource, ExecutionConfig, ObservationExportMode, ObservationSelection,
    PublishedLfcaReference, RouteRegisterInput, StepOutcome, TickInput, TrafficWorld,
    VehicleSpawnInput, WorldConfig, attest_published_shared_network_revision,
};

const LFCA: &[u8] = include_bytes!(
    "../../../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
);
const OTHER_LFCA: &[u8] = include_bytes!(
    "../../../../laneflow-compiler/tests/fixtures/portable/lfca-variants/min-headless.lfca"
);
const WAIT: Duration = Duration::from_secs(30);

fn options() -> SharedNetworkBuildOptions<'static> {
    SharedNetworkBuildOptions::new(
        SpatialBuildOption::RetainAvailable,
        SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
    )
}

fn reference(bytes: &[u8]) -> PublishedLfcaReference {
    let input = check_canonical_network_input(bytes, FormatLimits::HARD).unwrap();
    PublishedLfcaReference::new(
        "fixture://audit-step-overlap",
        input.canonical_artifact_digest(),
        input.canonical_artifact_byte_length(),
        input.network_revision(),
    )
    .unwrap()
}

fn world(root: &Arc<SharedNetworkRevision>, workers: u32) -> TrafficWorld {
    static NEXT_WORLD_ID: AtomicU64 = AtomicU64::new(96_300_000);
    let mut world = TrafficWorld::install(
        root.clone(),
        WorldConfig::new(8, 4, 1_024, 1_024, 100),
        ExecutionConfig::new(NonZeroU32::new(workers).unwrap()),
        CommittedNetworkSource::Published {
            reference: reference(LFCA),
        },
        NEXT_WORLD_ID.fetch_add(1, Ordering::Relaxed),
        crate::test_policy::selection(root),
    )
    .unwrap();
    let edges = [10_000, 8_000, 12_000].map(|length| {
        LaneEdgeOrdinal::try_from_usize(
            world
                .traffic()
                .lane_lengths_millimetres()
                .iter()
                .position(|actual| *actual == length)
                .expect("fixture path edge"),
        )
        .unwrap()
    });
    let route = world
        .register_route(RouteRegisterInput::new(edges.to_vec()))
        .unwrap();
    world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0, 0)
                .with_open_entrance(),
        )
        .unwrap();
    world
}

#[derive(Clone, Copy, Debug)]
enum Case {
    Pass,
    Source,
    Operational,
    Disable,
}

impl Case {
    fn audit(self, root: &SharedNetworkRevision) -> AttestationOutcome {
        let mut damaged = LFCA.to_vec();
        damaged[0] ^= 0xFF;
        let (bytes, expected, build) = match self {
            Self::Pass => (LFCA, reference(LFCA), options()),
            Self::Source => (damaged.as_slice(), reference(LFCA), options()),
            Self::Operational => (
                LFCA,
                reference(LFCA),
                SharedNetworkBuildOptions::new(
                    SpatialBuildOption::RetainAvailable,
                    SharedNetworkBuildLimits::new(1, 1),
                ),
            ),
            // 来源与引用一起替换为另一份自洽输入；不是在用修订物理失真的注入。
            Self::Disable => (OTHER_LFCA, reference(OTHER_LFCA), options()),
        };
        let outcome = attest_published_shared_network_revision(
            root,
            &expected,
            bytes,
            FormatLimits::HARD,
            build,
            None,
        );
        match (&outcome, self) {
            (AttestationOutcome::Concluded(report), Self::Pass) => {
                assert_eq!(report.verdict, AttestationVerdict::Pass);
                assert_eq!(report.disposition, Disposition::None);
            }
            (AttestationOutcome::Concluded(report), Self::Source) => {
                assert_eq!(report.verdict, AttestationVerdict::Mismatch);
                assert_eq!(report.cause, Some(AttestationCause::Source));
                assert_eq!(report.disposition, Disposition::IsolateSource);
            }
            (AttestationOutcome::Concluded(report), Self::Disable) => {
                assert_eq!(report.verdict, AttestationVerdict::Mismatch);
                assert_eq!(report.cause, Some(AttestationCause::Indeterminate));
                assert_eq!(report.disposition, Disposition::DisableRoot);
            }
            (AttestationOutcome::OperationFailed(failure), Self::Operational) => {
                assert_eq!(failure.kind, OperationFailureKind::BuilderBudgetExceeded);
            }
            _ => panic!("unexpected audit for {self:?}: {outcome:?}"),
        }
        outcome
    }
}

// 测试宿主由同一协调器裁决新调用；这是调用方示例，不是通用的多发起者停用协议。
#[derive(Default)]
struct Admission {
    stopped: AtomicBool,
    calls: AtomicUsize,
}

impl Admission {
    fn call<T>(&self, operation: impl FnOnce() -> T) -> Option<T> {
        if self.stopped.load(Ordering::Acquire) {
            return None;
        }
        self.calls.fetch_add(1, Ordering::Relaxed);
        Some(operation())
    }
}

#[derive(Default)]
struct Host {
    admission: Arc<Admission>,
    source_isolated: bool,
    retry: bool,
    published: Vec<StepOutcome>,
    quarantined: Vec<StepOutcome>,
}

impl Host {
    fn call<T>(&self, operation: impl FnOnce() -> T) -> Option<T> {
        self.admission.call(operation)
    }

    fn accept_audit(&mut self, outcome: &AttestationOutcome) {
        match outcome {
            AttestationOutcome::Concluded(report) => match report.disposition {
                Disposition::DisableRoot => self.admission.stopped.store(true, Ordering::Release),
                Disposition::IsolateSource => self.source_isolated = true,
                Disposition::None => {}
                Disposition::Retry => panic!("Retry belongs to operational failure"),
            },
            AttestationOutcome::OperationFailed(_) => self.retry = true,
        }
    }

    fn accept_returned_step(&mut self, outcome: StepOutcome) {
        if self.admission.stopped.load(Ordering::Acquire) {
            self.quarantined.push(outcome);
        } else {
            self.published.push(outcome);
        }
    }
}

struct ResumeOnDrop(Option<SyncSender<()>>);

impl ResumeOnDrop {
    fn resume(&mut self) {
        self.0.take().unwrap().send(()).unwrap();
    }
}

impl Drop for ResumeOnDrop {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.try_send(());
        }
    }
}

fn exercise(case: Case) {
    for workers in [1, 2] {
        let input = check_canonical_network_input(LFCA, FormatLimits::HARD).unwrap();
        let root = build_shared_network_revision(input, options()).unwrap();
        let mut in_flight = world(&root, workers);
        let mut other_world = world(&root, workers);
        let mut spatial = SpatialSession::bind(root.clone()).unwrap().unwrap();
        assert!(Arc::ptr_eq(&root, &in_flight.revision()));
        assert!(Arc::ptr_eq(&root, &other_world.revision()));
        assert!(Arc::ptr_eq(&root, &spatial.revision()));

        // 结论交付前已发布的非空批次留在宿主，停用不会撤销它们。
        let mut prior_step = None;
        for _ in 0..256 {
            prior_step = Some(other_world.step(TickInput::new(100)).unwrap());
            if !other_world.latest_transition_events().is_empty() {
                break;
            }
        }
        let events = other_world.latest_transition_events().to_vec();
        assert!(!events.is_empty(), "fixture must publish a transition");
        let mut observations = other_world
            .open_observation_export(ObservationSelection::AllLaneEdges)
            .unwrap();
        let observation = other_world
            .export_observation(&mut observations, ObservationExportMode::Full)
            .unwrap();
        assert!(!observation.rows().is_empty());
        let pose_inputs = [PoseInput::lane(
            PoseRecordId::new(1),
            LaneEdgeOrdinal::from_raw(0),
            0,
        )];
        let mut pose = CanonicalPoseBatch::new();
        spatial
            .extract_pose_batch(FramePlacementToken::new(1), &pose_inputs, &mut pose)
            .unwrap();
        assert_eq!(pose.records().len(), 1);
        let published_outputs = (events, observation, pose);
        let published_before = format!("{published_outputs:?}");
        let mut host = Host::default();
        host.published.push(prior_step.unwrap());
        let prior_published = host.published.clone();

        let (entered_send, entered) = mpsc::sync_channel(1);
        let (resume_send, resume) = mpsc::sync_channel(1);
        let (returned_send, returned) = mpsc::sync_channel(1);
        thread::scope(|scope| {
            let mut release = ResumeOnDrop(Some(resume_send));
            let admission = host.admission.clone();
            let in_flight_world = &mut in_flight;
            let worker = scope.spawn(move || {
                let caller = thread::current().id();
                crate::kernel::tick::before_next_commit(move || {
                    entered_send.send((caller, thread::current().id())).unwrap();
                    resume
                        .recv_timeout(WAIT)
                        .expect("host must release in-flight step");
                });
                let result = admission
                    .call(|| in_flight_world.step(TickInput::new(100)))
                    .unwrap();
                returned_send.send(()).unwrap();
                result
            });
            let (caller, executor) = entered
                .recv_timeout(WAIT)
                .expect("step must reach its pre-commit boundary");
            assert_eq!(caller == executor, workers == 1);
            let audit = case.audit(&root);
            assert!(matches!(
                returned.try_recv(),
                Err(mpsc::TryRecvError::Empty)
            ));
            // 下面需要独占调度状态；在途调用只持有 stopped/calls 两个原子值。
            host.accept_audit(&audit);
            let stopped = matches!(case, Case::Disable);
            assert_eq!(host.admission.stopped.load(Ordering::Acquire), stopped);
            assert_eq!(host.source_isolated, matches!(case, Case::Source));
            assert_eq!(host.retry, matches!(case, Case::Operational));
            let calls_before = host.admission.calls.load(Ordering::Relaxed);
            let second_step = host.call(|| other_world.step(TickInput::new(100)));
            let next_observation = host.call(|| {
                other_world.export_observation(&mut observations, ObservationExportMode::Full)
            });
            let mut next_pose = CanonicalPoseBatch::new();
            let next_spatial = host.call(|| {
                spatial.extract_pose_batch(
                    FramePlacementToken::new(2),
                    &pose_inputs,
                    &mut next_pose,
                )
            });
            assert_eq!(second_step.is_none(), stopped);
            assert_eq!(next_observation.is_none(), stopped);
            assert_eq!(next_spatial.is_none(), stopped);
            if stopped {
                assert_eq!(host.admission.calls.load(Ordering::Relaxed), calls_before);
            } else {
                second_step.unwrap().unwrap();
                next_observation.unwrap().unwrap();
                next_spatial.unwrap().unwrap();
                assert_eq!(
                    host.admission.calls.load(Ordering::Relaxed),
                    calls_before + 3
                );
            }
            release.resume();
            let outcome = worker
                .join()
                .unwrap()
                .expect("audit must not abort an already-started step");
            returned.recv_timeout(WAIT).unwrap();
            assert_eq!(outcome.tick_index(), 1);
            assert_eq!(outcome.time_ms(), 100);
            host.accept_returned_step(outcome);
        });
        assert_eq!(format!("{published_outputs:?}"), published_before);
        if matches!(case, Case::Disable) {
            assert_eq!(host.published, prior_published);
            assert_eq!(host.quarantined.len(), 1);
            // 连同刚返回的世界也阻止下一拍，不靠 Runtime 自有失效标志。
            assert!(host.call(|| in_flight.step(TickInput::new(100))).is_none());
        } else {
            assert!(host.quarantined.is_empty());
            assert_eq!(host.published.len(), prior_published.len() + 1);
            assert_eq!(
                host.call(|| in_flight.step(TickInput::new(100)))
                    .unwrap()
                    .unwrap()
                    .tick_index(),
                2
            );
        }
    }
}

#[test]
fn pass_audit_overlaps_step_without_stopping_consumers() {
    exercise(Case::Pass);
}

#[test]
fn source_mismatch_isolates_input_while_in_flight_step_finishes() {
    exercise(Case::Source);
}

#[test]
fn operational_failure_retries_without_stopping_consumers() {
    exercise(Case::Operational);
}

#[test]
fn disabling_audit_stops_new_calls_and_quarantines_late_step_output() {
    exercise(Case::Disable);
}
