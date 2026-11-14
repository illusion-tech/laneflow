//! 无窗口源码接入：检入走廊 LFCA 与 catalog，认证共享路网修订后安装，注册一条路线、
//! 生成一辆车并固定步进。
//!
//! 本文件是唯一完整程序。Cargo 同时把它登记为集成测试 `headless_smoke`：
//! `cargo run --example headless` 打印结果，`cargo test --test headless_smoke`
//! 执行同一份 [`run`] 上的断言。教程见
//! [从源码接入无窗口交通运行时](../../../docs/guides/source-integration.md)。

use std::error::Error;
use std::num::NonZeroU32;
use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    CommittedNetworkSource, ExecutionConfig, ObservationError, ObservationExportMode,
    ObservationSelection, PublishedLfcaReference, RouteError, RouteRegisterInput, StepError,
    TickInput, TrafficWorld, VehicleSpawnInput, VehicleStatus, WorldConfig, WorldPolicySelection,
    attest_published_shared_network_revision,
};
use laneflow_scenario::signalized_corridor::{
    CATALOG_VERSION, PASSENGER_CAR_PROFILE_KEY, PORTAL_IDS, bind,
};
use laneflow_static_contract::LaneEdgeOrdinal;
use laneflow_static_network::{
    AttestationOutcome, AttestationVerdict, SharedNetworkBuildLimits, SharedNetworkBuildOptions,
    SharedNetworkRevision, SpatialBuildOption, build_shared_network_revision,
};

/// 检入场景制品名。`v0.2` 是走廊文件名，不是可移植对象版本。
const ASSET_KEY: &str = "checked-in:v0.2-signalized-corridor";
/// 检查、构建与认证共用的格式天花板，不是任意不受信路网的安全预算。
const FORMAT_LIMITS: FormatLimits = FormatLimits::HARD;
const LFCA: &[u8] = include_bytes!("../../../examples/data/v0.2-signalized-corridor.lfca");
const CATALOG: &str = include_str!("../../../examples/data/v0.2-signalized-corridor.catalog.toml");

/// 与检入走廊信号程序一致。合法范围是 `4..=1000`，不要改成别的步长再指望相位仍对齐。
const TICK_MS: u64 = 16;
/// 与 [`TICK_MS`] 故意不一致，用来观察拒绝不提交。
const REJECTED_DELTA_MS: u64 = 15;
const DRIVE_STEPS: u64 = 64;
const WORLD_ID: u64 = 1;
/// 本示例只注册一条路线。边出现记录容量等于该路线的边数。
/// 这条左转路线在注册时的冲突出现记录数是 0，因此容量也是 0。
/// 0 只描述这一条路线，不表示整网没有 Gate、冲突区或参与者流。
const CONFLICT_OCCURRENCE_CAPACITY: u64 = 0;
/// 调用方给这次检入走廊构建的保留/暂存上限，不是任意不受信路网的安全预算。
const BUILD_RETAINED_BYTES: u64 = 64 * 1_024 * 1_024;
const BUILD_SCRATCH_BYTES: u64 = 16 * 1_024 * 1_024;

type ExampleResult<T> = Result<T, Box<dyn Error>>;

