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
        CoreError::SignalsVehicleCapabilityUnavailable.to_string(),
        "旧版 v0.4 Signals 车辆能力防护错误：#96 完整合规后不应再触发；若再次出现，请检查 SignalStop、hard projection 与 permission-aware traversal 是否完整接入"
    );
    assert_eq!(
        CoreError::InvalidSpeed { speed: -1.0 }.to_string(),
        "speed 无效：-1"
    );
    assert_eq!(
        CoreError::InvalidAcceleration { acceleration: -2.5 }.to_string(),
        "acceleration 无效：-2.5"
    );
    assert_eq!(
        CoreError::InvalidEdgeProgress {
            edge_progress: f64::NAN
        }
        .to_string(),
        "edge progress 无效：NaN"
    );
    assert_eq!(
        CoreError::InvalidExternalId {
            field: "laneGraph.edges[].id",
            external_id: "edge 1".to_owned(),
            pattern: "^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$",
        }
        .to_string(),
        "external ID 无效：field=laneGraph.edges[].id, value=`edge 1`，必须匹配 ^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$"
    );
    assert_eq!(
        CoreError::DuplicateLaneEdgeConnection {
            edge_id: "A".to_owned(),
            next_edge_id: "B".to_owned(),
        }
        .to_string(),
        "lane edge `A` 重复声明 connection target：B"
    );
    assert_eq!(
        CoreError::UnknownVehicleProfileHandle {
            vehicle_id: "V1".to_owned(),
            profile: VehicleProfileHandle::new(1),
        }
        .to_string(),
        format!(
            "vehicle `V1` 引用了未知的 Vehicle Profile handle：{:?}",
            VehicleProfileHandle::new(1)
        )
    );
    assert_eq!(
        CoreError::InvalidInactiveVehicleMotion {
            vehicle_id: "V1".to_owned(),
            status: VehicleStatus::Stopped,
            initial_speed: 1.0,
        }
        .to_string(),
        "inactive vehicle `V1` 的初始速度必须为 0：status=Stopped, initial_speed=1"
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
        CoreError::VehiclePhysicalOverlap {
            follower_id: "V1".to_owned(),
            leader_id: "V2".to_owned(),
            bumper_gap: -0.5,
        }
        .to_string(),
        "vehicle `V1` 与 leader `V2` 发生物理重叠：bumper_gap=-0.5"
    );
    assert_eq!(
        CoreError::NonFiniteLeaderComputation {
            vehicle: VehicleHandle::new(0, 0),
            stage: "hard_horizon",
            value: f64::INFINITY,
        }
        .to_string(),
        format!(
            "vehicle `{:?}` 的 leader detection 计算不是 finite：stage=hard_horizon, value=inf",
            VehicleHandle::new(0, 0)
        )
    );
    assert_eq!(
        CoreError::NonFiniteLongitudinalComputation {
            vehicle: VehicleHandle::new(0, 0),
            stage: "ballistic_travel",
            value: f64::INFINITY,
        }
        .to_string(),
        format!(
            "vehicle `{:?}` 的纵向计算不是 finite：stage=ballistic_travel, value=inf",
            VehicleHandle::new(0, 0)
        )
    );
    assert_eq!(
        CoreError::NonFiniteRouteTravel {
            vehicle: VehicleHandle::new(0, 0),
            speed: f64::MAX,
            delta_time_ms: 1_000,
        }
        .to_string(),
        format!(
            "vehicle `{:?}` 的 route travel distance 不是 finite：speed={}, delta=1000 ms；可通过同一 CoreWorld resolver 查询 external ID",
            VehicleHandle::new(0, 0),
            f64::MAX
        )
    );
}
