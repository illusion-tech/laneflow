//! 运行时快照的保存路径（#302 快照合同 §5；#512 切片 B）。
//!
//! 保存 = 固定步进安全边界上对已提交状态的**单一时点**捕获：只读
//! 已提交事实、把进程句柄解析为快照局部标识与稳定标识。派生状态
//! （信号灯色、占用索引、profile 派生车长）与禁绑字段（句柄 / 槽位 /
//! generation / 密集序号）不入快照。LFRS 编码待 `WorldConfig` 的
//! `route_edge_occurrence_capacity` 运行时面（#521）落地后同界衔接，
//! 不虚构值。

use laneflow_static_contract::StableId128 as ContractStableId128;
use laneflow_static_network::CanonicalNetworkOrigin;

use crate::{CommittedNetworkSource, TrafficWorld, VehicleStatus, WorldConfig};

/// LFRS 容器格式版本（快照合同 §4）。
pub const SNAPSHOT_FORMAT_VERSION: u32 = 1;
/// Runtime 逻辑状态形状轴（快照合同 §2 版本轴分离）。
pub const RUNTIME_STATE_VERSION: u16 = 1;

/// 快照局部标识的起点（1..=N 分配，0 保留为非法）。
const FIRST_SNAPSHOT_ID: u64 = 1;

/// 快照点已捕获的逻辑状态：编码无关的绑定集 + 全部每世界可变状态。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapturedSnapshot {
    /// 世界身份（快照局部）。
    pub(crate) world_id: u64,
    /// tick 游标。
    pub(crate) tick: u64,
    /// 时钟（`tick × fixed_delta_time_ms`，恢复侧核对）。
    pub(crate) time_ms: u64,
    /// 已应用输入命令计数。
    pub(crate) command_cursor: u64,
    /// 已提交事件游标（v1 无事件通道，恒零）。
    pub(crate) event_cursor: u64,
    /// 安装时冻结的世界配置。
    pub(crate) config: WorldConfig,
    /// 被绑定共享根的 LFCA origin。
    pub(crate) origin: CanonicalNetworkOrigin,
    /// 已提交路网来源。
    pub(crate) source: CommittedNetworkSource,
    /// 路线表（快照局部 ID 按 live 槽位序规范分配）。
    pub(crate) routes: Vec<CapturedRoute>,
    /// 车辆表（快照局部 ID 按 live 槽位序规范分配）。
    pub(crate) vehicles: Vec<CapturedVehicle>,
    /// live 顺序：`snapshot_vehicle_id` 的规范排序序列（实际更新顺序，
    /// 不是局部 ID 的自然序；恢复侧核对其为活跃车辆的精确排列）。
    pub(crate) live_order: Vec<u64>,
}

/// 快照路线：局部 ID + 有序边稳定标识序列（允许重复边，ADR 0029 §6）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapturedRoute {
    /// 快照局部路线 ID（本快照内唯一）。
    pub(crate) snapshot_route_id: u64,
    /// 有序边稳定标识（替代 `LaneEdgeOrdinal`）。
    pub(crate) edges: Vec<ContractStableId128>,
}

/// 快照车辆：局部 ID + 局部路线引用 + 路线序列下标与一维运动状态。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapturedVehicle {
    /// 快照局部车辆 ID（本快照内唯一）。
    pub(crate) snapshot_vehicle_id: u64,
    /// 所属快照局部路线 ID。
    pub(crate) snapshot_route_id: u64,
    /// 路线序列下标（不是路网序号）。
    pub(crate) route_edge_index: u32,
    /// 当前边进度（毫米）。
    pub(crate) progress_mm: u32,
    /// 亚毫米余数（微米）。
    pub(crate) carry_um: u16,
    /// 速度（毫米每秒）。
    pub(crate) speed_mm_s: u32,
    /// 生命周期状态。
    pub(crate) status: VehicleStatus,
    /// profile 稳定标识。
    pub(crate) profile: ContractStableId128,
    /// 参与者类别稳定标识。
    pub(crate) class: ContractStableId128,
    /// 停车位稳定标识；`None` 表示未绑定。
    pub(crate) parking_space: Option<ContractStableId128>,
}

impl CapturedSnapshot {
    /// 世界身份。
    #[must_use]
    pub const fn world_id(&self) -> u64 {
        self.world_id
    }

    /// tick 游标。
    #[must_use]
    pub const fn tick(&self) -> u64 {
        self.tick
    }

    /// 时钟（毫秒）。
    #[must_use]
    pub const fn time_ms(&self) -> u64 {
        self.time_ms
    }

    /// 已应用输入命令计数。
    #[must_use]
    pub const fn command_cursor(&self) -> u64 {
        self.command_cursor
    }

    /// 已提交事件游标（v1 恒零）。
    #[must_use]
    pub const fn event_cursor(&self) -> u64 {
        self.event_cursor
    }

    /// 安装时冻结的世界配置。
    #[must_use]
    pub const fn config(&self) -> WorldConfig {
        self.config
    }

    /// 被绑定共享根的 LFCA origin。
    #[must_use]
    pub const fn origin(&self) -> CanonicalNetworkOrigin {
        self.origin
    }