/// 一次无窗口运行的可观察结果。打印文本由 [`HeadlessReport::render`] 生成。
#[derive(Clone, Debug, PartialEq, Eq)]
struct HeadlessReport {
    /// 绑定得到的策略选择。走廊必须是 `Pinned`。
    pub policy: WorldPolicySelection,
    /// 共享路网修订标识的小写十六进制。
    pub network_revision: String,
    /// 安装前修订认证的结论。只有 `Pass` 才会继续安装。
    pub attestation_verdict: AttestationVerdict,
    /// 已提交来源的修订标识是否与共享路网修订相同。
    pub source_revision_matches: bool,
    /// catalog 首个 portal。
    pub portal_id: String,
    /// 该 portal 的入口车道下标。
    pub lane_index: usize,
    /// 与该车道入口对齐的 spawn slot。
    pub slot_id: String,
    /// 该车道第一条 catalog 路线。
    pub route_id: String,
    /// 注册进世界的边出现记录数。
    pub route_edges: usize,
    /// 固定步长（毫秒）。
    pub fixed_delta_time_ms: u64,
    /// 参与计算的 worker 数。
    pub worker_count: u32,
    /// 错误步长被拒绝后的已提交时间，必须仍为 0。
    pub time_after_rejected_step_ms: u64,
    /// 空路线被拒绝后，已生成车辆是否仍在。
    pub live_vehicles_after_rejected_route: usize,
    /// 空路线被拒绝后命令游标是否保持不变。
    pub command_cursor_unchanged: bool,
    /// 成功步进次数。
    pub steps: u64,
    /// 成功步进后的已提交时间。
    pub time_ms: u64,
    /// 成功步进后的 tick。
    pub tick_index: u64,
    /// 步进后的车辆状态。
    pub status: VehicleStatus,
    /// 生成时的路线边下标。
    pub edge_before: u32,
    /// 生成时的边内进度（毫米）。
    pub progress_before_mm: u32,
    /// 步进后的路线边下标。
    pub edge_after: u32,
    /// 步进后的边内进度（毫米）。
    pub progress_after_mm: u32,
    /// 步进后的速度（毫米每秒）。
    pub speed_mm_s: u32,
    /// 车辆沿路线前进，或已经走完。
    pub advanced: bool,
    /// 首次 Delta 被拒绝后，session 仍没有基线。
    pub rejected_delta_left_session_empty: bool,
    /// Full 批次的模式。
    pub observation_mode: ObservationExportMode,
    /// Full 批次没有 delta 基线。
    pub observation_base_is_none: bool,
    /// Full 批次的路网修订标识。
    pub observation_revision: String,
    /// Full 批次读取的已提交 tick。
    pub observation_tick: u64,
    /// Full 批次的世界身份是否等于安装时的世界。
    pub observation_world_matches: bool,
    /// Full 行数。
    pub observation_entries: u64,
    /// 所选边占用长度之和（毫米）。
    pub occupied_length_mm: u64,
    /// 前保险杠计数之和。
    pub front_vehicle_count: u32,
}

impl HeadlessReport {
    /// 稳定的键值文本，供示例标准输出和指南冻结。
    #[must_use]
    fn render(&self) -> String {
        let policy = match self.policy {
            WorldPolicySelection::Pinned(_) => "pinned",
            WorldPolicySelection::NotRequired => "not_required",
        };
        let status = match self.status {
            VehicleStatus::Active => "active",
            VehicleStatus::Parked => "parked",
            VehicleStatus::Completed => "completed",
        };
        let observation = match self.observation_mode {
            ObservationExportMode::Full => "full",
            ObservationExportMode::Delta => "delta",
        };
        let observation_base = if self.observation_base_is_none {
            "none"
        } else {
            "present"
        };
        let attestation = match self.attestation_verdict {
            AttestationVerdict::Pass => "pass",
            AttestationVerdict::Mismatch => "mismatch",
            AttestationVerdict::Untrusted => "untrusted",
        };
        let session_after_rejected_delta = if self.rejected_delta_left_session_empty {
            "empty"
        } else {
            "moved"
        };
        format!(
            "\
asset={ASSET_KEY}
catalog_version={CATALOG_VERSION}
spatial=omitted
policy={policy}
network_revision={revision}
attestation={attestation}
source_revision_matches={source_matches}
portal={portal}
lane_index={lane}
slot={slot}
route={route}
route_edges={route_edges}
fixed_delta_time_ms={tick_ms}
worker_count={workers}
rejected_step=DeltaMismatch
time_after_rejected_step_ms={rejected_time}
rejected_route=EmptySequence
live_vehicles={live_vehicles}
command_cursor_unchanged={cursor_unchanged}
steps={steps}
time_ms={time_ms}
status={status}
edge_before={edge_before}
progress_before_mm={progress_before}
edge_after={edge_after}
progress_after_mm={progress_after}
speed_mm_s={speed}
advanced={advanced}
rejected_observation=FirstExportMustBeFull
session_after_rejected_delta={session_after_rejected_delta}
observation={observation}
observation_base={observation_base}
observation_revision={observation_revision}
observation_tick={observation_tick}
observation_world_matches={world_matches}
observation_entries={entries}
occupied_length_mm={occupied}
front_vehicle_count={fronts}
",
            revision = self.network_revision,
            source_matches = self.source_revision_matches,
            portal = self.portal_id,
            lane = self.lane_index,
            slot = self.slot_id,
            route = self.route_id,
            route_edges = self.route_edges,
            tick_ms = self.fixed_delta_time_ms,
            workers = self.worker_count,
            rejected_time = self.time_after_rejected_step_ms,
            live_vehicles = self.live_vehicles_after_rejected_route,
            cursor_unchanged = self.command_cursor_unchanged,
            steps = self.steps,
            time_ms = self.time_ms,
            edge_before = self.edge_before,
            progress_before = self.progress_before_mm,
            edge_after = self.edge_after,
            progress_after = self.progress_after_mm,
            speed = self.speed_mm_s,
            advanced = self.advanced,
            observation_revision = self.observation_revision,
            observation_tick = self.observation_tick,
            world_matches = self.observation_world_matches,
            entries = self.observation_entries,
            occupied = self.occupied_length_mm,
            fronts = self.front_vehicle_count,
        )
    }
}

