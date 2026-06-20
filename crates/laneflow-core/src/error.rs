//! Core runtime 的错误类型。

/// Core runtime 暴露给调用方的错误。
#[derive(Clone, Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CoreError {
    /// `CoreWorld` 的固定步长必须大于 0。
    #[error("`fixed_delta_time_ms` 必须大于 0，实际值为 {fixed_delta_time_ms}")]
    InvalidFixedDeltaTime { fixed_delta_time_ms: u64 },
    /// tick 输入的 delta 必须等于当前 world 的固定步长。
    #[error("tick delta 不匹配：期望 {expected_delta_time_ms} ms，实际 {actual_delta_time_ms} ms")]
    TickDeltaMismatch {
        expected_delta_time_ms: u64,
        actual_delta_time_ms: u64,
    },
    /// tick/time 累计发生整数溢出。
    #[error("tick/time 累计发生整数溢出")]
    TimeOverflow,
    /// speed 必须是 finite 且大于或等于 0。
    #[error("speed 无效：{speed}")]
    InvalidSpeed { speed: f64 },
    /// edge progress 必须是 finite 且大于或等于 0。
    #[error("edge progress 无效：{edge_progress}")]
    InvalidEdgeProgress { edge_progress: f64 },
    /// lane edge length 必须是 finite 且大于 epsilon。
    #[error("lane edge length 无效：{edge_length}，必须是 finite 且大于 {min_exclusive}")]
    InvalidLaneEdgeLength {
        edge_length: f64,
        min_exclusive: f64,
    },
    /// lane edge id 在 graph 内必须唯一。
    #[error("lane edge id 重复：{edge_id}")]
    DuplicateLaneEdgeId { edge_id: String },
    /// lane edge 的 next edge 引用必须存在。
    #[error("lane edge `{edge_id}` 引用了不存在的 next edge：{next_edge_id}")]
    UnknownNextLaneEdge {
        edge_id: String,
        next_edge_id: String,
    },
    /// route id 在 world 内必须唯一。
    #[error("route id 重复：{route_id}")]
    DuplicateRouteId { route_id: String },
    /// route 至少需要一个 edge。
    #[error("route `{route_id}` 不能为空")]
    EmptyRoute { route_id: String },
    /// route 引用的 edge 必须存在。
    #[error("route `{route_id}` 引用了不存在的 lane edge：{edge_id}")]
    UnknownRouteEdge { route_id: String, edge_id: String },
    /// route 相邻 edge 必须连通。
    #[error("route `{route_id}` 中 edge `{from_edge_id}` 不能连接到 `{to_edge_id}`")]
    DisconnectedRouteEdge {
        route_id: String,
        from_edge_id: String,
        to_edge_id: String,
    },
    /// vehicle id 在 world 内必须唯一。
    #[error("vehicle id 重复：{vehicle_id}")]
    DuplicateVehicleId { vehicle_id: String },
    /// vehicle 引用的 route 必须存在。
    #[error("vehicle `{vehicle_id}` 引用了不存在的 route：{route_id}")]
    UnknownVehicleRoute {
        vehicle_id: String,
        route_id: String,
    },
    /// vehicle route edge index 必须落在 route edge sequence 范围内。
    #[error(
        "vehicle `{vehicle_id}` 的 route edge index 无效：route `{route_id}` 长度为 {route_edge_count}，实际 index 为 {route_edge_index}"
    )]
    InvalidVehicleRouteEdgeIndex {
        vehicle_id: String,
        route_id: String,
        route_edge_index: usize,
        route_edge_count: usize,
    },
    /// vehicle edge progress 必须小于或等于当前 edge length。
    #[error(
        "vehicle `{vehicle_id}` 在 edge `{edge_id}` 上的 progress 超出范围：progress={edge_progress}，edge length={edge_length}"
    )]
    VehicleEdgeProgressOutOfRange {
        vehicle_id: String,
        edge_id: String,
        edge_progress: f64,
        edge_length: f64,
    },
    /// completed vehicle 的初始位置必须位于 route 终点。
    #[error(
        "completed vehicle `{vehicle_id}` 的初始状态无效：route `{route_id}` 期望最后 edge index={expected_route_edge_index} 且 progress 在终点 epsilon 内，实际 index={route_edge_index}, progress={edge_progress}, edge length={edge_length}"
    )]
    InvalidCompletedVehicleState {
        vehicle_id: String,
        route_id: String,
        route_edge_index: usize,
        expected_route_edge_index: usize,
        edge_progress: f64,
        edge_length: f64,
    },
    /// route following 计算出的 travel distance 必须保持 finite。
    #[error(
        "vehicle `{vehicle_id}` 的 route travel distance 不是 finite：speed={speed}, delta={delta_time_ms} ms"
    )]
    NonFiniteRouteTravel {
        vehicle_id: String,
        speed: f64,
        delta_time_ms: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_messages_use_chinese_runtime_text() {
        assert_eq!(
            CoreError::InvalidFixedDeltaTime {
                fixed_delta_time_ms: 0
            }
            .to_string(),
            "`fixed_delta_time_ms` 必须大于 0，实际值为 0"
        );
        assert_eq!(
            CoreError::TickDeltaMismatch {
                expected_delta_time_ms: 20,
                actual_delta_time_ms: 16
            }
            .to_string(),
            "tick delta 不匹配：期望 20 ms，实际 16 ms"
        );
        assert_eq!(
            CoreError::TimeOverflow.to_string(),
            "tick/time 累计发生整数溢出"
        );
        assert_eq!(
            CoreError::InvalidSpeed { speed: -1.0 }.to_string(),
            "speed 无效：-1"
        );
        assert_eq!(
            CoreError::InvalidEdgeProgress {
                edge_progress: f64::NAN
            }
            .to_string(),
            "edge progress 无效：NaN"
        );
        assert_eq!(
            CoreError::InvalidCompletedVehicleState {
                vehicle_id: "V1".to_owned(),
                route_id: "R1".to_owned(),
                route_edge_index: 0,
                expected_route_edge_index: 1,
                edge_progress: 1.0,
                edge_length: 5.0,
            }
            .to_string(),
            "completed vehicle `V1` 的初始状态无效：route `R1` 期望最后 edge index=1 且 progress 在终点 epsilon 内，实际 index=0, progress=1, edge length=5"
        );
        assert_eq!(
            CoreError::NonFiniteRouteTravel {
                vehicle_id: "V1".to_owned(),
                speed: f64::MAX,
                delta_time_ms: 1000,
            }
            .to_string(),
            format!(
                "vehicle `V1` 的 route travel distance 不是 finite：speed={}, delta=1000 ms",
                f64::MAX
            )
        );
    }
}
