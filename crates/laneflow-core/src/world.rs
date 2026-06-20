//! Core world 与 fixed-step orchestration。

use crate::{
    error::CoreError,
    time::{StepResult, TickInput},
    vehicle::VehicleState,
};

/// LaneFlow Core 的最小 runtime state。
#[derive(Clone, Debug, PartialEq)]
pub struct CoreWorld {
    fixed_delta_time_ms: u64,
    tick_index: u64,
    time_ms: u64,
    vehicles: Vec<VehicleState>,
}

impl CoreWorld {
    /// 创建不含车辆的 Core world。
    pub fn new(fixed_delta_time_ms: u64) -> Result<Self, CoreError> {
        Self::with_vehicles(fixed_delta_time_ms, Vec::new())
    }

    /// 创建包含初始车辆的 Core world。
    pub fn with_vehicles(
        fixed_delta_time_ms: u64,
        vehicles: Vec<VehicleState>,
    ) -> Result<Self, CoreError> {
        if fixed_delta_time_ms == 0 {
            return Err(CoreError::InvalidFixedDeltaTime {
                fixed_delta_time_ms,
            });
        }

        for vehicle in &vehicles {
            vehicle.validate()?;
        }

        Ok(Self {
            fixed_delta_time_ms,
            tick_index: 0,
            time_ms: 0,
            vehicles,
        })
    }

    /// 返回当前 world 的固定 tick 步长。
    pub const fn fixed_delta_time_ms(&self) -> u64 {
        self.fixed_delta_time_ms
    }

    /// 返回当前 tick index。
    pub const fn tick_index(&self) -> u64 {
        self.tick_index
    }

    /// 返回当前累计 simulation time。
    pub const fn time_ms(&self) -> u64 {
        self.time_ms
    }

    /// 返回当前车辆状态。
    pub fn vehicles(&self) -> &[VehicleState] {
        &self.vehicles
    }

    /// 推进一个 fixed-step tick。
    ///
    /// 成功时，`StepResult` 使用 post-step tick/time；失败时 world 保持不变。
    pub fn step(&mut self, input: TickInput) -> Result<StepResult, CoreError> {
        if input.delta_time_ms != self.fixed_delta_time_ms {
            return Err(CoreError::TickDeltaMismatch {
                expected_delta_time_ms: self.fixed_delta_time_ms,
                actual_delta_time_ms: input.delta_time_ms,
            });
        }

        for vehicle in &self.vehicles {
            vehicle.validate()?;
        }

        let next_tick_index = self
            .tick_index
            .checked_add(1)
            .ok_or(CoreError::TimeOverflow)?;
        let next_time_ms = self
            .time_ms
            .checked_add(self.fixed_delta_time_ms)
            .ok_or(CoreError::TimeOverflow)?;

        self.tick_index = next_tick_index;
        self.time_ms = next_time_ms;

        Ok(StepResult {
            tick_index: self.tick_index,
            time_ms: self.time_ms,
            events: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CoreError, TickInput, VehicleState};

    #[test]
    fn unit_step_advances_post_step_time() {
        let mut world = CoreWorld::new(20).expect("valid world");

        let result = world.step(TickInput::new(20)).expect("step succeeds");

        assert_eq!(world.tick_index(), 1);
        assert_eq!(world.time_ms(), 20);
        assert_eq!(result.tick_index, 1);
        assert_eq!(result.time_ms, 20);
    }

    #[test]
    fn unit_delta_mismatch_keeps_world_unchanged() {
        let vehicle = VehicleState::active("V1", "R1", 0, 1.0, 0.0);
        let mut world = CoreWorld::with_vehicles(20, vec![vehicle]).expect("valid world");
        let before = world.clone();

        let error = world
            .step(TickInput::new(16))
            .expect_err("delta mismatch must fail");

        assert_eq!(
            error,
            CoreError::TickDeltaMismatch {
                expected_delta_time_ms: 20,
                actual_delta_time_ms: 16
            }
        );
        assert_eq!(world, before);
    }
}
