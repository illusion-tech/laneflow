//! 单活动 LaneFlow Session：TrafficWorld + 可选 Spatial session。

use std::{collections::HashMap, mem, num::NonZeroU32, sync::Arc, time::Duration};

use bevy_ecs::entity::Entity;
use bevy_ecs::resource::Resource;
use laneflow_runtime::{
    CommittedNetworkSource, CutoverEventBatch, CutoverPreflightLimits, CutoverTransactionLimits,
    LeaveParkingTarget, LfcaOriginBinding, MigrationPolicyKind, NetworkRevisionCutoverDescriptor,
    ParkedVehicleSpawnInput, ParkedVehicleSpawnRecord, ParkingCancelRecord, ParkingCommandOutcome,
    ParkingError, ParkingLeaveRecord, ParkingParkRecord, ParkingRebindRecord, ParkingReserveRecord,
    ParkingTarget, PoseSource as RuntimePoseSource, RebindParkingTarget, ReserveParkingTarget,
    RouteError, RouteHandle, RouteRegisterInput, SemanticDiffOriginBinding, SpawnError,
    StepOutcome, TickInput, TrafficWorld, VehicleHandle, VehicleSpawnInput, WorldGeneration,
};
use laneflow_spatial::{CanonicalPoseBatch, PoseInput, PoseRecordId, SpatialSession};
use laneflow_static_network::SharedNetworkRevision;

use crate::LaneFlowAdapterError;
use crate::observation::LaneFlowJunctionObservation;

/// 把 Runtime 已提交 pose 源映射为 Spatial 批次输入。
fn pose_input(record: PoseRecordId, source: RuntimePoseSource) -> PoseInput {
    match source {
        RuntimePoseSource::Lane { edge, progress_mm } => PoseInput::lane(record, edge, progress_mm),
        RuntimePoseSource::Parking { space } => PoseInput::parking(record, space),
    }
}

/// 单活动 Session 的 fixed-schedule 配置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaneFlowSessionConfig {
    max_catch_up_steps: NonZeroU32,
}

impl LaneFlowSessionConfig {
    /// 创建显式 catch-up 上限配置。
    pub const fn new(max_catch_up_steps: NonZeroU32) -> Self {
        Self { max_catch_up_steps }
    }

    /// 返回单个 outer frame 允许的最大 step 数。
    pub const fn max_catch_up_steps(self) -> NonZeroU32 {
        self.max_catch_up_steps
    }
}

/// 最近一个 Bevy outer frame 的 LaneFlow 推进摘要。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LaneFlowFrameReport {
    frame_delta: Duration,
    steps_run: u32,
    backlog: Duration,
    catch_up_limit_reached: bool,
}

impl LaneFlowFrameReport {
    /// 返回宿主在该 outer frame 提供的 delta。
    pub const fn frame_delta(self) -> Duration {
        self.frame_delta
    }

    /// 返回该 outer frame 成功提交的 step 数。
    pub const fn steps_run(self) -> u32 {
        self.steps_run
    }

    /// 返回该 outer frame 结束后完整保留的时间 backlog。
    pub const fn backlog(self) -> Duration {
        self.backlog
    }

    /// 返回是否因为达到配置上限而仍有至少一个完整 fixed quantum 待处理。
    pub const fn catch_up_limit_reached(self) -> bool {
        self.catch_up_limit_reached
    }
}

/// 一个 Bevy `App` 中唯一活动的 LaneFlow runtime resource。
#[derive(Resource)]
pub struct LaneFlowSession {
    world: TrafficWorld,
    spatial: Option<SpatialSession>,
    config: LaneFlowSessionConfig,
    accumulator: Duration,
    frame_report: LaneFlowFrameReport,
    frame_step_results: Vec<StepOutcome>,
    pub(crate) last_error: Option<LaneFlowAdapterError>,
    vehicle_entities: VehicleEntityMap,
    pose_scratch: Vec<PoseInput>,
    pose_vehicle_scratch: Vec<VehicleHandle>,
}

impl LaneFlowSession {
    /// 创建 Session。若提供 Spatial，必须与 world 满足 `Arc::ptr_eq`。
    ///
    /// # Errors
    ///
    /// 提供的 `spatial` 与 world 活动根不满足 `Arc::ptr_eq` 时返回
    /// [`LaneFlowAdapterError::RevisionMismatch`]。
    pub fn new(
        world: TrafficWorld,
        spatial: Option<SpatialSession>,
        config: LaneFlowSessionConfig,
    ) -> Result<Self, LaneFlowAdapterError> {
        if let Some(session) = spatial.as_ref()
            && !Arc::ptr_eq(&world.revision(), &session.revision())
        {
            return Err(LaneFlowAdapterError::RevisionMismatch);
        }
        let vehicle_entities = VehicleEntityMap::with_capacity(
            usize::try_from(world.config().vehicle_capacity()).unwrap_or(0),
        );
        Ok(Self {
            world,
            spatial,
            config,
            accumulator: Duration::ZERO,
            frame_report: LaneFlowFrameReport::default(),
            frame_step_results: Vec::new(),
            last_error: None,
            vehicle_entities,
            pose_scratch: Vec::new(),
            pose_vehicle_scratch: Vec::new(),
        })
    }

    /// 已提交交通世界。
    pub const fn world(&self) -> &TrafficWorld {
        &self.world
    }

