use std::collections::HashSet;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use crate::Error;

pub const CONFIG_VERSION: &str = "0.1";
const MAX_PORTABLE_SIGNAL_TIME_MS: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JunctionConfig {
    pub junction_config_version: String,
    pub frame_id: String,
    pub fixed_delta_ms: u64,
    pub geometry: GeometryConfig,
    pub speeds: SpeedConfig,
    pub signals: SignalConfig,
    pub profile: ProfileConfig,
    pub output: OutputConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeometryConfig {
    pub arm_length_meters: f64,
    pub junction_radius_meters: f64,
    pub lane_width_meters: f64,
    pub center_offset_meters: f64,
    pub pocket_length_meters: f64,
    pub pocket_offset_meters: f64,
    pub curve_control_meters: f64,
    pub loop_corner_radius_meters: f64,
    pub loop_outer_widen_meters: f64,
    pub spawn_slot_pitch_meters: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpeedConfig {
    pub main_meters_per_second: f64,
    pub secondary_meters_per_second: f64,
    pub turn_meters_per_second: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SignalConfig {
    pub main_left_green_ms: u64,
    pub main_through_green_ms: u64,
    pub secondary_through_green_ms: u64,
    pub yellow_ms: u64,
    pub all_red_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfig {
    pub length_meters: f64,
    pub desired_speed_meters_per_second: f64,
    pub min_gap_meters: f64,
    pub time_headway_seconds: f64,
    pub max_acceleration_meters_per_second_squared: f64,
    pub comfortable_deceleration_meters_per_second_squared: f64,
    pub emergency_deceleration_meters_per_second_squared: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OutputConfig {
    pub directory: String,
    pub catalog_file_name: String,
    pub lfca_file_name: String,
}

impl JunctionConfig {
    pub fn parse(input: &str) -> Result<Self, Error> {
        let config: Self = toml::from_str(input)?;
        config.validate()?;
        Ok(config)
    }

    /// 车辆前保险杠到 edge 端点必须保留的净距。
    pub fn endpoint_clearance_meters(&self) -> f64 {
        self.profile.length_meters + self.profile.min_gap_meters
    }

    pub fn validate(&self) -> Result<(), Error> {
        if self.junction_config_version != CONFIG_VERSION {
            return Err(config_error(format!(
                "junction_config_version must be {CONFIG_VERSION:?}"
            )));
        }
        // 与 TrafficWorld::install 的 DeltaOutOfRange 对齐，生成前拒绝不可安装的 tick。
        if !(4..=1_000).contains(&self.fixed_delta_ms) {
            return Err(config_error(
                "fixed_delta_ms must be within the runtime-supported 4..=1000 range",
            ));
        }
        laneflow_spatial::CanonicalFrameId::try_new(self.frame_id.clone())
            .map_err(|error| config_error(format!("frame_id is invalid: {error}")))?;

        let geometry = &self.geometry;
        for (name, value) in [
            ("geometry.arm_length_meters", geometry.arm_length_meters),
            (
                "geometry.junction_radius_meters",
                geometry.junction_radius_meters,
            ),
            ("geometry.lane_width_meters", geometry.lane_width_meters),
            (
                "geometry.center_offset_meters",
                geometry.center_offset_meters,
            ),
            (
                "geometry.pocket_length_meters",
                geometry.pocket_length_meters,
            ),
            (
                "geometry.pocket_offset_meters",
                geometry.pocket_offset_meters,
            ),
            (
                "geometry.curve_control_meters",
                geometry.curve_control_meters,
            ),
            (
                "geometry.loop_corner_radius_meters",
                geometry.loop_corner_radius_meters,
            ),
            (
                "geometry.loop_outer_widen_meters",
                geometry.loop_outer_widen_meters,
            ),
            (
                "geometry.spawn_slot_pitch_meters",
                geometry.spawn_slot_pitch_meters,
            ),
            (
                "speeds.main_meters_per_second",
                self.speeds.main_meters_per_second,
            ),
            (
                "speeds.secondary_meters_per_second",
                self.speeds.secondary_meters_per_second,
            ),
            (
                "speeds.turn_meters_per_second",
                self.speeds.turn_meters_per_second,
            ),
            ("profile.length_meters", self.profile.length_meters),
            (
                "profile.desired_speed_meters_per_second",
                self.profile.desired_speed_meters_per_second,
            ),
            ("profile.min_gap_meters", self.profile.min_gap_meters),
            (
                "profile.time_headway_seconds",
                self.profile.time_headway_seconds,
            ),
            (
                "profile.max_acceleration_meters_per_second_squared",
                self.profile.max_acceleration_meters_per_second_squared,
            ),
            (
                "profile.comfortable_deceleration_meters_per_second_squared",
                self.profile
                    .comfortable_deceleration_meters_per_second_squared,
            ),
            (
                "profile.emergency_deceleration_meters_per_second_squared",
                self.profile
                    .emergency_deceleration_meters_per_second_squared,
            ),
        ] {
            if !value.is_finite() || value <= 0.0 {
                return Err(config_error(format!("{name} must be finite and positive")));
            }
        }
        if geometry.center_offset_meters <= geometry.lane_width_meters {
            return Err(config_error(
                "center_offset_meters must exceed lane_width_meters so the median gap stays open",
            ));
        }
        if geometry.junction_radius_meters
            <= geometry.pocket_length_meters + geometry.curve_control_meters
        {
            return Err(config_error(
                "junction_radius_meters must exceed pocket_length_meters + curve_control_meters",
            ));
        }
        // 待转 pocket 横向净距：入口 lane0 中心线偏移 a = center − width/2，
        // pocket 中心线 z = a − pocket_offset；车辆半宽不进入对向 lane0
        // （中心线 −a、半宽 w/2）要求 pocket_offset < 2·(center − width)。
        let pocket_offset_limit =
            2.0 * (geometry.center_offset_meters - geometry.lane_width_meters);
        if geometry.pocket_offset_meters >= pocket_offset_limit {
            return Err(config_error(format!(
                "pocket_offset_meters must be below {pocket_offset_limit} m so the waiting \
                 pocket stays clear of the opposing through lanes",
            )));
        }
        // 下限：pocket 也不得退回本向 lane0 的车道带（中心线 a、半宽 w/2），
        // 否则 w-n.i1 与 W→E lane0 在同相位门下物理重叠。
        if geometry.pocket_offset_meters < geometry.lane_width_meters {
            return Err(config_error(
                "pocket_offset_meters must be at least lane_width_meters so the waiting pocket \
                 stays clear of the approach through lane",
            ));
        }
        // 待转区容量 1 的存储长度即 pocket 长度；车长超限会让 route-w-left-waiting
        // 的每次 spawn 都以 VehicleTooLong 失败。
        if self.profile.length_meters > geometry.pocket_length_meters {
            return Err(config_error(
                "profile.length_meters must not exceed pocket_length_meters so the waiting \
                 route remains spawnable",
            ));
        }
        if geometry.arm_length_meters <= geometry.junction_radius_meters * 2.0 {
            return Err(config_error(
                "arm_length_meters must exceed twice junction_radius_meters to leave a loop corner",
            ));
        }
        // 环路三段圆弧的直线腿长度 = arm_length − 车道偏移 − corner_radius（外侧环路
        // 首段圆弧半径再加大一个车道宽，且远端外扩 widen）；直线腿必须为正且不小于
        // 转角半径。
        let shortest_leg = geometry.arm_length_meters
            - (geometry.center_offset_meters + geometry.lane_width_meters / 2.0)
            - (geometry.loop_corner_radius_meters + geometry.lane_width_meters);
        if shortest_leg < geometry.loop_corner_radius_meters {
            return Err(config_error(
                "arm_length_meters too short for loop_corner_radius_meters: loop straight legs \
                 must be at least the corner radius",
            ));
        }

        if geometry.spawn_slot_pitch_meters < self.endpoint_clearance_meters() {
            return Err(config_error(format!(
                "spawn_slot_pitch_meters must be at least {} m",
                self.endpoint_clearance_meters()
            )));
        }
        // 同 portal 配对环路的槽位交错 pitch/2；交错量必须不小于车长，
        // 否则两条边重合段上的车辆物理重叠。pitch/2 ≥ length ⟺ pitch ≥ 2·length。
        if geometry.spawn_slot_pitch_meters < self.profile.length_meters * 2.0 {
            return Err(config_error(
                "spawn_slot_pitch_meters must be at least twice profile.length_meters so the \
                 half-pitch stagger keeps paired loop slots one vehicle length apart",
            ));
        }

        self.signal_cycle_ms()?;
        for (name, duration) in [
            (
                "signals.main_left_green_ms",
                self.signals.main_left_green_ms,
            ),
            (
                "signals.main_through_green_ms",
                self.signals.main_through_green_ms,
            ),
            (
                "signals.secondary_through_green_ms",
                self.signals.secondary_through_green_ms,
            ),
            ("signals.yellow_ms", self.signals.yellow_ms),
            ("signals.all_red_ms", self.signals.all_red_ms),
        ] {
            if duration < self.fixed_delta_ms {
                return Err(config_error(format!(
                    "{name} must be at least fixed_delta_ms ({})",
                    self.fixed_delta_ms
                )));
            }
            // TrafficWorld::install 要求每个相位时长是 fixed_delta 的整数倍；
            // 在 config 校验期就拒绝，避免生成无法安装的 LFCA。
            if duration % self.fixed_delta_ms != 0 {
                return Err(config_error(format!(
                    "{name} must be a whole multiple of fixed_delta_ms ({})",
                    self.fixed_delta_ms
                )));
            }
        }

        if self.output.directory.trim().is_empty() {
            return Err(config_error("output.directory must not be empty"));
        }
        let names = [&self.output.catalog_file_name, &self.output.lfca_file_name];
        let mut unique = HashSet::new();
        for name in names {
            if !is_single_file_name(name) {
                return Err(config_error(format!(
                    "output file name {name:?} must be one non-empty path component"
                )));
            }
            if !unique.insert(name) {
                return Err(config_error(format!(
                    "output file names must be distinct; duplicate {name:?}"
                )));
            }
        }
        Ok(())
    }

    /// 固定时制信号周期：三组 active set（主路左转、主路直行+许可左转、次路直行）
    /// 各自 green+yellow 加三次 all-red。
    pub fn signal_cycle_ms(&self) -> Result<u64, Error> {
        let green_sum = self
            .signals
            .main_left_green_ms
            .checked_add(self.signals.main_through_green_ms)
            .and_then(|value| value.checked_add(self.signals.secondary_through_green_ms));
        green_sum
            .and_then(|value| value.checked_add(self.signals.yellow_ms.checked_mul(3)?))
            .and_then(|value| value.checked_add(self.signals.all_red_ms.checked_mul(3)?))
            .filter(|value| *value <= MAX_PORTABLE_SIGNAL_TIME_MS)
            .ok_or_else(|| config_error("signal cycle overflows the portable signal time range"))
    }
}

fn is_single_file_name(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && path.components().count() == 1
        && matches!(path.components().next(), Some(Component::Normal(_)))
}

fn config_error(message: impl Into<String>) -> Error {
    Error::Config(message.into())
}
