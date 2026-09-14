//! Verification hosts share one demand scheduler and one committed-state oracle.
//! This module adds no mutable-world escape hatch to the production Adapter.

use std::{ops::Deref, time::Instant};

use laneflow_runtime::*;

use crate::{Result, checked};

pub(crate) enum Host {
    Headless(Box<TrafficWorld>),
    #[cfg(feature = "adapter")]
    Adapter(Box<bevy_app::App>),
}

impl Deref for Host {
    type Target = TrafficWorld;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Headless(world) => world,
            #[cfg(feature = "adapter")]
            Self::Adapter(app) => app
                .world()
                .resource::<laneflow_bevy::LaneFlowSession>()
                .world(),
        }
    }
}

impl Host {
    pub(crate) fn spawn_vehicle(
        &mut self,
        input: VehicleSpawnInput,
    ) -> std::result::Result<VehicleHandle, SpawnError> {
        match self {
            Self::Headless(world) => world.spawn_vehicle(input),
            #[cfg(feature = "adapter")]
            Self::Adapter(app) => app
                .world_mut()
                .resource_mut::<laneflow_bevy::LaneFlowSession>()
                .world_mut()
                .spawn_vehicle(input),
        }
    }

    // The outer Result preserves Adapter infrastructure errors separately from
    // expected Runtime rejections, which the existing demand scheduler classifies.
    pub(crate) fn replace_completed_vehicle(
        &mut self,
        old: VehicleHandle,
        input: VehicleSpawnInput,
    ) -> Result<std::result::Result<VehicleReplaceRecord, ReplaceError>> {
        match self {
            Self::Headless(world) => Ok(world.replace_completed_vehicle(old, input)),
            #[cfg(feature = "adapter")]
            Self::Adapter(app) => {
                use laneflow_bevy::{LaneFlowAdapterError, LaneFlowVehicleReplaceOutcome};
                match laneflow_bevy::replace_completed_vehicle(app.world_mut(), old, input) {
                    Ok(LaneFlowVehicleReplaceOutcome::Replaced(record)) => {
                        Ok(Ok(VehicleReplaceRecord {
                            old: record.old,
                            new: record.new,
                        }))
                    }
                    Ok(LaneFlowVehicleReplaceOutcome::Blocked(block)) => {
                        Ok(Err(ReplaceError::Blocked(block)))
                    }
                    Err(LaneFlowAdapterError::VehicleReplace { source, .. }) => Ok(Err(source)),
                    Err(error) => Err(crate::invalid(format!("Adapter replacement: {error}"))),
                    Ok(_) => Err(crate::invalid("unknown Adapter replacement outcome")),
                }
            }
        }
    }

    pub(crate) fn despawn_vehicle(
        &mut self,
        vehicle: VehicleHandle,
    ) -> Result<std::result::Result<VehicleDespawnRecord, ParkingError>> {
        match self {
            Self::Headless(world) => Ok(world.despawn_vehicle(vehicle)),
            #[cfg(feature = "adapter")]
            Self::Adapter(app) => match laneflow_bevy::despawn_vehicle(app.world_mut(), vehicle) {
                Ok(record) => {
                    if let Some(entity) = record.entity {
                        app.world_mut().despawn(entity);
                    }
                    Ok(Ok(record.runtime))
                }
                Err(laneflow_bevy::LaneFlowAdapterError::VehicleDespawn { source, .. }) => {
                    Ok(Err(source))
                }
                Err(error) => Err(crate::invalid(format!("Adapter despawn: {error}"))),
            },
        }
    }

    pub(crate) fn reserve_parking(
        &mut self,
        vehicle: VehicleHandle,
        target: ReserveParkingTarget,
    ) -> std::result::Result<ParkingCommandOutcome<ParkingReserveRecord>, ParkingError> {
        match self {
            Self::Headless(world) => world.reserve_parking(vehicle, target),
            #[cfg(feature = "adapter")]
            Self::Adapter(app) => app
                .world_mut()
                .resource_mut::<laneflow_bevy::LaneFlowSession>()
                .world_mut()
                .reserve_parking(vehicle, target),
        }
    }

    pub(crate) fn park_vehicle(
        &mut self,
        vehicle: VehicleHandle,
        target: ParkingTarget,
    ) -> std::result::Result<ParkingCommandOutcome<ParkingParkRecord>, ParkingError> {
        match self {
            Self::Headless(world) => world.park_vehicle(vehicle, target),
            #[cfg(feature = "adapter")]
            Self::Adapter(app) => app
                .world_mut()
                .resource_mut::<laneflow_bevy::LaneFlowSession>()
                .world_mut()
                .park_vehicle(vehicle, target),
        }
    }