    /// 借用活动 Session 的复杂路口领域只读观测视图（#285 复杂路口观测
    /// G1 §3）。
    ///
    /// 创建为 O(1) 且无堆分配；视图借用期间借用规则禁止对同一 Session
    /// 推进或提交生命周期命令。headless Session 也能读取领域观察；几何
    /// 绘制另外要求有效 Spatial 配对。
    #[must_use]
    pub const fn junction_observation(&self) -> LaneFlowJunctionObservation<'_> {
        LaneFlowJunctionObservation::new(&self.world)
    }

    /// 两次 `step` 之间提交 route、spawn 与 parking lifecycle 命令。
    ///
    /// 已绑定车辆的原子替换必须走 [`crate::replace_completed_vehicle`]，避免 Runtime
    /// 成功后留下 stale 映射；真正移除必须走 [`crate::despawn_vehicle`]，先验证宿主
    /// Entity 再组合 Runtime removal 与 mapping 清理。
    pub const fn world_mut(&mut self) -> LaneFlowWorldMut<'_> {
        LaneFlowWorldMut {
            world: &mut self.world,
        }
    }

    pub(crate) const fn runtime_mut(&mut self) -> &mut TrafficWorld {
        &mut self.world
    }

    /// 可选 Spatial session。只读检视；位姿提取必须走
    /// [`Self::extract_committed_pose_batch`] 的受控区间。
    pub const fn spatial(&self) -> Option<&SpatialSession> {
        self.spatial.as_ref()
    }

    /// 当前消费上下文（世界身份 + 世代）。
    ///
    /// 位姿批次仅在采集时的上下文仍为当前世代时可应用到本世界；任何成功
    /// 切换（跨修订或同修订换根）都会使先前上下文过期。
    #[must_use]
    pub const fn consumption_context(&self) -> LaneFlowConsumptionContext {
        LaneFlowConsumptionContext {
            world_id: self.world.world_id(),
            world_generation: self.world.world_generation(),
        }
    }

    /// 校验一批结果的消费上下文是否仍为当前；过期结果整批拒绝。
    #[must_use]
    pub fn consumption_context_is_current(&self, context: LaneFlowConsumptionContext) -> bool {
        self.consumption_context() == context
    }

    /// v1 同步封闭路径：同一调用内采集世界已提交位姿源、校验根配对并按
    /// 当前配对 Spatial 提取批次（#534 G1 冻结形态）。
    ///
    /// 采集 → 校验 → 提取处于单次 `&mut self` 内，不存在插入切换的窗口；
    /// 调用方不得缓存位姿输入跨过切换边界重放。`output.vehicles()` 与
    /// `output.batch().records()` 按序对齐（记录身份为过滤后连续序号）。
    ///
    /// 一次消费来源迭代器，同时构建候选输入与候选车辆序列两组 Session
    /// 缓冲；Spatial 采样成功后的最终提交只剩缓冲交换、接管清空与上下文
    /// 值更新，不再执行可返回错误的工作。稳定容量复用（adapter-api §6）：
    /// 调用方持有 `output` 跨帧复用，Session 与参与 output 均暖机后零新增
    /// 分配；任一失败路径整个 `output`（车辆序列、批次、上下文）原样保持。
    ///
    /// # Errors
    ///
    /// 当前没有配对 Spatial（[`LaneFlowAdapterError::PoseExtractionWithoutSpatial`]）、
    /// Spatial 与世界活动根 `Arc::ptr_eq` 失配（[`LaneFlowAdapterError::RevisionMismatch`]）
    /// 或 Spatial 批次提取失败（[`LaneFlowAdapterError::SpatialPoseExtraction`]）时返回；
    /// 失败不产生部分批次。
    pub fn extract_committed_pose_batch(
        &mut self,
        placement_token: laneflow_spatial::FramePlacementToken,
        output: &mut LaneFlowCommittedPoseBatch,
    ) -> Result<(), LaneFlowAdapterError> {
        // 创建迭代器执行公开入口检查；创建本身不扫描来源。
        let sources = self.world.committed_pose_sources();
        let context = self.consumption_context();
        let Some(spatial) = self.spatial.as_mut() else {
            return Err(LaneFlowAdapterError::PoseExtractionWithoutSpatial);
        };
        // 每批固定 O(1) 配对检查（与车辆数无关）；`revision()` 各克隆一次 Arc。
        if !Arc::ptr_eq(&self.world.revision(), &spatial.revision()) {
            return Err(LaneFlowAdapterError::RevisionMismatch);
        }
        // 一次遍历同时构建两份候选；连续记录序号在来源过滤之后产生，
        // 与实际输出车辆一一对应。
        self.pose_scratch.clear();
        self.pose_vehicle_scratch.clear();
        for (vehicle, source) in sources {
            let record = u32::try_from(self.pose_vehicle_scratch.len())
                .expect("pose record index fits vehicle capacity");
            self.pose_scratch
                .push(pose_input(PoseRecordId::new(record), source));
            self.pose_vehicle_scratch.push(vehicle);
        }
        // 全部可失败工作已完成；提交阶段只做所有权交换与值更新。
        spatial
            .extract_pose_batch(placement_token, &self.pose_scratch, &mut output.batch)
            .map_err(|source| LaneFlowAdapterError::SpatialPoseExtraction { source })?;
        mem::swap(&mut self.pose_vehicle_scratch, &mut output.vehicles);
        // 接管上一批输出的车辆存储，保留容量供下一批候选复用。
        self.pose_vehicle_scratch.clear();
        output.context = context;
        Ok(())
    }

    /// 同步维护暂停式跨修订直移切换（切换合同 §4；#534 G1 冻结形态）。
    ///
    /// 单次 `&mut self` 内完成 Runtime `prepare` → `commit`，无步进交错。
    /// 目标 Spatial 的全部可失败验证在 Runtime `prepare` 之前完成；
    /// Runtime `commit` 成功后只剩不可失败的配对替换。成功返回时当前
    /// 配对已指向目标根，不存在生产可见的「新世界 + 旧 Spatial」中间态。
    /// 事务对象由本方法独占持有，任一失败路径都不会遗留在途事务
    /// （`commit` 按值消耗并无条件解除武装）。
    ///
    /// 调用时机：任意不处于 fixed step 执行中的宿主系统；推荐
    /// `LaneFlowOuterFrameSet::Administration`（零步进 outer frame 亦运行）。
    ///
    /// # Errors
    ///
    /// 目标 Spatial 与目标根 `Arc::ptr_eq` 失配时返回
    /// [`LaneFlowAdapterError::TargetSpatialRevisionMismatch`]（先于 Runtime `prepare`）；
    /// Runtime `prepare`/`commit` 失败包装为 [`LaneFlowAdapterError::Cutover`] 返回。任一
    /// 失败当前配对保持不变，不存在半切换状态。
    #[allow(clippy::too_many_arguments)]
    pub fn cross_revision_cutover(
        &mut self,
        target_revision: Arc<SharedNetworkRevision>,
        target_source: CommittedNetworkSource,
        semantic_diff: &[u8],
        diff_binding: SemanticDiffOriginBinding,
        target_spatial: LaneFlowTargetSpatial,
        preflight_limits: &CutoverPreflightLimits,
        transaction_limits: &CutoverTransactionLimits,
    ) -> Result<LaneFlowCutoverRecord, LaneFlowAdapterError> {
        let target_spatial = Self::validated_target_spatial(target_spatial, &target_revision)?;
        let descriptor = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(*self.world.revision().canonical_origin()),
            LfcaOriginBinding::from_canonical_origin(*target_revision.canonical_origin()),
            Some(diff_binding),
            MigrationPolicyKind::CrossRevisionDirect,
            self.world.world_binding(),
        );
        let transaction = self
            .world
            .prepare_cross_revision_cutover(
                target_revision,
                target_source,
                &descriptor,
                semantic_diff,
                preflight_limits,
                transaction_limits,
            )
            .map_err(|source| LaneFlowAdapterError::Cutover { source })?;
        let commit = transaction
            .commit(&mut self.world)
            .map_err(|source| LaneFlowAdapterError::Cutover { source })?;
        Ok(self.swap_spatial(target_spatial, commit.events))
    }

    /// 同步维护暂停式同修订换根（`same_revision_restore`；#534 G1 冻结形态）。
    ///
    /// 修订号不变、根 `Arc` 换新、世代递增；配对语义与跨修订入口一致：
    /// 成功返回时已完成新配对。目标根必须与当前根修订号相同，否则由
    /// Runtime 描述符认证失败关闭。
    ///
    /// # Errors
    ///
    /// 目标 Spatial 与目标根 `Arc::ptr_eq` 失配时返回
    /// [`LaneFlowAdapterError::TargetSpatialRevisionMismatch`]（先于 Runtime 调用）；
    /// Runtime 同修订换根失败包装为 [`LaneFlowAdapterError::Cutover`] 返回。任一失败
    /// 当前配对保持不变，不存在半切换状态。
    pub fn same_revision_restore(
        &mut self,
        target_revision: Arc<SharedNetworkRevision>,
        target_source: CommittedNetworkSource,
        target_spatial: LaneFlowTargetSpatial,
        preflight_limits: &CutoverPreflightLimits,
    ) -> Result<LaneFlowCutoverRecord, LaneFlowAdapterError> {
        let target_spatial = Self::validated_target_spatial(target_spatial, &target_revision)?;
        let descriptor = NetworkRevisionCutoverDescriptor::new(
            LfcaOriginBinding::from_canonical_origin(*self.world.revision().canonical_origin()),
            LfcaOriginBinding::from_canonical_origin(*target_revision.canonical_origin()),
            None,
            MigrationPolicyKind::SameRevisionRestore,
            self.world.world_binding(),
        );
        let events = self
            .world
            .cutover_same_revision(
                target_revision,
                target_source,
                &descriptor,
                preflight_limits,
            )
            .map_err(|source| LaneFlowAdapterError::Cutover { source })?;
        Ok(self.swap_spatial(target_spatial, events))
    }

    /// 目标 Spatial 的可失败验证全部前置于 Runtime 事务之前（G1 冻结）。
    fn validated_target_spatial(
        target_spatial: LaneFlowTargetSpatial,
        target_revision: &Arc<SharedNetworkRevision>,
    ) -> Result<Option<SpatialSession>, LaneFlowAdapterError> {
        match target_spatial {
            LaneFlowTargetSpatial::Rebind(session) => {
                if !Arc::ptr_eq(&session.revision(), target_revision) {
                    return Err(LaneFlowAdapterError::TargetSpatialRevisionMismatch);
                }
                Ok(Some(session))
            }
            // 宿主显式选择转 headless；不存在静默降级路径。
            LaneFlowTargetSpatial::Headless => Ok(None),
        }
    }

    /// Runtime 切换成功后的不可失败配对替换；返回换出的旧 Spatial。
    fn swap_spatial(
        &mut self,
        target: Option<SpatialSession>,
        events: CutoverEventBatch,
    ) -> LaneFlowCutoverRecord {
        LaneFlowCutoverRecord {
            retired_spatial: mem::replace(&mut self.spatial, target),
            world_binding: self.world.world_binding(),
            events,
        }
    }

    /// Session 配置。
    pub const fn config(&self) -> LaneFlowSessionConfig {
        self.config
    }

    /// 最近一个 outer frame 的推进摘要。
    pub const fn frame_report(&self) -> LaneFlowFrameReport {
        self.frame_report
    }

    /// 最近一个 outer frame 中按执行顺序提交的步进结果。
    pub fn frame_step_results(&self) -> &[StepOutcome] {
        &self.frame_step_results
    }

    /// 最近失败。
    pub const fn last_error(&self) -> Option<&LaneFlowAdapterError> {
        self.last_error.as_ref()
    }

    /// 把 live 车辆绑到宿主 Entity。未绑定车辆保持未绑定。
    ///
    /// # Errors
    ///
    /// 车辆不在当前世界（[`LaneFlowAdapterError::UnknownVehicle`]）、已绑定其它
    /// Entity（[`LaneFlowAdapterError::DuplicateVehicleBinding`]）或目标 Entity 已被
    /// 其它车辆映射（[`LaneFlowAdapterError::DuplicateEntityBinding`]）时返回相应
    /// [`LaneFlowAdapterError`]。
    pub fn bind_vehicle_entity(
        &mut self,
        vehicle: VehicleHandle,
        entity: Entity,
    ) -> Result<(), LaneFlowAdapterError> {
        if self.world.vehicle(vehicle).is_none() {
            return Err(LaneFlowAdapterError::UnknownVehicle { vehicle });
        }
        self.vehicle_entities.bind(vehicle, entity)
    }

    /// 解除车辆绑定。
    ///
    /// # Errors
    ///
    /// 车辆当前没有 Entity 绑定时返回 [`LaneFlowAdapterError::UnknownVehicle`]。
    pub fn unbind_vehicle(
        &mut self,
        vehicle: VehicleHandle,
    ) -> Result<Entity, LaneFlowAdapterError> {
        self.vehicle_entities.unbind_vehicle(vehicle)
    }

    /// 查询车辆当前绑定的 Entity。
    #[must_use]
    pub fn vehicle_entity(&self, vehicle: VehicleHandle) -> Option<Entity> {
        self.vehicle_entities.entity(vehicle)
    }

    pub(crate) fn validate_replacement(
        &self,
        old: VehicleHandle,
    ) -> Result<Option<Entity>, LaneFlowAdapterError> {
        if self.world.vehicle(old).is_none() {
            return Err(LaneFlowAdapterError::UnknownVehicle { vehicle: old });
        }
        Ok(self.vehicle_entities.entity(old))
    }

    pub(crate) fn rotate_replaced_vehicle(
        &mut self,
        old: VehicleHandle,
        new: VehicleHandle,
        entity: Option<Entity>,
    ) {
        self.vehicle_entities.rotate(old, new, entity);
    }

    pub(crate) fn prepare_despawned_vehicle(
        &self,
        vehicle: VehicleHandle,
    ) -> Option<PreparedVehicleEntityRemoval> {
        self.vehicle_entities.prepare_remove(vehicle)
    }

    pub(crate) fn commit_despawned_vehicle(
        &mut self,
        prepared: Option<PreparedVehicleEntityRemoval>,
    ) -> Option<Entity> {
        prepared.map(|prepared| self.vehicle_entities.commit_remove(prepared))
    }

    pub(crate) fn fixed_quantum(&self) -> Duration {
        Duration::from_millis(self.world.config().fixed_delta_time_ms())
    }

    pub(crate) fn begin_outer_frame(&mut self, frame_delta: Duration) -> bool {
        self.frame_step_results.clear();
        self.last_error = None;
        self.frame_report = LaneFlowFrameReport {
            frame_delta,
            steps_run: 0,
            backlog: self.accumulator,
            catch_up_limit_reached: false,
        };
        let Some(accumulator) = self.accumulator.checked_add(frame_delta) else {
            self.last_error = Some(LaneFlowAdapterError::AccumulatorOverflow {
                backlog: self.accumulator,
                frame_delta,
            });
            return false;
        };
        self.accumulator = accumulator;
        true
    }

    pub(crate) fn record_missing_time(&mut self) {
        self.frame_step_results.clear();
        self.last_error = Some(LaneFlowAdapterError::MissingTimeResource);
        self.frame_report = LaneFlowFrameReport {
            frame_delta: Duration::ZERO,
            steps_run: 0,
            backlog: self.accumulator,
            catch_up_limit_reached: false,
        };
    }

    pub(crate) fn can_step(&self) -> bool {
        self.last_error.is_none() && self.accumulator >= self.fixed_quantum()
    }

    pub(crate) fn step_world(&mut self) {
        if self.last_error.is_some() {
            return;
        }
        let delta = self.world.config().fixed_delta_time_ms();
        match self.world.step(TickInput::new(delta)) {
            Ok(result) => {
                self.accumulator = self
                    .accumulator
                    .checked_sub(self.fixed_quantum())
                    .unwrap_or(Duration::ZERO);
                self.frame_report.steps_run = self.frame_report.steps_run.saturating_add(1);
                self.frame_step_results.push(result);
            }
            Err(error) => {
                self.last_error = Some(LaneFlowAdapterError::StepFailed(error));
            }
        }
    }

    pub(crate) fn finish_outer_frame(&mut self) {
        self.frame_report.backlog = self.accumulator;
        self.frame_report.catch_up_limit_reached =
            self.last_error.is_none() && self.accumulator >= self.fixed_quantum();
    }
}

