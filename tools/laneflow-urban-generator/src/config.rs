use serde::{Deserialize, Serialize};

use crate::{Error, Result};

pub const CELL_SIZE_METERS: u32 = 250;
pub const CELLS_PER_TILE: u32 = 10;
pub const SIGNAL_QUANTUM_MS: u64 = 528;
pub const MIN_PARKING_PER_TILE: u64 = 750;

/// Fixture is a two-tile test input; only 10k and 100k are delivery scales.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Scale {
    #[serde(rename = "fixture")]
    Fixture,
    #[serde(rename = "10k")]
    TenThousand,
    #[serde(rename = "100k")]
    HundredThousand,
}

impl Scale {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "fixture" => Ok(Self::Fixture),
            "10k" => Ok(Self::TenThousand),
            "100k" => Ok(Self::HundredThousand),
            _ => Err(Error::Config("scale must be fixture, 10k or 100k".into())),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Fixture => "fixture",
            Self::TenThousand => "10k",
            Self::HundredThousand => "100k",
        }
    }

    pub const fn tile_count(self) -> u32 {
        match self {
            Self::Fixture => 2,
            Self::TenThousand => 10,
            Self::HundredThousand => 100,
        }
    }

    pub const fn nominal_individual_count(self) -> u32 {
        self.tile_count() * 1_000
    }

    pub const fn fixed_step_ms(self) -> u64 {
        match self {
            Self::Fixture | Self::TenThousand => 16,
            Self::HundredThousand => 33,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UrbanConfig {
    pub config_version: u32,
    pub topology_template: String,
    pub template_slots: [crate::Template; 10],
    pub connection_rule: String,
    pub boundary_rule: String,
    pub arterial_speed_mps: f64,
    pub secondary_speed_mps: f64,
    pub turn_speed_mps: f64,
    pub cell_virtual_capacity: u32,
    pub garage_virtual_capacity: u32,
    pub parking_space_length_meters: f64,
    pub signals: SignalConfig,
    pub profiles: Vec<ProfileConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SignalConfig {
    pub through_quanta: u32,
    pub left_quanta: u32,
    pub yellow_quanta: u32,
    pub all_red_quanta: u32,
    pub offset_step_quanta: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfig {
    pub key: String,
    pub length_meters: f64,
    pub desired_speed_mps: f64,
    pub min_gap_meters: f64,
    pub time_headway_seconds: f64,
    pub max_acceleration_mps2: f64,
    pub comfortable_deceleration_mps2: f64,
    pub emergency_deceleration_mps2: f64,
}

impl UrbanConfig {
    pub fn parse(input: &str) -> Result<Self> {
        let value: Self = toml::from_str(input)?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<()> {
        if self.config_version != 1 {
            return Err(Error::Config("config_version must be 1".into()));
        }
        if self.topology_template != "connected-cn-urban-v1" {
            return Err(Error::Config(
                "topology_template must be connected-cn-urban-v1".into(),
            ));
        }
        if self.template_slots != crate::layout::TEMPLATE_ORDER
            || self.connection_rule != "reciprocal-cardinal-ports-v1"
            || self.boundary_rule != "unmatched-ports-open-v1"
        {
            return Err(Error::Config(
                "template slots and connection/boundary rules must match the fixed v1 topology"
                    .into(),
            ));
        }
        if self.profiles.is_empty() {
            return Err(Error::Config(
                "at least one physical profile is required".into(),
            ));
        }
        let mut keys = std::collections::BTreeSet::new();
        for profile in &self.profiles {
            if !keys.insert(&profile.key) {
                return Err(Error::Config(format!(
                    "duplicate profile key: {}",
                    profile.key
                )));
            }
            laneflow_compiler::road_editing::VehicleProfileInput::try_new(
                &profile.key,
                laneflow_compiler::road_editing::ParticipantClassReference::local("road-vehicle")?,
                profile.iidm()?,
            )?;
            if profile.length_meters > self.parking_space_length_meters {
                return Err(Error::Config(format!(
                    "parking spaces cannot accommodate profile {}",
                    profile.key
                )));
            }
        }
        for speed in [
            self.arterial_speed_mps,
            self.secondary_speed_mps,
            self.turn_speed_mps,
        ] {
            laneflow_compiler::road_editing::LaneEdgeInput::try_new(
                "configuration-speed",
                speed,
                Vec::new(),
                None,
            )?;
        }
        if !self.parking_space_length_meters.is_finite() || self.parking_space_length_meters <= 0.0
        {
            return Err(Error::Config(
                "parking space length must be finite and positive".into(),
            ));
        }
        // The virtual pools alone provide the floor, without counting an explicit bay twice.
        if self.cell_virtual_capacity == 0
            || self.garage_virtual_capacity == 0
            || self.garage_virtual_capacity == u32::MAX
            || u64::from(self.cell_virtual_capacity) * u64::from(CELLS_PER_TILE)
                + u64::from(self.garage_virtual_capacity)
                < MIN_PARKING_PER_TILE
        {
            return Err(Error::Config(
                "nonzero mixed/garage pools must provide at least 750 positions per tile; garage must allow the accepted +1 cutover".into(),
            ));
        }
        for quanta in [
            self.signals.through_quanta,
            self.signals.left_quanta,
            self.signals.yellow_quanta,
            self.signals.all_red_quanta,
        ] {
            if quanta == 0 {
                return Err(Error::Config(
                    "phase duration_quanta must be positive".into(),
                ));
            }
        }
        Ok(())
    }
}

impl ProfileConfig {
    pub(crate) fn iidm(&self) -> Result<laneflow_compiler::road_editing::IidmVehicleProfileInput> {
        Ok(
            laneflow_compiler::road_editing::IidmVehicleProfileInput::try_new(
                self.length_meters,
                self.desired_speed_mps,
                self.min_gap_meters,
                self.time_headway_seconds,
                self.max_acceleration_mps2,
                self.comfortable_deceleration_mps2,
                self.emergency_deceleration_mps2,
            )?,
        )
    }
}