/// 检查检入 LFCA、省略 Spatial 保留数据、认证修订，并安装一辆车的世界。
fn run() -> ExampleResult<HeadlessReport> {
    let catalog: laneflow_scenario::signalized_corridor::CorridorCatalog = toml::from_str(CATALOG)?;
    if catalog.catalog_version != CATALOG_VERSION {
        return Err(format!(
            "catalog 版本应为 {CATALOG_VERSION}，实际 {}",
            catalog.catalog_version
        )
        .into());
    }
    let (revision, reference) = admit_and_build(LFCA)?;
    let bound = bind(&catalog, &revision)?;
    let WorldPolicySelection::Pinned(_) = bound.policy_selection else {
        return Err("走廊含 Gate / ConflictZone / ParticipantStream，不能使用 NotRequired".into());
    };
    let choice = entry_choice(&catalog, &bound)?;
    let (attestation_verdict, mut world) = attest_and_install(
        &revision,
        reference,
        LFCA,
        bound.policy_selection,
        u64::try_from(choice.edges.len())?,
    )?;
    let source_revision_matches =
        world.committed_source().network_revision() == revision.network_revision();
    if !source_revision_matches {
        return Err("已提交来源的 NetworkRevisionId 与共享路网修订不一致".into());
    }

    let rejected_step = world.step(TickInput::new(REJECTED_DELTA_MS));
    if !matches!(
        rejected_step,
        Err(StepError::DeltaMismatch {
            expected_delta_time_ms: TICK_MS,
            actual_delta_time_ms: REJECTED_DELTA_MS,
        })
    ) {
        return Err(format!("错误步长应返回 DeltaMismatch，实际 {rejected_step:?}").into());
    }
    let time_after_rejected_step_ms = world.time_ms();
    if time_after_rejected_step_ms != 0 || world.tick_index() != 0 {
        return Err("错误步长提交了时间".into());
    }

    let route = world.register_route(RouteRegisterInput::new(choice.edges.clone()))?;
    let profile = *bound
        .profiles
        .get(PASSENGER_CAR_PROFILE_KEY)
        .ok_or("bind 没有乘用车 profile")?;
    let profile_view = revision
        .traffic()
        .relations()
        .vehicle_profile(profile)
        .ok_or("共享路网修订没有乘用车 profile")?;
    let speed_limit = *revision
        .traffic()
        .lane_speed_limits_millimetres_per_second()
        .get(choice.edge.index())
        .ok_or("入口边没有限速")?;
    let vehicle = world.spawn_vehicle(
        VehicleSpawnInput::new(
            profile,
            route,
            choice.route_edge_index,
            choice.progress_mm,
            profile_view.desired_speed_mm_s().min(speed_limit),
        )
        .with_open_entrance(),
    )?;

    let cursor = world.command_cursor();
    let rejected_route =
        world.register_route(RouteRegisterInput::new(Vec::<LaneEdgeOrdinal>::new()));
    if !matches!(rejected_route, Err(RouteError::EmptySequence)) {
        return Err(format!("空路线应被拒绝，实际 {rejected_route:?}").into());
    }
    let command_cursor_unchanged = world.command_cursor() == cursor;
    let live_vehicles_after_rejected_route = world.live_vehicles().len();

    let before = world.vehicle(vehicle).ok_or("生成后的车辆句柄不可读")?;
    let step = TickInput::new(TICK_MS);
    for _ in 0..DRIVE_STEPS {
        world.step(step)?;
    }
    let after = world.vehicle(vehicle).ok_or("步进后的车辆句柄不可读")?;
    let advanced = after.status() == VehicleStatus::Completed
        || after.route_edge_index() > before.route_edge_index()
        || (after.route_edge_index() == before.route_edge_index()
            && after.progress_mm() > before.progress_mm());

    let mut session = world.open_observation_export(ObservationSelection::AllLaneEdges)?;
    let rejected_delta = world.export_observation(&mut session, ObservationExportMode::Delta);
    if !matches!(rejected_delta, Err(ObservationError::FirstExportMustBeFull)) {
        return Err(format!("首次 Delta 应被拒绝，实际 {rejected_delta:?}").into());
    }
    let rejected_delta_left_session_empty = session.previous().is_none();
    let batch = world.export_observation(&mut session, ObservationExportMode::Full)?;
    let observation_world_matches = batch.stream_binding().world_id() == world.world_id()
        && batch.stream_binding().world_generation() == world.world_generation();
    let occupied_length_mm = batch
        .rows()
        .iter()
        .try_fold(0_u64, |total, row| {
            total.checked_add(row.occupied_length_mm())
        })
        .ok_or("占用长度求和溢出")?;
    let front_vehicle_count = batch
        .rows()
        .iter()
        .try_fold(0_u32, |total, row| {
            total.checked_add(row.front_vehicle_count())
        })
        .ok_or("前保险杠计数求和溢出")?;

    Ok(HeadlessReport {
        policy: bound.policy_selection,
        network_revision: format!("{:x}", revision.network_revision().as_digest()),
        attestation_verdict,
        source_revision_matches,
        portal_id: choice.portal_id,
        lane_index: choice.lane_index,
        slot_id: choice.slot_id,
        route_id: choice.route_id,
        route_edges: choice.edges.len(),
        fixed_delta_time_ms: world.config().fixed_delta_time_ms(),
        worker_count: world.execution_config().worker_count().get(),
        time_after_rejected_step_ms,
        live_vehicles_after_rejected_route,
        command_cursor_unchanged,
        steps: DRIVE_STEPS,
        time_ms: world.time_ms(),
        tick_index: world.tick_index(),
        status: after.status(),
        edge_before: before.route_edge_index(),
        progress_before_mm: before.progress_mm(),
        edge_after: after.route_edge_index(),
        progress_after_mm: after.progress_mm(),
        speed_mm_s: after.speed_mm_s(),
        advanced,
        rejected_delta_left_session_empty,
        observation_mode: batch.mode(),
        observation_base_is_none: batch.base().is_none(),
        observation_revision: format!("{:x}", batch.network_revision().as_digest()),
        observation_tick: batch.tick(),
        observation_world_matches,
        observation_entries: batch.entry_count(),
        occupied_length_mm,
        front_vehicle_count,
    })
}