/// `world_mut` 可提交的生命周期命令。不含 replace，以免绕过映射轮换。
pub struct LaneFlowWorldMut<'a> {
    world: &'a mut TrafficWorld,
}

impl LaneFlowWorldMut<'_> {
    /// 生成一辆车。
    ///
    /// # Errors
    ///
    /// 与 [`TrafficWorld::spawn_vehicle`] 相同；本包装不新增失败面。
    pub fn spawn_vehicle(&mut self, input: VehicleSpawnInput) -> Result<VehicleHandle, SpawnError> {
        self.world.spawn_vehicle(input)
    }

    /// 注册本世界路线。
    ///
    /// # Errors
    ///
    /// 与 [`TrafficWorld::register_route`] 相同；本包装不新增失败面。
    pub fn register_route(&mut self, input: RouteRegisterInput) -> Result<RouteHandle, RouteError> {
        self.world.register_route(input)
    }

    /// 移除本世界路线。
    ///
    /// # Errors
    ///
    /// 与 [`TrafficWorld::remove_route`] 相同；本包装不新增失败面。
    pub fn remove_route(&mut self, route: RouteHandle) -> Result<(), RouteError> {
        self.world.remove_route(route)
    }

    /// 预留精确停车 target/payload。
    ///
    /// # Errors
    ///
    /// 与 [`TrafficWorld::reserve_parking`] 相同；本包装不新增失败面。
    pub fn reserve_parking(
        &mut self,
        vehicle: VehicleHandle,
        target: ReserveParkingTarget,
    ) -> Result<ParkingCommandOutcome<ParkingReserveRecord>, ParkingError> {
        self.world.reserve_parking(vehicle, target)
    }

    /// 取消 exact reservation。
    ///
    /// # Errors
    ///
    /// 与 [`TrafficWorld::cancel_parking`] 相同；本包装不新增失败面。
    pub fn cancel_parking(
        &mut self,
        vehicle: VehicleHandle,
        target: ParkingTarget,
    ) -> Result<ParkingCancelRecord, ParkingError> {
        self.world.cancel_parking(vehicle, target)
    }

    /// 提交 exact arrived reservation。
    ///
    /// # Errors
    ///
    /// 与 [`TrafficWorld::park_vehicle`] 相同；本包装不新增失败面。
    pub fn park_vehicle(
        &mut self,
        vehicle: VehicleHandle,
        target: ParkingTarget,
    ) -> Result<ParkingCommandOutcome<ParkingParkRecord>, ParkingError> {
        self.world.park_vehicle(vehicle, target)
    }

    /// 从 parking target 安全插回 lane。
    ///
    /// # Errors
    ///
    /// 与 [`TrafficWorld::leave_parking`] 相同；本包装不新增失败面。
    pub fn leave_parking(
        &mut self,
        vehicle: VehicleHandle,
        target: LeaveParkingTarget,
    ) -> Result<ParkingLeaveRecord, ParkingError> {
        self.world.leave_parking(vehicle, target)
    }

    /// 在完整 footprint 相等时重绑 reservation route。
    ///
    /// # Errors
    ///
    /// 与 [`TrafficWorld::rebind_parking_route`] 相同；本包装不新增失败面。
    pub fn rebind_parking_route(
        &mut self,
        vehicle: VehicleHandle,
        target: RebindParkingTarget,
    ) -> Result<ParkingCommandOutcome<ParkingRebindRecord>, ParkingError> {
        self.world.rebind_parking_route(vehicle, target)
    }

    /// 直接构造 `Parked + Occupied`，不建立 lane pose。
    ///
    /// # Errors
    ///
    /// 与 [`TrafficWorld::spawn_parked_vehicle`] 相同；本包装不新增失败面。
    pub fn spawn_parked_vehicle(
        &mut self,
        input: ParkedVehicleSpawnInput,
        target: ParkingTarget,
    ) -> Result<ParkedVehicleSpawnRecord, ParkingError> {
        self.world.spawn_parked_vehicle(input, target)
    }
}