    /// 已提交路网来源。
    #[must_use]
    pub const fn source(&self) -> &CommittedNetworkSource {
        &self.source
    }

    /// 快照路线表（按局部 ID 升序）。
    #[must_use]
    pub fn routes(&self) -> &[CapturedRoute] {
        &self.routes
    }

    /// 快照车辆表（按局部 ID 升序）。
    #[must_use]
    pub fn vehicles(&self) -> &[CapturedVehicle] {
        &self.vehicles
    }

    /// live 顺序（`snapshot_vehicle_id` 序列，实际更新顺序）。
    #[must_use]
    pub fn live_order(&self) -> &[u64] {
        &self.live_order
    }
}

impl TrafficWorld {
    /// 在固定步进安全边界捕获快照点（快照合同 §5 单一时点）。
    ///
    /// 只读已提交状态，不改变世界、不推进游标。局部标识分配规范：路线
    /// 按 live 槽位序取 `1..=N`，车辆按 live 槽位序取 `1..=M`；`live_order`
    /// 保存实际更新顺序，与局部 ID 的自然序解耦。
    #[must_use]
    pub fn capture_snapshot(&self) -> CapturedSnapshot {
        let identity = self.revision.identity();

        // 路线：live 槽位序枚举，序号→稳定标识经 SharedIdentityIndex。
        let mut routes = Vec::with_capacity(usize::try_from(self.live_route_count).unwrap_or(0));
        let mut route_ids: Vec<(u32, u32, u64)> = Vec::with_capacity(routes.capacity());
        for (slot_index, slot) in self.routes.iter().enumerate() {
            if slot.compiled.is_none() {
                continue;
            }
            let handle = crate::RouteHandle::new(
                u32::try_from(slot_index).expect("route index fits u32"),
                slot.generation,
            );
            let snapshot_route_id = FIRST_SNAPSHOT_ID + routes.len() as u64;
            route_ids.push((slot.generation, handle.index(), snapshot_route_id));
            let edges: Vec<ContractStableId128> = self
                .route_edges(handle)
                .expect("live route slot resolves edges")
                .iter()
                .map(|ordinal| {
                    identity
                        .stable_id(*ordinal)
                        .map(|stable| *stable.as_untyped())
                        .expect("live edge ordinal resolves to stable id")
                })
                .collect();
            routes.push(CapturedRoute {
                snapshot_route_id,
                edges,
            });
        }
        let route_id_for = |generation: u32, index: u32| -> u64 {
            route_ids
                .iter()
                .find(|(g, i, _)| *g == generation && *i == index)
                .map(|(_, _, id)| *id)
                .expect("live vehicle route resolves to snapshot route id")
        };

        // 车辆：live 槽位序枚举，profile/class/停车位解析为稳定标识。
        let mut vehicles = Vec::with_capacity(self.live_order.len());
        let mut vehicle_ids: Vec<(u32, u32, u64)> = Vec::with_capacity(vehicles.capacity());
        for (slot_index, slot) in self.vehicles.iter().enumerate() {
            let Some(state) = slot.state.as_ref() else {
                continue;
            };
            let snapshot_vehicle_id = FIRST_SNAPSHOT_ID + vehicles.len() as u64;
            vehicle_ids.push((
                slot.generation,
                u32::try_from(slot_index).expect("vehicle index fits u32"),
                snapshot_vehicle_id,
            ));
            vehicles.push(CapturedVehicle {
                snapshot_vehicle_id,
                snapshot_route_id: route_id_for(state.route.generation(), state.route.index()),
                route_edge_index: state.route_edge_index,
                progress_mm: state.progress_mm,
                carry_um: state.carry_um,
                speed_mm_s: state.speed_mm_s,
                status: state.status,
                profile: *identity
                    .stable_id(state.profile)
                    .expect("live profile ordinal resolves to stable id")
                    .as_untyped(),
                class: *identity
                    .stable_id(state.class)
                    .expect("live class ordinal resolves to stable id")
                    .as_untyped(),
                parking_space: state.parking.map(|ordinal| {
                    *identity
                        .stable_id(ordinal)
                        .expect("parking ordinal resolves to stable id")
                        .as_untyped()
                }),
            });
        }
        let live_order = self
            .live_order
            .iter()
            .map(|handle| {
                vehicle_ids
                    .iter()
                    .find(|(g, i, _)| *g == handle.generation() && *i == handle.index())
                    .map(|(_, _, id)| *id)
                    .expect("live order handle resolves to snapshot vehicle id")
            })
            .collect();

        CapturedSnapshot {
            world_id: self.world_id,
            tick: self.tick_index,
            time_ms: self.time_ms,
            command_cursor: self.command_cursor,
            event_cursor: 0,
            config: self.config,
            origin: *self.revision.canonical_origin(),
            source: self.source.clone(),
            routes,
            vehicles,
            live_order,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cutover::tests::transaction_tests::world_with_vehicle;
    use crate::{RouteRegisterInput, TickInput, VehicleSpawnInput};

    #[test]
    fn capture_binds_cursors_config_and_origin() {
        let (world, _, _) = world_with_vehicle(true);
        let snapshot = world.capture_snapshot();
        assert_eq!(snapshot.world_id(), 1);
        assert_eq!(snapshot.tick(), 0);
        assert_eq!(snapshot.time_ms(), 0);
        assert_eq!(snapshot.command_cursor(), 2);
        assert_eq!(snapshot.event_cursor(), 0);
        assert_eq!(snapshot.config(), world.config());
        assert_eq!(snapshot.origin(), *world.revision().canonical_origin());
        assert_eq!(snapshot.source(), world.committed_source());
    }

    #[test]
    fn capture_resolves_routes_vehicles_and_live_order() {
        let (mut world, route, vehicle) = world_with_vehicle(true);
        world.step(TickInput::new(100)).expect("step");
        // 先停第一辆（Parked 不占车道），再在相同位置生成第二辆。
        let space = laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0);
        world.occupy_parking(vehicle, space).expect("parking");
        let (_, vehicle_2) = spawn_on(&mut world, route);

        let snapshot = world.capture_snapshot();
        // 路线：单条，局部 ID 1，边序解析为稳定标识。
        assert_eq!(snapshot.routes().len(), 1);
        assert_eq!(snapshot.routes()[0].snapshot_route_id, 1);
        let revision = world.revision();
        let identity = revision.identity();
        let expected_edges: Vec<ContractStableId128> = world
            .route_edges(route)
            .expect("route")
            .iter()
            .map(|ordinal| *identity.stable_id(*ordinal).expect("edge").as_untyped())
            .collect();
        assert_eq!(snapshot.routes()[0].edges, expected_edges);

        // 车辆：两辆，局部 ID 按槽位序 1、2；停车绑定在先停的第一辆上。
        assert_eq!(snapshot.vehicles().len(), 2);
        let [first, second] = snapshot.vehicles() else {
            unreachable!("两辆车");
        };
        assert_eq!(first.snapshot_vehicle_id, 1);
        assert_eq!(second.snapshot_vehicle_id, 2);
        assert_eq!(first.snapshot_route_id, 1);
        assert_eq!(second.snapshot_route_id, 1);
        assert!(first.parking_space.is_some());
        assert!(second.parking_space.is_none());
        assert_eq!(first.status, VehicleStatus::Parked);
        assert_eq!(second.status, VehicleStatus::Active);
        assert_eq!(
            first.parking_space,
            state_parking(&world, vehicle)
                .map(|ordinal| { *identity.stable_id(ordinal).expect("space").as_untyped() })
        );
        let state = world.vehicle(vehicle_2).expect("vehicle");
        assert_eq!(second.route_edge_index, state.route_edge_index());
        assert_eq!(second.progress_mm, state.progress_mm());
        assert_eq!(second.speed_mm_s, state.speed_mm_s());

        // live 顺序 = 实际更新顺序（先 1 后 2），不是槽位自然序的重复声明。
        assert_eq!(snapshot.live_order(), &[1, 2]);
    }

    #[test]
    fn capture_is_deterministic_and_side_effect_free() {
        let (mut world, _, _) = world_with_vehicle(true);
        world.step(TickInput::new(100)).expect("step");
        let before_cursor = world.command_cursor();
        let first = world.capture_snapshot();
        let second = world.capture_snapshot();
        assert_eq!(first, second);
        assert_eq!(world.command_cursor(), before_cursor);
        // 捕获后世界照常步进：单一时点捕获不持借用、不改状态。
        world.step(TickInput::new(100)).expect("step after capture");
        assert_eq!(world.command_cursor(), before_cursor);
    }

    fn spawn_on(
        world: &mut TrafficWorld,
        route: crate::RouteHandle,
    ) -> (crate::RouteHandle, crate::VehicleHandle) {
        let handle = world
            .spawn_vehicle(VehicleSpawnInput::new(
                laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                1_000,
                0,
            ))
            .expect("spawn");
        (route, handle)
    }

    fn state_parking(
        world: &TrafficWorld,
        vehicle: crate::VehicleHandle,
    ) -> Option<laneflow_static_contract::ParkingSpaceOrdinal> {
        world.vehicle(vehicle).expect("vehicle").parking
    }

    #[test]
    fn capture_survives_route_slot_reuse() {
        let (mut world, route, _) = world_with_vehicle(true);
        // 追加一条无车路线再移除，槽位回收后重新注册：局部 ID 仍按
        // live 槽位序稠密分配，不受槽位复用影响。
        let second = world
            .register_route(RouteRegisterInput::new(
                world.route_edges(route).expect("route").to_vec(),
            ))
            .expect("register second");
        world.remove_route(second).expect("remove unused route");
        let third = world
            .register_route(RouteRegisterInput::new(
                world.route_edges(route).expect("route").to_vec(),
            ))
            .expect("register third");
        let _ = third;
        let snapshot = world.capture_snapshot();
        assert_eq!(snapshot.routes().len(), 2);
        assert_eq!(snapshot.routes()[0].snapshot_route_id, 1);
        assert_eq!(snapshot.routes()[1].snapshot_route_id, 2);
    }
}