struct EntryChoice {
    portal_id: String,
    lane_index: usize,
    slot_id: String,
    route_id: String,
    edges: Vec<LaneEdgeOrdinal>,
    route_edge_index: u32,
    progress_mm: u32,
    edge: LaneEdgeOrdinal,
}

fn debug_error(error: impl core::fmt::Debug) -> Box<dyn Error> {
    format!("{error:?}").into()
}

fn build_options() -> SharedNetworkBuildOptions<'static> {
    SharedNetworkBuildOptions::new(
        SpatialBuildOption::Omit,
        SharedNetworkBuildLimits::new(BUILD_RETAINED_BYTES, BUILD_SCRATCH_BYTES),
    )
}

/// 宿主准入阶段：检查 LFCA，在受检输入上记录发布来源引用，再构建共享路网修订。
///
/// 引用取自受检输入（reader 对本次字节重算的 digest、精确长度与修订标识），
/// 不取自之后构建出的修订的 `origin`。真实宿主保存自己发布或准入时记录的引用。
fn admit_and_build(
    lfca: &'static [u8],
) -> ExampleResult<(Arc<SharedNetworkRevision>, PublishedLfcaReference)> {
    let input = check_canonical_network_input(lfca, FORMAT_LIMITS).map_err(debug_error)?;
    let reference = PublishedLfcaReference::new(
        ASSET_KEY,
        input.canonical_artifact_digest(),
        input.canonical_artifact_byte_length(),
        input.network_revision(),
    )?;
    let revision = build_shared_network_revision(input, build_options()).map_err(debug_error)?;
    if revision.spatial().is_some() {
        return Err("SpatialBuildOption::Omit 仍保留了 Spatial component".into());
    }
    Ok((revision, reference))
}