/// 切换目标的 Spatial 配对方式（#534 G1 冻结）。
///
/// `Rebind` 携带已绑定目标根的 `SpatialSession`，入口校验其与目标根
/// `Arc::ptr_eq`；`Headless` 是宿主对「切换后无表现」的显式选择——枚举
/// 形态保证不存在把 `Some(Spatial)` 静默降级为 `None` 的路径。
pub enum LaneFlowTargetSpatial {
    /// 按目标根重绑表现；`SpatialSession::bind(target_root)` 的产物。
    Rebind(SpatialSession),
    /// 宿主显式选择切换后 headless。
    Headless,
}

/// 一次成功切换的结果。
///
/// `retired_spatial` 是换出的旧 `SpatialSession`：它对旧根的历史读取仍
/// 合法（在途借用可完成），但其结果不得作为当前世界的表现提交。
///
/// `#[must_use]` 继承 Runtime 对 `CutoverCommit` / `CutoverEventBatch` 的
/// 恰一次交付义务（切换合同 §10）：语句位丢弃本记录即丢弃事件交付。
#[must_use = "切换事件批次恰一次交付；丢弃记录会静默丢弃交付"]
pub struct LaneFlowCutoverRecord {
    retired_spatial: Option<SpatialSession>,
    world_binding: laneflow_runtime::WorldBinding,
    events: CutoverEventBatch,
}

