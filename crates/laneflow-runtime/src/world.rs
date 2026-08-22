use std::sync::Arc;

use laneflow_static_contract::{
    EntityKind, ParkingSpaceOrdinal, SignalAspect, SignalControllerOrdinal, SignalGroupOrdinal,
    StaticRouteOrdinal,
};
use laneflow_static_network::SharedNetworkRevision;

use crate::{
    CommittedPoseSourceBatch, CommittedSignalGroupBatch, InstallError, LookupError, RouteHandle,
    StepError, StepOutcome, TickInput, VehicleHandle, WorldConfig,
};

/// 1-worker 交通世界。只克隆根 `Arc`，不复制静态 component。
pub struct TrafficWorld {
    revision: Arc<SharedNetworkRevision>,
    config: WorldConfig,
    tick_index: u64,
    time_ms: u64,
    signal_aspects: Box<[SignalAspect]>,
}

impl TrafficWorld {
    /// 安装完整共享根。失败不留下可观察的半个 world。
    pub fn install(
        revision: Arc<SharedNetworkRevision>,
        config: WorldConfig,
    ) -> Result<Self, InstallError> {
        if config.fixed_delta_time_ms() == 0 {
            return Err(InstallError::NonPositiveDelta);
        }
        if config.worker_count() != 1 {
            return Err(InstallError::WorkerCountNotOne);
        }
        validate_signal_programs(revision.as_ref(), config.fixed_delta_time_ms())?;
        let group_count = usize::try_from(
            revision
                .traffic()
                .entity_counts()
                .count(EntityKind::SignalGroup),
        )
        .expect("signal group count fits usize");
        let mut world = Self {
            revision,
            config,
            tick_index: 0,
            time_ms: 0,
            signal_aspects: vec![SignalAspect::Red; group_count].into_boxed_slice(),
        };
        world.refresh_signals();
        Ok(world)
    }

    /// 共享根。
    #[must_use]
    pub fn revision(&self) -> Arc<SharedNetworkRevision> {
        Arc::clone(&self.revision)
    }

    /// 共享 Traffic component。
    #[must_use]
    pub fn traffic(&self) -> &laneflow_static_network::SharedTrafficNetwork {
        self.revision.traffic()
    }

    #[must_use]
    pub const fn tick_index(&self) -> u64 {
        self.tick_index
    }

    #[must_use]
    pub const fn time_ms(&self) -> u64 {
        self.time_ms
    }

    #[must_use]
    pub const fn config(&self) -> WorldConfig {
        self.config
    }

    /// 取得 compiler 预编译静态路线句柄。
    pub fn static_route(&self, route: StaticRouteOrdinal) -> Result<RouteHandle, LookupError> {
        let count = self
            .revision
            .traffic()
            .entity_counts()
            .count(EntityKind::StaticRoute);
        if route.raw() >= count {
            return Err(LookupError::UnknownStaticRoute);
        }
        Ok(RouteHandle::static_route(route.raw()))
    }

    /// 固定步进。失败不推进时间，已提交查询与失败前一致。
    pub fn step(&mut self, input: TickInput) -> Result<StepOutcome, StepError> {
        let expected = self.config.fixed_delta_time_ms();
        if input.delta_time_ms != expected {
            return Err(StepError::DeltaMismatch {
                expected_delta_time_ms: expected,
                actual_delta_time_ms: input.delta_time_ms,
            });
        }
        let tick_index = self.tick_index.checked_add(1).ok_or(StepError::Overflow)?;
        let time_ms = self
            .time_ms
            .checked_add(expected)
            .ok_or(StepError::Overflow)?;
        self.tick_index = tick_index;
        self.time_ms = time_ms;
        self.refresh_signals();
        Ok(StepOutcome::new(tick_index, time_ms))
    }

    /// 稳定顺序的已提交 pose 源。尚无车辆时为空。
    #[must_use]
    pub fn committed_pose_sources(&self) -> CommittedPoseSourceBatch {
        CommittedPoseSourceBatch::default()
    }

    /// 按停车位序号读占用者。
    #[must_use]
    pub fn committed_parking_occupant(&self, space: ParkingSpaceOrdinal) -> Option<VehicleHandle> {
        let count = self
            .revision
            .traffic()
            .entity_counts()
            .count(EntityKind::ParkingSpace);
        if space.raw() >= count {
            return None;
        }
        None
    }