    pub(crate) fn leave_parking(
        &mut self,
        vehicle: VehicleHandle,
        target: LeaveParkingTarget,
    ) -> std::result::Result<ParkingLeaveRecord, ParkingError> {
        match self {
            Self::Headless(world) => world.leave_parking(vehicle, target),
            #[cfg(feature = "adapter")]
            Self::Adapter(app) => app
                .world_mut()
                .resource_mut::<laneflow_bevy::LaneFlowSession>()
                .world_mut()
                .leave_parking(vehicle, target),
        }
    }

    pub(crate) fn step(&mut self, input: TickInput) -> Result<(StepOutcome, u64)> {
        match self {
            Self::Headless(world) => {
                let started = Instant::now();
                let result = world.step(input);
                let elapsed = nanos(started.elapsed());
                Ok((checked("TrafficWorld step", result)?, elapsed))
            }
            #[cfg(feature = "adapter")]
            Self::Adapter(app) => adapter::step(app, input),
        }
    }
}

fn nanos(duration: std::time::Duration) -> u64 {
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}

#[cfg(feature = "adapter")]
mod adapter {
    use std::{num::NonZeroU32, time::Duration};

    use bevy_app::App;
    use bevy_ecs::{
        prelude::{ResMut, Resource, World},
        schedule::IntoScheduleConfigs,
    };
    use bevy_time::Time;
    use laneflow_bevy::{
        LaneFlowFixed, LaneFlowFixedSet, LaneFlowPlugin, LaneFlowSession, LaneFlowSessionConfig,
    };
    use laneflow_spatial::SpatialSession;

    use super::*;
    use crate::invalid;

    #[derive(Resource, Default)]
    struct StepClock {
        started: Option<Instant>,
        elapsed: u64,
        samples: u32,
    }

    fn before_step(mut clock: ResMut<'_, StepClock>) {
        clock.started = Some(Instant::now());
    }

    fn after_step(mut clock: ResMut<'_, StepClock>) {
        if let Some(started) = clock.started.take() {
            clock.elapsed = nanos(started.elapsed());
            clock.samples += 1;
        }
    }

    impl Host {
        pub(crate) fn into_adapter(self, spatial: SpatialSession) -> Result<Self> {
            let Self::Headless(world) = self else {
                return Err(invalid("harness already uses Adapter"));
            };
            let session = checked(
                "Adapter installation",
                LaneFlowSession::new(
                    *world,
                    Some(spatial),
                    LaneFlowSessionConfig::new(NonZeroU32::new(1).unwrap()),
                ),
            )?;
            let mut app = App::new();
            app.insert_resource(Time::<()>::default())
                .insert_resource(session)
                .init_resource::<StepClock>()
                .add_plugins(LaneFlowPlugin)
                .add_systems(
                    LaneFlowFixed,
                    (
                        before_step
                            .after(LaneFlowFixedSet::Lifecycle)
                            .before(LaneFlowFixedSet::Step),
                        after_step
                            .after(LaneFlowFixedSet::Step)
                            .before(LaneFlowFixedSet::Observe),
                    ),
                );
            Ok(Self::Adapter(Box::new(app)))
        }

        pub(crate) fn adapter_world(&mut self) -> Result<&mut World> {
            match self {
                Self::Adapter(app) => Ok(app.world_mut()),
                Self::Headless(_) => Err(invalid("harness is headless")),
            }
        }
    }

    pub(super) fn step(app: &mut App, input: TickInput) -> Result<(StepOutcome, u64)> {
        let before = app
            .world()
            .resource::<LaneFlowSession>()
            .world()
            .tick_index();
        app.world_mut().resource_mut::<StepClock>().samples = 0;
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_millis(input.delta_time_ms));
        app.update();
        let session = app.world().resource::<LaneFlowSession>();
        if let Some(error) = session.last_error() {
            return Err(invalid(format!("Adapter step: {error}")));
        }
        let clock = app.world().resource::<StepClock>();
        if session.world().tick_index() != before + 1
            || session.frame_report().steps_run() != 1
            || session.frame_report().backlog() != Duration::ZERO
            || clock.samples != 1
        {
            return Err(invalid(
                "Adapter must commit exactly one fixed step without backlog",
            ));
        }
        let outcome = session
            .frame_step_results()
            .first()
            .ok_or_else(|| invalid("missing Adapter step result"))?
            .clone();
        Ok((outcome, clock.elapsed))
    }
}