impl std::fmt::Debug for LaneFlowCutoverRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LaneFlowCutoverRecord")
            .field(
                "retired_spatial",
                &self.retired_spatial.as_ref().map(|_| "SpatialSession"),
            )
            .field("world_binding", &self.world_binding)
            .field("events", &self.events.as_slice())
            .finish()
    }
}

impl LaneFlowCutoverRecord {
    /// 换出的旧 Spatial session；调用方负责在途借用收尾与释放。
    /// 以 `&mut self` 取出而非消耗记录：取出后仍可继续读取
    /// [`Self::events`] 与 [`Self::world_binding`]。
    pub fn retired_spatial(&mut self) -> Option<SpatialSession> {
        self.retired_spatial.take()
    }

    /// 切换后的世界绑定（身份、世代与双基线游标）。
    #[must_use]
    pub const fn world_binding(&self) -> laneflow_runtime::WorldBinding {
        self.world_binding
    }

    /// 恰一次交付的切换事件批次（#302 切换合同 §6）。
    pub const fn events(&self) -> &CutoverEventBatch {
        &self.events
    }
}

/// 位姿结果的消费上下文：世界身份 + 世界世代。
///
/// 任何成功切换（跨修订或同修订换根）都使先前上下文过期；世代是
/// Runtime 签发的权威轴，Adapter 只消费不重建。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LaneFlowConsumptionContext {
    world_id: u64,
    world_generation: WorldGeneration,
}

impl LaneFlowConsumptionContext {
    /// 世界身份（宿主指定的 `world_id`）。
    #[must_use]
    pub const fn world_id(&self) -> u64 {
        self.world_id
    }

    /// 采集批次时的世界世代。
    #[must_use]
    pub const fn world_generation(&self) -> WorldGeneration {
        self.world_generation
    }
}

/// 受控提取区间产出：位姿批次 + 消费上下文 + 与记录对齐的车辆句柄。
///
/// 调用方持有一份跨帧复用（adapter-api §6 稳定容量合同）：稳态提取
/// 原地重填本结构，容量保持；首次成功提取前 `context` 为占位值。
#[derive(Debug)]
pub struct LaneFlowCommittedPoseBatch {
    batch: CanonicalPoseBatch,
    context: LaneFlowConsumptionContext,
    vehicles: Vec<VehicleHandle>,
}

impl Default for LaneFlowCommittedPoseBatch {
    fn default() -> Self {
        Self {
            batch: CanonicalPoseBatch::new(),
            context: LaneFlowConsumptionContext {
                world_id: 0,
                world_generation: WorldGeneration::INITIAL,
            },
            vehicles: Vec::new(),
        }
    }
}

impl LaneFlowCommittedPoseBatch {
    /// 空批次缓冲；跨帧复用时仅需构造一次。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 提取出的共享根位姿批次。
    #[must_use]
    pub const fn batch(&self) -> &CanonicalPoseBatch {
        &self.batch
    }

    /// 采集时的消费上下文；应用前用
    /// [`LaneFlowSession::consumption_context_is_current`] 复核。
    #[must_use]
    pub const fn context(&self) -> LaneFlowConsumptionContext {
        self.context
    }

    /// 与 `batch().records()` 按序对齐的车辆句柄（记录身份为序号）。
    #[must_use]
    pub fn vehicles(&self) -> &[VehicleHandle] {
        &self.vehicles
    }
}

#[derive(Clone, Debug, Default)]
struct VehicleEntityMap {
    // 只作完整代际句柄的查找，不以 HashMap 迭代顺序决定提取、应用或求解顺序。
    by_vehicle: HashMap<VehicleHandle, Entity>,
    by_entity: HashMap<Entity, VehicleHandle>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PreparedVehicleEntityRemoval {
    vehicle: VehicleHandle,
    entity: Entity,
}

impl PreparedVehicleEntityRemoval {
    pub(crate) const fn entity(self) -> Entity {
        self.entity
    }
}

impl VehicleEntityMap {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            by_vehicle: HashMap::with_capacity(capacity),
            by_entity: HashMap::with_capacity(capacity),
        }
    }

    fn entity(&self, vehicle: VehicleHandle) -> Option<Entity> {
        self.by_vehicle.get(&vehicle).copied()
    }

    fn bind(&mut self, vehicle: VehicleHandle, entity: Entity) -> Result<(), LaneFlowAdapterError> {
        if let Some(existing) = self.entity(vehicle) {
            return Err(LaneFlowAdapterError::DuplicateVehicleBinding {
                vehicle,
                existing,
                requested: entity,
            });
        }
        if let Some(&existing) = self.by_entity.get(&entity) {
            return Err(LaneFlowAdapterError::DuplicateEntityBinding {
                entity,
                existing,
                requested: vehicle,
            });
        }
        self.by_vehicle.insert(vehicle, entity);
        self.by_entity.insert(entity, vehicle);
        Ok(())
    }

    fn unbind_vehicle(&mut self, vehicle: VehicleHandle) -> Result<Entity, LaneFlowAdapterError> {
        let prepared = self
            .prepare_remove(vehicle)
            .ok_or(LaneFlowAdapterError::UnknownVehicle { vehicle })?;
        Ok(self.commit_remove(prepared))
    }

    fn rotate(&mut self, old: VehicleHandle, new: VehicleHandle, entity: Option<Entity>) {
        let Some(entity) = entity else {
            return;
        };
        debug_assert_eq!(self.entity(old), Some(entity));
        self.by_vehicle.remove(&old);
        debug_assert!(!self.by_vehicle.contains_key(&new));
        self.by_vehicle.insert(new, entity);
        self.by_entity.insert(entity, new);
    }

    fn prepare_remove(&self, vehicle: VehicleHandle) -> Option<PreparedVehicleEntityRemoval> {
        let entity = self.entity(vehicle)?;
        Some(PreparedVehicleEntityRemoval { vehicle, entity })
    }

    fn commit_remove(&mut self, prepared: PreparedVehicleEntityRemoval) -> Entity {
        let removed = self
            .by_vehicle
            .remove(&prepared.vehicle)
            .expect("prepared binding remains live");
        debug_assert_eq!(removed, prepared.entity);
        self.by_entity.remove(&removed);
        removed
    }
}
// 追加到 session.rs 末尾的容量观测测试模块（由脚本拼接）。
#[cfg(test)]
mod capacity_tests {
    //! #712 容量轨迹观测：Adapter 三块自有缓冲的实际 len/capacity 行为。
    //!
    //! 夹具：8 条 frame-main 平行车道 + 1 条 frame-alt 车道 + 虚拟池设施。
    //! 只断言可观察合同（容量保留、轮换、旧尾部不可见、失败不变），不假设
    //! Vec 的具体增长倍数。元素大小在断言消息中记录，供报告引用。

