//! 单活动 LaneFlow Session：TrafficWorld + 可选 Spatial session。

use std::{mem, num::NonZeroU32, sync::Arc, time::Duration};

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
}

impl LaneFlowSession {
    /// 创建 Session。若提供 Spatial，必须与 world 满足 `Arc::ptr_eq`。
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
        })
    }

    /// 已提交交通世界。
    pub const fn world(&self) -> &TrafficWorld {
        &self.world
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
    /// `output.batch().records()` 按序对齐（记录身份为序号）。
    ///
    /// 稳定容量复用（adapter-api §6）：调用方持有 `output` 跨帧复用，
    /// 稳态除 Runtime `committed_pose_sources` 自身的按值返回外零新增
    /// 分配；任一失败路径 `output` 原样保持。
    pub fn extract_committed_pose_batch(
        &mut self,
        placement_token: laneflow_spatial::FramePlacementToken,
        output: &mut LaneFlowCommittedPoseBatch,
    ) -> Result<(), LaneFlowAdapterError> {
        let sources = self.world.committed_pose_sources();
        let Some(spatial) = self.spatial.as_mut() else {
            return Err(LaneFlowAdapterError::PoseExtractionWithoutSpatial);
        };
        // 每批固定 O(1) 配对检查（与车辆数无关）；`revision()` 各克隆一次 Arc。
        if !Arc::ptr_eq(&self.world.revision(), &spatial.revision()) {
            return Err(LaneFlowAdapterError::RevisionMismatch);
        }
        self.pose_scratch.clear();
        self.pose_scratch.extend(
            sources
                .as_slice()
                .iter()
                .enumerate()
                .map(|(index, (_, source))| pose_input(PoseRecordId::new(index as u32), *source)),
        );
        spatial
            .extract_pose_batch(placement_token, &self.pose_scratch, &mut output.batch)
            .map_err(|source| LaneFlowAdapterError::SpatialPoseExtraction { source })?;
        output.vehicles.clear();
        output
            .vehicles
            .extend(sources.as_slice().iter().map(|(vehicle, _)| *vehicle));
        output.context = self.consumption_context();
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
    pub fn spawn_vehicle(&mut self, input: VehicleSpawnInput) -> Result<VehicleHandle, SpawnError> {
        self.world.spawn_vehicle(input)
    }

    /// 注册本世界路线。
    pub fn register_route(&mut self, input: RouteRegisterInput) -> Result<RouteHandle, RouteError> {
        self.world.register_route(input)
    }

    /// 移除本世界路线。
    pub fn remove_route(&mut self, route: RouteHandle) -> Result<(), RouteError> {
        self.world.remove_route(route)
    }

    /// 预留精确停车 target/payload。
    pub fn reserve_parking(
        &mut self,
        vehicle: VehicleHandle,
        target: ReserveParkingTarget,
    ) -> Result<ParkingCommandOutcome<ParkingReserveRecord>, ParkingError> {
        self.world.reserve_parking(vehicle, target)
    }

    /// 取消 exact reservation。
    pub fn cancel_parking(
        &mut self,
        vehicle: VehicleHandle,
        target: ParkingTarget,
    ) -> Result<ParkingCancelRecord, ParkingError> {
        self.world.cancel_parking(vehicle, target)
    }

    /// 提交 exact arrived reservation。
    pub fn park_vehicle(
        &mut self,
        vehicle: VehicleHandle,
        target: ParkingTarget,
    ) -> Result<ParkingCommandOutcome<ParkingParkRecord>, ParkingError> {
        self.world.park_vehicle(vehicle, target)
    }

    /// 从 parking target 安全插回 lane。
    pub fn leave_parking(
        &mut self,
        vehicle: VehicleHandle,
        target: LeaveParkingTarget,
    ) -> Result<ParkingLeaveRecord, ParkingError> {
        self.world.leave_parking(vehicle, target)
    }

    /// 在完整 footprint 相等时重绑 reservation route。
    pub fn rebind_parking_route(
        &mut self,
        vehicle: VehicleHandle,
        target: RebindParkingTarget,
    ) -> Result<ParkingCommandOutcome<ParkingRebindRecord>, ParkingError> {
        self.world.rebind_parking_route(vehicle, target)
    }

    /// 直接构造 `Parked + Occupied`，不建立 lane pose。
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
    pairs: Vec<(VehicleHandle, Entity)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PreparedVehicleEntityRemoval {
    index: usize,
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
            pairs: Vec::with_capacity(capacity),
        }
    }

    fn entity(&self, vehicle: VehicleHandle) -> Option<Entity> {
        self.pairs
            .iter()
            .find_map(|(handle, entity)| (*handle == vehicle).then_some(*entity))
    }

    fn bind(&mut self, vehicle: VehicleHandle, entity: Entity) -> Result<(), LaneFlowAdapterError> {
        if let Some((_, existing)) = self.pairs.iter().find(|(handle, _)| *handle == vehicle) {
            return Err(LaneFlowAdapterError::DuplicateVehicleBinding {
                vehicle,
                existing: *existing,
                requested: entity,
            });
        }
        if let Some((existing, _)) = self.pairs.iter().find(|(_, bound)| *bound == entity) {
            return Err(LaneFlowAdapterError::DuplicateEntityBinding {
                entity,
                existing: *existing,
                requested: vehicle,
            });
        }
        self.pairs.push((vehicle, entity));
        Ok(())
    }

    fn unbind_vehicle(&mut self, vehicle: VehicleHandle) -> Result<Entity, LaneFlowAdapterError> {
        let Some(index) = self.pairs.iter().position(|(handle, _)| *handle == vehicle) else {
            return Err(LaneFlowAdapterError::UnknownVehicle { vehicle });
        };
        Ok(self.pairs.swap_remove(index).1)
    }

    fn rotate(&mut self, old: VehicleHandle, new: VehicleHandle, entity: Option<Entity>) {
        let Some(entity) = entity else {
            return;
        };
        if let Some(pair) = self.pairs.iter_mut().find(|(handle, _)| *handle == old) {
            *pair = (new, entity);
        }
    }

    fn prepare_remove(&self, vehicle: VehicleHandle) -> Option<PreparedVehicleEntityRemoval> {
        let index = self
            .pairs
            .iter()
            .position(|(handle, _)| *handle == vehicle)?;
        Some(PreparedVehicleEntityRemoval {
            index,
            vehicle,
            entity: self.pairs[index].1,
        })
    }

    fn commit_remove(&mut self, prepared: PreparedVehicleEntityRemoval) -> Entity {
        let removed = self.pairs.swap_remove(prepared.index);
        debug_assert_eq!(removed, (prepared.vehicle, prepared.entity));
        removed.1
    }
}
