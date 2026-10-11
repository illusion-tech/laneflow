//! SUMO `vType` / `vTypeDistribution` model for profile conversion.

use crate::sumo::decimal::ExactDecimal;

/// One SUMO `<vType>` entry.
#[derive(Clone, Debug)]
pub struct SumoVType {
    pub id: String,
    pub v_class: String,
    pub accel: ExactDecimal,
    pub decel: ExactDecimal,
    pub length: ExactDecimal,
    pub min_gap: ExactDecimal,
    pub max_speed: ExactDecimal,
}