    use std::{num::NonZeroU32, sync::Arc};

    use laneflow_compiler::{
        CanonicalFrameInput, CanonicalPoint3F32Input, CompilationUnitBuilder, CompileLimits,
        Compiler, IidmVehicleProfileInput, LaneEdgeGeometryInput, LaneEdgeInput, LaneEdgeReference,
        ParkingFacilityInput, ParkingLaneAnchorInput, ParticipantClassInput,
        ParticipantClassReference, PortableDiffBase, PortableEmissionProvenance,
        SourceModuleHeader, SourceModuleHeaderInput, SyntheticModuleBuilder, VehicleProfileInput,
        emit_portable_candidate,
    };
    use laneflow_format::{FormatLimits, check_post_emission_bundle};
    use laneflow_runtime::{
        CommittedNetworkSource, PublishedLfcaReference, ReserveParkingTarget, RouteHandle,
        RouteRegisterInput, TrafficWorld, VehicleHandle, VehicleSpawnInput,
        VirtualEntryAnchorSelector, WorldConfig, WorldPolicySelection,
    };
    use laneflow_spatial::SpatialSession;
    use laneflow_static_contract::{EntityKind, LaneEdgeId, VehicleProfileOrdinal};

    use super::{LaneFlowCommittedPoseBatch, LaneFlowSession, LaneFlowSessionConfig};

    const NAMESPACE: &str = "bevy/session-capacity";
    const PROFILE: VehicleProfileOrdinal = VehicleProfileOrdinal::from_raw(0);
    const MAIN_LANES: usize = 8;
    const ALT_LANES: usize = 1;
    const LANE_LENGTH_MM: u32 = 200_000;
    const FACILITY: laneflow_static_contract::ParkingFacilityOrdinal =
        laneflow_static_contract::ParkingFacilityOrdinal::from_raw(0);

    fn edge_ordinal(
        root: &laneflow_static_network::SharedNetworkRevision,
        key: &str,
    ) -> laneflow_static_contract::LaneEdgeOrdinal {
        let stable = laneflow_compiler::derive_canonical_stable_id_v1(
            EntityKind::LaneEdge,
            NAMESPACE,
            key,
            &CompileLimits::p100_initial_v1(),
        )
        .expect("edge stable id");
        root.identity()
            .ordinal(LaneEdgeId::from_untyped(stable))
            .expect("edge ordinal")
    }