    /// 稳定按组序号的当前 aspect。
    #[must_use]
    pub fn committed_signal_groups(&self) -> CommittedSignalGroupBatch {
        let items = self
            .signal_aspects
            .iter()
            .enumerate()
            .map(|(index, aspect)| {
                (
                    SignalGroupOrdinal::from_raw(
                        u32::try_from(index).expect("signal group index fits u32"),
                    ),
                    *aspect,
                )
            })
            .collect();
        CommittedSignalGroupBatch { items }
    }

    fn refresh_signals(&mut self) {
        self.signal_aspects.fill(SignalAspect::Red);
        let relations = self.revision.traffic().relations();
        let controller_count = self
            .revision
            .traffic()
            .entity_counts()
            .count(EntityKind::SignalController);
        for raw in 0..controller_count {
            let controller = SignalControllerOrdinal::from_raw(raw);
            let Some(view) = relations.signal_controller(controller) else {
                continue;
            };
            let cycle_ms = view.cycle_ms();
            if cycle_ms == 0 || view.phases().is_empty() {
                continue;
            }
            let position = u64::try_from(
                (u128::from(self.time_ms) + u128::from(view.offset_ms())) % u128::from(cycle_ms),
            )
            .expect("cycle position fits u64");
            let phases = view.phases();
            let phase_index = phases.partition_point(|phase| {
                relations.phase_end_offset_ms(*phase).unwrap_or(0) <= position
            });
            let Some(phase) = phases.get(phase_index).copied() else {
                continue;
            };
            let Some((groups, aspects)) = relations.phase_states(phase) else {
                continue;
            };
            for (group, aspect) in groups.iter().copied().zip(aspects.iter().copied()) {
                if let Some(slot) = self.signal_aspects.get_mut(group.index()) {
                    *slot = aspect;
                }
            }
        }
    }
}

fn validate_signal_programs(
    revision: &SharedNetworkRevision,
    fixed_delta_time_ms: u64,
) -> Result<(), InstallError> {
    let relations = revision.traffic().relations();
    let controller_count = revision
        .traffic()
        .entity_counts()
        .count(EntityKind::SignalController);
    for raw in 0..controller_count {
        let controller = SignalControllerOrdinal::from_raw(raw);
        let Some(view) = relations.signal_controller(controller) else {
            return Err(InstallError::InvalidSignalProgram);
        };
        if view.cycle_ms() == 0 || view.phases().is_empty() {
            return Err(InstallError::InvalidSignalProgram);
        }
        for phase in view.phases() {
            let Some(duration_ms) = relations.phase_duration_ms(*phase) else {
                return Err(InstallError::InvalidSignalProgram);
            };
            if duration_ms < fixed_delta_time_ms {
                return Err(InstallError::PhaseShorterThanTick);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod overflow_tests {
    use super::*;
    use laneflow_format::{FormatLimits, check_canonical_network_input_v1};
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };

    const FULL_SPATIAL: &[u8] = include_bytes!(
        "../../laneflow-compiler/tests/fixtures/portable-v1/lfca-v1-full-spatial/expected.lfca"
    );

    fn world() -> TrafficWorld {
        let input = check_canonical_network_input_v1(FULL_SPATIAL, FormatLimits::V1_HARD)
            .expect("checked canonical network input");
        let revision = build_shared_network_revision(
            input,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::RetainAvailable,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .expect("shared network revision");
        TrafficWorld::install(revision, WorldConfig::new(8, 4, 1, 100)).expect("install")
    }

    #[test]
    fn step_rejects_tick_and_time_overflow() {
        let mut world = world();
        world.tick_index = u64::MAX;
        world.time_ms = 0;
        assert_eq!(
            world.step(TickInput::new(100)).unwrap_err(),
            StepError::Overflow
        );
        assert_eq!(world.tick_index, u64::MAX);
        assert_eq!(world.time_ms, 0);

        world.tick_index = 0;
        world.time_ms = u64::MAX;
        assert_eq!(
            world.step(TickInput::new(100)).unwrap_err(),
            StepError::Overflow
        );
        assert_eq!(world.tick_index, 0);
        assert_eq!(world.time_ms, u64::MAX);
    }
}
