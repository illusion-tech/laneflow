//! SUMO DUE `<vehicle>` records used for population selection.

use crate::sumo::decimal::ExactDecimal;

/// One `<vehicle>` with an inline `<route edges="...">`.
#[derive(Clone, Debug)]
pub struct DueVehicle {
    pub id: String,
    pub type_id: String,
    pub depart: ExactDecimal,
    pub road_edge_ids: Vec<String>,
    /// 原始 `departPos` 属性（#253 U5：缺失/非 `random` 在入选集内
    /// fail-closed；解析期原样保留，不做判定）。
    pub depart_pos: Option<String>,
    /// Which DUE file this came from: `0`, `1`, or `2`.
    pub source_file_ordinal: u8,
    /// 0-based ordinal of this `<vehicle>` within its source file.
    pub source_vehicle_ordinal: u64,
}