    fn revision() -> Arc<laneflow_static_network::SharedNetworkRevision> {
        let limits = CompileLimits::p100_initial_v1();
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: NAMESPACE,
                source_document_key: "session-capacity.document",
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x81; 32],
                frontend_options_digest: [0x31; 32],
                random_seed: Some(725),
                provenance: "repository:laneflow",
            },
            &limits,
        )
        .expect("header");
        let mut module = SyntheticModuleBuilder::new(header, &limits).expect("module");
        let key = |lane: usize, alt: bool| {
            if alt {
                format!("alt-{lane}")
            } else {
                format!("main-{lane}")
            }
        };
        let main_keys: Vec<&'static str> = (0..MAIN_LANES)
            .map(|lane| -> &'static str { Box::leak(key(lane, false).into_boxed_str()) })
            .collect();
        let alt_keys: Vec<&'static str> = (0..ALT_LANES)
            .map(|lane| -> &'static str { Box::leak(key(lane, true).into_boxed_str()) })
            .collect();
        module
            .add_participant_class(ParticipantClassInput {
                participant_class_key: "road-user",
                extends: None,
            })
            .expect("class")
            .add_vehicle_profile(VehicleProfileInput {
                vehicle_profile_key: "car",
                participant_class: ParticipantClassReference::local("road-user"),
                iidm: IidmVehicleProfileInput {
                    length_meters: 4.5,
                    desired_speed_meters_per_second: 13.75,
                    min_gap_meters: 2.0,
                    time_headway_seconds: 1.4,
                    max_acceleration_meters_per_second_squared: 1.8,
                    comfortable_deceleration_meters_per_second_squared: 2.0,
                    emergency_deceleration_meters_per_second_squared: 4.5,
                },
            })
            .expect("profile");
        for key in main_keys.iter().chain(alt_keys.iter()) {
            module
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: key,
                    length_meters: 200.0,
                    speed_limit_meters_per_second: 15.0,
                    successors: &[],
                })
                .expect("lane edge");
        }
        fn anchor(edge: &str, progress: f64) -> ParkingLaneAnchorInput<'_> {
            ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local(edge),
                progress_meters: progress,
            }
        }
        module
            .add_parking_facility(ParkingFacilityInput {
                parking_facility_key: "facility",
                virtual_capacity: 4,
                virtual_entries: &[anchor(alt_keys[0], 10.0)],
                virtual_exits: &[anchor(alt_keys[0], 20.0)],
            })
            .expect("facility");
        let point = |x: f32, z: f32| CanonicalPoint3F32Input { x, y: 0.0, z };
        let main_points: Vec<_> = (0..MAIN_LANES)
            .map(|index| {
                [
                    point(0.0, (index as f32) * 3.0),
                    point(200.0, (index as f32) * 3.0),
                ]
            })
            .collect();
        let main_geometry: Vec<LaneEdgeGeometryInput> = main_keys
            .iter()
            .zip(main_points.iter())
            .map(|(key, points)| LaneEdgeGeometryInput {
                lane_edge: LaneEdgeReference::local(key),
                centerline_points: points,
            })
            .collect();
        module
            .add_canonical_frame(CanonicalFrameInput {
                canonical_frame_key: "frame-main",
                lane_edge_geometries: &main_geometry,
            })
            .expect("frame-main");
        let alt_points: Vec<_> = (0..ALT_LANES)
            .map(|index| {
                [
                    point(0.0, 100.0 + (index as f32) * 3.0),
                    point(200.0, 100.0 + (index as f32) * 3.0),
                ]
            })
            .collect();
        let alt_geometry: Vec<LaneEdgeGeometryInput> = alt_keys
            .iter()
            .zip(alt_points.iter())
            .map(|(key, points)| LaneEdgeGeometryInput {
                lane_edge: LaneEdgeReference::local(key),
                centerline_points: points,
            })
            .collect();
        module
            .add_canonical_frame(CanonicalFrameInput {
                canonical_frame_key: "frame-alt",
                lane_edge_geometries: &alt_geometry,
            })
            .expect("frame-alt");
        let mut unit = CompilationUnitBuilder::new(limits);
        unit.add_synthetic_module(module.finish().expect("module"))
            .expect("unit module");
        let output = Compiler::new()
            .compile(unit.build().expect("unit"))
            .expect("compile");
        let provenance =
            PortableEmissionProvenance::try_new("bevy-session-capacity-v1").expect("provenance");
        let candidate = emit_portable_candidate(
            &output,
            &provenance,
            FormatLimits::HARD,
            PortableDiffBase::Genesis,
        )
        .expect("candidate");
        let checked = check_post_emission_bundle(
            candidate.canonical_artifact().bytes(),
            candidate.source_map().bytes(),
            candidate.semantic_diff().bytes(),
            candidate.expected_semantic_diff_base(),
            FormatLimits::HARD,
        )
        .expect("bundle");
        laneflow_static_network::build_shared_network_revision(
            checked.canonical_network_input(),
            laneflow_static_network::SharedNetworkBuildOptions::new(
                laneflow_static_network::SpatialBuildOption::RetainAvailable,
                laneflow_static_network::SharedNetworkBuildLimits::new(
                    64 * 1_024 * 1_024,
                    16 * 1_024 * 1_024,
                ),
            ),
        )
        .expect("revision")
    }

    struct Rig {
        session: LaneFlowSession,
        main_routes: Vec<RouteHandle>,
        alt_route: RouteHandle,
    }

    fn rig() -> Rig {
        let root = revision();
        let origin = root.canonical_origin();
        let mut world = TrafficWorld::install(
            Arc::clone(&root),
            WorldConfig::new(512, 16, 1_024, 1_024, 100),
            laneflow_runtime::ExecutionConfig::new(NonZeroU32::MIN),
            CommittedNetworkSource::Published {
                reference: PublishedLfcaReference::new(
                    "fixture://session-capacity",
                    origin.canonical_artifact_digest(),
                    origin.canonical_artifact_byte_length(),
                    origin.network_revision(),
                )
                .expect("source"),
            },
            725,
            WorldPolicySelection::NotRequired,
        )
        .expect("install");
        let main_routes: Vec<_> = (0..MAIN_LANES)
            .map(|lane| {
                world
                    .register_route(RouteRegisterInput::new(vec![edge_ordinal(
                        &root,
                        &format!("main-{lane}"),
                    )]))
                    .expect("main lane route")
            })
            .collect();
        let alt_route = world
            .register_route(RouteRegisterInput::new(vec![edge_ordinal(&root, "alt-0")]))
            .expect("alt route");
        let spatial = SpatialSession::bind(Arc::clone(&root))
            .expect("bind")
            .expect("spatial");
        Rig {
            session: LaneFlowSession::new(
                world,
                Some(spatial),
                LaneFlowSessionConfig::new(NonZeroU32::new(8).expect("non-zero")),
            )
            .expect("session"),
            main_routes,
            alt_route,
        }
    }

    /// 在 main 车道上生成槽位区间 [from, to) 的 Active，均布在各车道
    /// （间隔 6 m，可容纳每车道 ~33 辆）。
    fn spawn_slots(rig: &mut Rig, from: usize, to: usize) {
        let per_lane = (LANE_LENGTH_MM / 6_000) as usize;
        for slot in from..to {
            let lane = slot / per_lane;
            let progress = u32::try_from(slot % per_lane).expect("progress fits u32") * 6_000;
            rig.session
                .world_mut()
                .spawn_vehicle(VehicleSpawnInput::new(
                    PROFILE,
                    rig.main_routes[lane],
                    0,
                    progress,
                    0,
                ))
                .expect("spawn");
        }
    }

    fn extract(rig: &mut LaneFlowSession, output: &mut LaneFlowCommittedPoseBatch) {
        rig.extract_committed_pose_batch(laneflow_spatial::FramePlacementToken::new(1), output)
            .expect("extract");
    }

    /// 同一 Session/output：小批 → 大批 → 缩回小批。缩量通过合法生命周期
    /// （despawn Active）完成；记录各观察点的 len/capacity 实值轨迹
    /// （eprintln，--nocapture 可见），并断言缩量后内容正确、容量保留、
    /// 旧尾部不可见。
    #[test]
    fn grow_then_shrink_keeps_capacity_and_hides_tail() {
        let mut rig = rig();
        let mut output = LaneFlowCommittedPoseBatch::new();
        spawn_slots(&mut rig, 0, 4);
        extract(&mut rig.session, &mut output);
        let trace = |step: &str, out: &LaneFlowCommittedPoseBatch, sess: &LaneFlowSession| {
            eprintln!(
                "capacity-trace {step}: presentable={} out.len={} out.cap={} pose.len={} pose.cap={} pose_sz={} veh.len={} veh.cap={} veh_sz={}",
                out.vehicles().len(),
                out.vehicles().len(),
                out.vehicles.capacity(),
                sess.pose_scratch.len(),
                sess.pose_scratch.capacity(),
                std::mem::size_of::<laneflow_spatial::PoseInput>(),
                sess.pose_vehicle_scratch.len(),
                sess.pose_vehicle_scratch.capacity(),
                std::mem::size_of::<VehicleHandle>(),
            );
        };
        trace("small-1", &output, &rig.session);

        spawn_slots(&mut rig, 4, 68);
        extract(&mut rig.session, &mut output);
        assert_eq!(output.vehicles.len(), 68);
        assert_eq!(output.batch.records().len(), 68);
        trace("large", &output, &rig.session);
        let large_output_cap = output.vehicles.capacity();
        let large_pose_cap = rig.session.pose_scratch.capacity();
        assert!(large_output_cap >= 68 && large_pose_cap >= 68);

        // 合法缩量：despawn 后 64 辆（保留 live 序列前 4 辆）。
        let all: Vec<_> = output.vehicles().to_vec();
        for vehicle in &all[4..] {
            rig.session
                .world
                .despawn_vehicle(*vehicle)
                .expect("legal lifecycle despawn");
        }
        extract(&mut rig.session, &mut output);
        trace("small-2", &output, &rig.session);
        assert_eq!(
            output.vehicles(),
            all[..4].to_vec().as_slice(),
            "kept members and order"
        );
        assert_eq!(output.batch.records().len(), 4);
        for (index, record) in output.batch.records().iter().enumerate() {
            assert_eq!(
                record.record().raw(),
                index as u32,
                "ids restart at 0, old tail hidden"
            );
        }
        // 轮换感知的容量保留：成功提交交换 backing，缩量后大批 backing 可能
        // 轮换到 Session 一侧；按两侧最大值断言“未释放”，不误判为主动缩容。
        assert!(
            output.vehicles.capacity().max(rig.session.pose_vehicle_scratch.capacity())
                >= large_output_cap,
            "the grown vehicles backing must survive the shrink on one side"
        );
        assert!(
            rig.session.pose_scratch.capacity() >= large_pose_cap,
            "session candidates keep capacity after shrink"
        );
    }

    /// 两个不同容量的 output 交替：更新一方不改变另一方完整输出；暖机后
    /// 重复交替不再增长容量（backing 随所有权轮换）。
    #[test]
    fn alternating_outputs_keep_snapshots_and_rotate_capacity() {
        let mut rig = rig();
        let mut a = LaneFlowCommittedPoseBatch::new();
        let mut b = LaneFlowCommittedPoseBatch::new();
        spawn_slots(&mut rig, 0, 4);
        extract(&mut rig.session, &mut a);
        spawn_slots(&mut rig, 4, 64);
        extract(&mut rig.session, &mut b);
        assert_eq!(b.vehicles.len(), 64);
        let b_snapshot = (b.vehicles().to_vec(), b.batch().clone(), b.context());

        // 更新 A（内容为当前全部 64 辆）：B 的完整输出不变。
        extract(&mut rig.session, &mut a);
        assert_eq!(a.vehicles.len(), 64, "full extraction follows the world");
        assert_eq!(
            b.vehicles(),
            b_snapshot.0.as_slice(),
            "updating a must not modify b vehicles"
        );
        assert_eq!(
            *b.batch(),
            b_snapshot.1,
            "updating a must not modify b batch"
        );
        assert_eq!(
            b.context(),
            b_snapshot.2,
            "updating a must not modify b context"
        );

        // 再更新 B：A 的完整输出不变。
        let a_snapshot = a.vehicles().to_vec();
        extract(&mut rig.session, &mut b);
        assert_eq!(
            a.vehicles(),
            a_snapshot.as_slice(),
            "updating b must not modify a"
        );

        // 暖机后重复交替：容量稳定不再增长。
        extract(&mut rig.session, &mut a);
        let stable_a = a.vehicles.capacity();
        let stable_b = b.vehicles.capacity();
        for _ in 0..3 {
            extract(&mut rig.session, &mut a);
            extract(&mut rig.session, &mut b);
        }
        assert_eq!(
            a.vehicles.capacity(),
            stable_a,
            "steady alternation is stable"
        );
        assert_eq!(
            b.vehicles.capacity(),
            stable_b,
            "steady alternation is stable"
        );
        assert!(stable_a >= 64 && stable_b >= 64);
    }

    /// 已暖机 Session 换入全新 output：首调用把 Session 候选 backing 交换给
    /// 全新 output、接回其空 backing（容量可回落到 0），下一批重新增长——
    /// 这是 #711 冻结的轮换合同，不是泄漏。
    #[test]
    fn warm_session_with_fresh_output_builds_then_steadies() {
        let mut rig = rig();
        spawn_slots(&mut rig, 0, 16);
        let mut warm = LaneFlowCommittedPoseBatch::new();
        extract(&mut rig.session, &mut warm);
        extract(&mut rig.session, &mut warm);
        let pose_cap_after_warm = rig.session.pose_scratch.capacity();
        assert!(pose_cap_after_warm >= 16);

        let mut fresh = LaneFlowCommittedPoseBatch::new();
        assert_eq!(
            fresh.vehicles.capacity(),
            0,
            "brand-new output starts empty"
        );
        extract(&mut rig.session, &mut fresh);
        assert_eq!(fresh.vehicles.len(), 16);
        // 全新 output 接住 Session 暖 backing；Session 接回 fresh 的空 backing。
        assert!(
            fresh.vehicles.capacity() >= 16,
            "fresh output takes warm backing"
        );
        assert_eq!(
            rig.session.pose_vehicle_scratch.capacity(),
            0,
            "session takes the fresh output's previous empty backing"
        );

        // 下一批在同容量 output 上恢复稳定：候选重新增长且再次稳定。
        extract(&mut rig.session, &mut fresh);
        assert!(rig.session.pose_vehicle_scratch.capacity() >= 16);
        let stable = fresh.vehicles.capacity();
        extract(&mut rig.session, &mut fresh);
        assert_eq!(fresh.vehicles.capacity(), stable, "steady reuse is stable");
    }

    /// 有效旧输出 → 混 frame 失败 → 修正（虚拟池停入 stray）→ 重试：
    /// 失败不改变旧输出内容与 backing 容量；候选缓冲容量在失败后保留。
    #[test]
    fn failure_keeps_output_and_candidates_then_retry_succeeds() {
        let mut rig = rig();
        spawn_slots(&mut rig, 0, 4);
        let mut output = LaneFlowCommittedPoseBatch::new();
        extract(&mut rig.session, &mut output);
        // 首次成功提取之后才引入 alt frame 来源，触发后续混 frame 失败。
        let stray = rig
            .session
            .world_mut()
            .spawn_vehicle(VehicleSpawnInput::new(PROFILE, rig.alt_route, 0, 10_000, 0))
            .expect("stray");
        let before_vehicles = output.vehicles().to_vec();
        let before_capacity = output.vehicles.capacity();
        let pose_cap = rig.session.pose_scratch.capacity();
        let vehicle_cap = rig.session.pose_vehicle_scratch.capacity();

        let error = rig
            .session
            .extract_committed_pose_batch(
                laneflow_spatial::FramePlacementToken::new(2),
                &mut output,
            )
            .expect_err("mixed frame must fail");
        assert!(matches!(
            error,
            crate::LaneFlowAdapterError::SpatialPoseExtraction { .. }
        ));
        assert_eq!(output.vehicles(), before_vehicles.as_slice());
        assert_eq!(
            output.vehicles.capacity(),
            before_capacity,
            "failure keeps backing"
        );
        assert!(
            rig.session.pose_scratch.capacity() >= pose_cap
                && rig.session.pose_vehicle_scratch.capacity() >= vehicle_cap,
            "failure keeps candidate capacity for retry"
        );

        let mut world = rig.session.world_mut();
        world
            .reserve_parking(
                stray,
                ReserveParkingTarget::VirtualPool {
                    facility: FACILITY,
                    entry_anchor: VirtualEntryAnchorSelector::from_raw(0),
                    entry_route_occurrence: 0,
                },
            )
            .expect("reserve");
        world
            .park_vehicle(
                stray,
                laneflow_runtime::ParkingTarget::VirtualPool(FACILITY),
            )
            .expect("park");
        extract(&mut rig.session, &mut output);
        assert_eq!(output.vehicles.len(), 4);
        assert_eq!(output.vehicles(), before_vehicles.as_slice());
    }

    /// 元素大小记录（报告引用；断言消息承载观测值）。
    #[test]
    fn adapter_backing_element_sizes() {
        assert_eq!(
            std::mem::size_of::<laneflow_spatial::PoseInput>(),
            16,
            "PoseInput backing element size"
        );
        assert_eq!(
            std::mem::size_of::<VehicleHandle>(),
            8,
            "VehicleHandle backing element size"
        );
    }
}