/// 安装前认证修订，只有 `Pass` 才安装；其他结论与操作失败都停止本次接入。
///
/// 认证使用与检查、构建相同的格式限额、Spatial 选项和构建预算。安装时发布来源
/// 只按 `NetworkRevisionId` 拒绝错修订；digest / 精确长度只作来源审计。
fn attest_and_install(
    revision: &Arc<SharedNetworkRevision>,
    reference: PublishedLfcaReference,
    lfca: &[u8],
    policy: WorldPolicySelection,
    edge_occurrences: u64,
) -> ExampleResult<(AttestationVerdict, TrafficWorld)> {
    let outcome = attest_published_shared_network_revision(
        revision,
        &reference,
        lfca,
        FORMAT_LIMITS,
        build_options(),
        None,
    );
    let verdict = match outcome {
        AttestationOutcome::Concluded(report) if report.verdict == AttestationVerdict::Pass => {
            report.verdict
        }
        AttestationOutcome::Concluded(report) => {
            return Err(format!(
                "修订认证未通过，停止安装：verdict={:?} cause={:?}",
                report.verdict, report.cause
            )
            .into());
        }
        AttestationOutcome::OperationFailed(failure) => {
            return Err(format!("修订认证操作失败，停止安装：{failure:?}").into());
        }
    };
    let world = TrafficWorld::install(
        Arc::clone(revision),
        WorldConfig::new(
            1,
            1,
            edge_occurrences,
            CONFLICT_OCCURRENCE_CAPACITY,
            TICK_MS,
        ),
        ExecutionConfig::new(NonZeroU32::MIN),
        CommittedNetworkSource::Published { reference },
        WORLD_ID,
        policy,
    )?;
    Ok((verdict, world))
}

fn entry_choice(
    catalog: &laneflow_scenario::signalized_corridor::CorridorCatalog,
    bound: &laneflow_scenario::signalized_corridor::BoundCorridorCatalog,
) -> ExampleResult<EntryChoice> {
    let portal = catalog.portals.first().ok_or("catalog 没有 portal")?;
    if portal.id != PORTAL_IDS[0] {
        return Err(format!(
            "catalog 首个 portal 应为 {}，实际 {}",
            PORTAL_IDS[0], portal.id
        )
        .into());
    }
    let lane = portal.lanes.first().ok_or("portal 没有车道")?;
    if lane.lane_index != 0 {
        return Err(format!("入口车道下标应为 0，实际 {}", lane.lane_index).into());
    }
    let route_choice = lane.route_choices.first().ok_or("入口车道没有路线选项")?;
    let edges = bound
        .routes
        .get(&route_choice.route_id)
        .ok_or_else(|| format!("bind 没有路线 {}", route_choice.route_id))?
        .to_vec();
    let slot = bound
        .spawn_slots
        .iter()
        .find(|slot| slot.slot_id == lane.entry_spawn_slot_id)
        .ok_or_else(|| format!("bind 没有入口槽 {}", lane.entry_spawn_slot_id))?;
    let route_edge_index = edges
        .iter()
        .position(|edge| *edge == slot.edge)
        .ok_or_else(|| {
            format!(
                "入口槽 {} 不在路线 {} 上",
                slot.slot_id, route_choice.route_id
            )
        })?;
    if route_edge_index != 0 {
        return Err(format!(
            "入口槽 {} 所在边不是路线 {} 的起点出现记录",
            slot.slot_id, route_choice.route_id
        )
        .into());
    }
    let route_edge_index = u32::try_from(route_edge_index)?;
    Ok(EntryChoice {
        portal_id: portal.id.clone(),
        lane_index: lane.lane_index,
        slot_id: slot.slot_id.clone(),
        route_id: route_choice.route_id.clone(),
        edges,
        route_edge_index,
        progress_mm: slot.progress_mm,
        edge: slot.edge,
    })
}

