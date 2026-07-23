use std::collections::HashSet;
use std::path::{Component, Path};

use laneflow_core::MAX_PORTABLE_SIGNAL_TIME_MS;
use serde::Deserialize;

use crate::Error;

pub const CONFIG_VERSION: &str = "0.1";
pub const VEHICLE_LENGTH_METERS: f64 = 4.5;
pub const MIN_GAP_METERS: f64 = 2.0;
pub const ENDPOINT_CLEARANCE_METERS: f64 = VEHICLE_LENGTH_METERS + MIN_GAP_METERS;
pub const MIN_SPAWN_SLOT_COUNT: usize = 200;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorridorConfig {
    pub corridor_config_version: String,
    pub frame_id: String,
    pub fixed_delta_ms: u64,
    pub geometry: GeometryConfig,
    #[serde(rename = "speed_limits_kmh")]
    pub speed_limits: SpeedLimitConfig,
    pub signals: SignalConfig,
    pub output: OutputConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeometryConfig {
    pub main_length_meters: f64,
    pub secondary_lengths_meters: [f64; 2],
    pub intersection_x_meters: [f64; 2],
    pub lane_width_meters: f64,
    pub spawn_slot_pitch_meters: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeedLimitConfig {
    #[serde(rename = "main")]
    pub main_kilometers_per_hour: f64,
    #[serde(rename = "secondary")]
    pub secondary_kilometers_per_hour: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignalConfig {
    pub main_green_ms: u64,
    pub secondary_green_ms: u64,
    pub yellow_ms: u64,
    pub all_red_ms: u64,
    #[serde(rename = "intersection_offsets_ms")]
    pub controller_offsets_ms: [u64; 2],
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputConfig {
    pub directory: String,
    pub traffic_artifact_ref: String,
    pub spatial_artifact_ref: String,
    pub manifest_file_name: String,
    pub catalog_file_name: String,
}

impl CorridorConfig {
    pub fn parse(input: &str) -> Result<Self, Error> {
        let config: Self = toml::from_str(input)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), Error> {
        if self.corridor_config_version != CONFIG_VERSION {
            return Err(config_error(format!(
                "corridor_config_version must be {CONFIG_VERSION:?}"
            )));
        }
        if self.fixed_delta_ms == 0 {
            return Err(config_error("fixed_delta_ms must be greater than zero"));
        }
        laneflow_spatial::CanonicalFrameId::try_new(self.frame_id.clone())
            .map_err(|error| config_error(format!("frame_id is invalid: {error}")))?;

        for (name, value) in [
            (
                "geometry.main_length_meters",
                self.geometry.main_length_meters,
            ),
            (
                "geometry.secondary_lengths_meters[0]",
                self.geometry.secondary_lengths_meters[0],
            ),
            (
                "geometry.secondary_lengths_meters[1]",
                self.geometry.secondary_lengths_meters[1],
            ),
            (
                "geometry.lane_width_meters",
                self.geometry.lane_width_meters,
            ),
            (
                "geometry.spawn_slot_pitch_meters",
                self.geometry.spawn_slot_pitch_meters,
            ),
            (
                "speed_limits.main_kilometers_per_hour",
                self.speed_limits.main_kilometers_per_hour,
            ),
            (
                "speed_limits.secondary_kilometers_per_hour",
                self.speed_limits.secondary_kilometers_per_hour,
            ),
        ] {
            if !value.is_finite() || value <= 0.0 {
                return Err(config_error(format!("{name} must be finite and positive")));
            }
        }
        for (index, value) in self.geometry.intersection_x_meters.iter().enumerate() {
            if !value.is_finite() {
                return Err(config_error(format!(
                    "geometry.intersection_x_meters[{index}] must be finite"
                )));
            }
        }

        let total_length = self.geometry.main_length_meters
            + self.geometry.secondary_lengths_meters.iter().sum::<f64>();
        if total_length > 2_000.0 {
            return Err(config_error(format!(
                "physical road length is {total_length} m; maximum is 2000 m"
            )));
        }
        if self.geometry.intersection_x_meters[0] >= self.geometry.intersection_x_meters[1] {
            return Err(config_error(
                "intersection_x_meters must be strictly increasing",
            ));
        }

        let main_half = self.geometry.main_length_meters / 2.0;
        let secondary_half_width = self.geometry.lane_width_meters * 2.0;
        let main_half_width = self.geometry.lane_width_meters * 3.0;
        let [left, right] = self.geometry.intersection_x_meters;
        if left - secondary_half_width <= -main_half || right + secondary_half_width >= main_half {
            return Err(config_error(
                "each intersection must leave a positive main-road portal segment",
            ));
        }
        if left + secondary_half_width >= right - secondary_half_width {
            return Err(config_error(
                "intersection connector envelopes overlap or leave no middle segment",
            ));
        }
        for (index, length) in self.geometry.secondary_lengths_meters.iter().enumerate() {
            if length / 2.0 <= main_half_width {
                return Err(config_error(format!(
                    "secondary road {index} must extend beyond the main-road envelope"
                )));
            }
        }
        if self.geometry.spawn_slot_pitch_meters < ENDPOINT_CLEARANCE_METERS {
            return Err(config_error(format!(
                "spawn_slot_pitch_meters must be at least {ENDPOINT_CLEARANCE_METERS} m"
            )));
        }

        let cycle = self.signal_cycle_ms()?;
        for (name, duration) in [
            ("signals.main_green_ms", self.signals.main_green_ms),
            (
                "signals.secondary_green_ms",
                self.signals.secondary_green_ms,
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
        }
        for (index, offset) in self.signals.controller_offsets_ms.iter().enumerate() {
            if *offset >= cycle {
                return Err(config_error(format!(
                    "signals.controller_offsets_ms[{index}] must satisfy 0 <= offset < cycle ({cycle})"
                )));
            }
        }

        if self.output.directory.trim().is_empty() {
            return Err(config_error("output.directory must not be empty"));
        }
        let names = [
            &self.output.traffic_artifact_ref,
            &self.output.spatial_artifact_ref,
            &self.output.manifest_file_name,
            &self.output.catalog_file_name,
        ];
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

    pub fn signal_cycle_ms(&self) -> Result<u64, Error> {
        self.signals
            .main_green_ms
            .checked_add(self.signals.secondary_green_ms)
            .and_then(|value| value.checked_add(self.signals.yellow_ms.checked_mul(2)?))
            .and_then(|value| value.checked_add(self.signals.all_red_ms.checked_mul(2)?))
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