#[cfg(not(test))]
fn main() -> ExampleResult<()> {
    print!("{}", run()?.render());
    Ok(())
}

#[cfg(test)]
mod behavior {
    use super::{LFCA, admit_and_build, attest_and_install, run};
    use laneflow_runtime::{ObservationExportMode, VehicleStatus, WorldPolicySelection};
    use laneflow_static_network::AttestationVerdict;

    #[test]
    fn checked_corridor_advances_one_vehicle_and_bad_delta_does_not_commit() {
        let report = run().expect("headless example");
        let text = report.render();

        assert!(
            matches!(report.policy, WorldPolicySelection::Pinned(_)),
            "corridor catalog must pin a policy"
        );
        assert_eq!(report.attestation_verdict, AttestationVerdict::Pass);
        assert!(report.source_revision_matches, "published source revision");
        assert_eq!(
            report.network_revision,
            "2de4820d35bee0ef71374cae49036fbef75155a2a587cbd13b351d5f7655ad27"
        );
        assert_eq!(report.portal_id, "portal-main-west");
        assert_eq!(report.lane_index, 0);
        assert_eq!(report.slot_id, "slot-main-west-lane-0-000");
        assert_eq!(report.route_id, "route-main-west-near-left");
        assert_eq!(report.fixed_delta_time_ms, 16);
        assert_eq!(report.worker_count, 1);
        assert_eq!(report.route_edges, 3);
        assert_eq!(report.live_vehicles_after_rejected_route, 1);
        assert!(
            report.command_cursor_unchanged,
            "rejected route registration must not consume the command cursor"
        );
        assert_eq!(report.time_after_rejected_step_ms, 0);
        assert_eq!(report.steps, 64);
        assert_eq!(report.time_ms, 1_024);
        assert_eq!(report.tick_index, 64);
        assert!(report.advanced, "vehicle did not move");
        assert_eq!(report.status, VehicleStatus::Active);
        assert_eq!(report.edge_before, 0);
        assert_eq!(report.edge_after, 0);
        assert_eq!(report.progress_before_mm, 6_500);
        assert_eq!(report.progress_after_mm, 23_567);
        assert_eq!(report.speed_mm_s, 16_667);
        assert!(report.rejected_delta_left_session_empty);
        assert_eq!(report.observation_mode, ObservationExportMode::Full);
        assert!(report.observation_base_is_none);
        assert_eq!(report.observation_revision, report.network_revision);
        assert_eq!(report.observation_tick, report.tick_index);
        assert!(report.observation_world_matches);
        assert_eq!(report.observation_entries, 66);
        assert_eq!(report.occupied_length_mm, 4_500);
        assert_eq!(report.front_vehicle_count, 1);
        assert!(text.contains(&format!("time_ms={}", report.time_ms)));
        assert!(text.contains(&format!("network_revision={}", report.network_revision)));
        assert!(text.contains("attestation=pass"));
        assert!(text.contains("rejected_step=DeltaMismatch"));
        assert!(text.contains("rejected_observation=FirstExportMustBeFull"));
    }

    #[test]
    fn failed_attestation_stops_before_install() {
        let (revision, reference) = admit_and_build(LFCA).expect("admit checked corridor");
        // 交给认证的来源字节被截断：来源准备阶段失败，不得继续安装。
        let truncated = &LFCA[..LFCA.len() - 1];
        let error = attest_and_install(
            &revision,
            reference,
            truncated,
            WorldPolicySelection::NotRequired,
            0,
        )
        .err()
        .expect("non-Pass attestation must stop the integration");
        let message = error.to_string();
        assert!(
            message.contains("停止安装") && message.contains("cause=Some(Source)"),
            "rejection must come from attestation, got {message}"
        );
    }
}
