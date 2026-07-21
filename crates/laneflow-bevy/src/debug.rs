//! 可选、预算受控的 Bevy Gizmos 调试可视化。

use bevy_app::{App, Plugin, PostUpdate};
use bevy_color::palettes::basic::{BLUE, GREEN, RED, WHITE, YELLOW};
use bevy_color::palettes::css::{DEEP_SKY_BLUE, LIMEGREEN};
use bevy_ecs::{
    resource::Resource,
    schedule::IntoScheduleConfigs,
    system::{Query, Res, ResMut},
};
use bevy_gizmos::{GizmoPlugin, gizmos::Gizmos};
use bevy_math::Vec3;
use bevy_transform::{TransformSystems, components::GlobalTransform};
use laneflow_core::VehicleHandle;
use laneflow_spatial::{CanonicalFrameId, CanonicalPoint3F32, CanonicalPoseRecordF32};

use crate::LaneFlowSession;

/// 调试 Gizmos 系统最近一次运行的总体状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LaneFlowDebugGizmosStatus {
    /// Plugin 已安装，但尚未运行绘制系统。
    #[default]
    WaitingForFrame,
    /// 宿主没有在 LaneFlow debug plugin 前安装 Bevy `GizmoPlugin`。
    MissingGizmoPlugin,
    /// 宿主尚未插入显式 debug 配置。
    MissingConfig,
    /// 运行时配置关闭了调试绘制。
    Disabled,
    /// 当前 App 没有活动的 LaneFlow Session。
    MissingSession,
    /// 当前 Session 没有 frame placement。
    MissingPlacement,
    /// 当前 outer frame 没有通过完整 Adapter 校验的 presentation batch。
    NoValidatedBatch,
    /// validated batch 的 frame 或 placement token 已不再匹配当前 Session。
    FrameOrTokenMismatch,
    /// 当前 frame-root 没有可用的 `GlobalTransform`。
    MissingFrameRootGlobalTransform,
    /// 配置中的绘制尺寸不是有限正数。
    InvalidConfig,
    /// 已按当前配置完成绘制请求。
    Drawn,
}

/// 调用方提供的中心线在最近一帧中的处理状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LaneFlowDebugCenterlineStatus {
    /// 当前没有中心线输入。
    #[default]
    NotProvided,
    /// 输入 frame 与 validated batch 不匹配，因此只跳过中心线。
    FrameMismatch,
    /// 输入 frame 匹配并已按预算处理。
    Drawn,
}

/// 车辆 marker 的稳定 membership 过滤器。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LaneFlowDebugVehicleFilter {
    /// 接受 validated batch 中的所有车辆。
    #[default]
    All,
    /// 只接受当前具有 Adapter Entity 绑定的车辆。
    MappedOnly,
    /// 只接受调用方显式列出的车辆；batch 顺序仍是唯一绘制顺序。
    AllowList(Vec<VehicleHandle>),
}

impl LaneFlowDebugVehicleFilter {
    fn matches(&self, vehicle: VehicleHandle, session: &LaneFlowSession) -> bool {
        match self {
            Self::All => true,
            Self::MappedOnly => session.vehicle_entities().entity(vehicle).is_some(),
            Self::AllowList(vehicles) => vehicles.contains(&vehicle),
        }
    }
}

/// 运行时 debug 开关、绘制尺寸、预算与过滤配置。
///
/// `Default` 保持关闭；调用方必须显式插入并启用该 Resource。
#[derive(Clone, Debug, Resource, PartialEq)]
pub struct LaneFlowDebugGizmosConfig {
    /// 是否请求本帧调试绘制。
    pub enabled: bool,
    /// 是否绘制 frame-root 的 X/Y/Z 三轴。
    pub draw_frame_axes: bool,
    /// frame axes 长度，单位为 Bevy world unit。
    pub frame_axes_length: f32,
    /// 每辆车 position cross 的完整边长。
    pub position_marker_size: f32,
    /// 每辆车 forward/up ray 的长度。
    pub direction_marker_length: f32,
    /// 每帧最多绘制的车辆数；过滤后按 batch 稳定顺序截取。
    pub vehicle_budget: usize,
    /// 每帧最多绘制的调用方中心线 segment 数。
    pub centerline_segment_budget: usize,
    /// 车辆 membership 过滤器。
    pub vehicle_filter: LaneFlowDebugVehicleFilter,
}

impl Default for LaneFlowDebugGizmosConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            draw_frame_axes: true,
            frame_axes_length: 2.0,
            position_marker_size: 0.4,
            direction_marker_length: 1.5,
            vehicle_budget: 0,
            centerline_segment_budget: 0,
            vehicle_filter: LaneFlowDebugVehicleFilter::All,
        }
    }
}

impl LaneFlowDebugGizmosConfig {
    /// 创建显式启用且采用保守 marker 尺寸的预算配置。
    pub fn enabled(vehicle_budget: usize, centerline_segment_budget: usize) -> Self {
        Self {
            enabled: true,
            vehicle_budget,
            centerline_segment_budget,
            ..Self::default()
        }
    }

    fn has_valid_dimensions(&self) -> bool {
        [
            self.frame_axes_length,
            self.position_marker_size,
            self.direction_marker_length,
        ]
        .into_iter()
        .all(|value| value.is_finite() && value > 0.0)
    }
}

/// 调用方已加载的 canonical 中心线快照。
///
/// 本类型不计算长度、不重采样，也不构造 `SpatialRegistry`；它只保存绘制输入顺序。
#[derive(Clone, Debug, Resource, PartialEq)]
pub struct LaneFlowDebugCenterlines {
    frame_id: CanonicalFrameId,
    polylines: Vec<Vec<CanonicalPoint3F32>>,
}

impl LaneFlowDebugCenterlines {
    /// 创建带显式 frame identity 的调用方中心线输入。
    pub fn new(frame_id: CanonicalFrameId, polylines: Vec<Vec<CanonicalPoint3F32>>) -> Self {
        Self {
            frame_id,
            polylines,
        }
    }

    /// 返回调用方声明的 canonical frame。
    pub const fn frame_id(&self) -> &CanonicalFrameId {
        &self.frame_id
    }

    /// 返回保持调用方顺序的折线切片。
    pub fn polylines(&self) -> &[Vec<CanonicalPoint3F32>] {
        &self.polylines
    }
}

/// 最近一帧 debug 绘制的稳定摘要。
#[derive(Clone, Copy, Debug, Default, Resource, PartialEq, Eq)]
pub struct LaneFlowDebugGizmosReport {
    status: LaneFlowDebugGizmosStatus,
    centerline_status: LaneFlowDebugCenterlineStatus,
    frame_axes_drawn: bool,
    eligible_vehicle_records: usize,
    drawn_vehicle_records: usize,
    truncated_vehicle_records: usize,
    first_drawn_vehicle: Option<VehicleHandle>,
    last_drawn_vehicle: Option<VehicleHandle>,
    available_centerline_segments: usize,
    drawn_centerline_segments: usize,
    truncated_centerline_segments: usize,
    emitted_line_segments: usize,
}

impl LaneFlowDebugGizmosReport {
    /// 返回最近一次系统状态。
    pub const fn status(self) -> LaneFlowDebugGizmosStatus {
        self.status
    }

    /// 返回中心线输入的独立处理状态。
    pub const fn centerline_status(self) -> LaneFlowDebugCenterlineStatus {
        self.centerline_status
    }

    /// 返回本帧是否请求了三条 frame axis 线段。
    pub const fn frame_axes_drawn(self) -> bool {
        self.frame_axes_drawn
    }

    /// 返回过滤后的车辆记录数。
    pub const fn eligible_vehicle_records(self) -> usize {
        self.eligible_vehicle_records
    }

    /// 返回预算内绘制的车辆记录数。
    pub const fn drawn_vehicle_records(self) -> usize {
        self.drawn_vehicle_records
    }

    /// 返回因车辆预算被稳定截取的记录数。
    pub const fn truncated_vehicle_records(self) -> usize {
        self.truncated_vehicle_records
    }

    /// 返回 batch 顺序中的第一辆已绘制车辆。
    pub const fn first_drawn_vehicle(self) -> Option<VehicleHandle> {
        self.first_drawn_vehicle
    }

    /// 返回 batch 顺序中的最后一辆已绘制车辆。
    pub const fn last_drawn_vehicle(self) -> Option<VehicleHandle> {
        self.last_drawn_vehicle
    }

    /// 返回调用方输入中的中心线 segment 总数。
    pub const fn available_centerline_segments(self) -> usize {
        self.available_centerline_segments
    }

    /// 返回预算内绘制的中心线 segment 数。
    pub const fn drawn_centerline_segments(self) -> usize {
        self.drawn_centerline_segments
    }

    /// 返回因中心线预算被稳定截取的 segment 数。
    pub const fn truncated_centerline_segments(self) -> usize {
        self.truncated_centerline_segments
    }

    /// 返回本帧向 Bevy Gizmos 请求的 line segment 总数。
    pub const fn emitted_line_segments(self) -> usize {
        self.emitted_line_segments
    }
}

/// 安装 feature-gated debug 绘制系统。
///
/// 宿主必须先安装 Bevy `GizmoPlugin`（完整 `DefaultPlugins` 已包含它）。若顺序不满足，
/// 本 plugin 只留下 `MissingGizmoPlugin` report，不注册会访问缺失资源的系统。
#[derive(Clone, Copy, Debug, Default)]
pub struct LaneFlowDebugGizmosPlugin;

impl Plugin for LaneFlowDebugGizmosPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LaneFlowDebugGizmosReport>();
        if !app.is_plugin_added::<GizmoPlugin>() {
            app.world_mut()
                .resource_mut::<LaneFlowDebugGizmosReport>()
                .status = LaneFlowDebugGizmosStatus::MissingGizmoPlugin;
            return;
        }

        app.add_systems(
            PostUpdate,
            draw_lane_flow_debug_gizmos.after(TransformSystems::Propagate),
        );
    }
}

fn draw_lane_flow_debug_gizmos(
    config: Option<Res<'_, LaneFlowDebugGizmosConfig>>,
    session: Option<Res<'_, LaneFlowSession>>,
    centerlines: Option<Res<'_, LaneFlowDebugCenterlines>>,
    roots: Query<'_, '_, &GlobalTransform>,
    mut report: ResMut<'_, LaneFlowDebugGizmosReport>,
    mut gizmos: Gizmos<'_, '_>,
) {
    *report = LaneFlowDebugGizmosReport::default();

    let Some(config) = config else {
        report.status = LaneFlowDebugGizmosStatus::MissingConfig;
        return;
    };
    if !config.enabled {
        report.status = LaneFlowDebugGizmosStatus::Disabled;
        return;
    }
    if !config.has_valid_dimensions() {
        report.status = LaneFlowDebugGizmosStatus::InvalidConfig;
        return;
    }

    let Some(session) = session else {
        report.status = LaneFlowDebugGizmosStatus::MissingSession;
        return;
    };
    let Some(placement) = session.frame_placement() else {
        report.status = LaneFlowDebugGizmosStatus::MissingPlacement;
        return;
    };
    let Some(batch) = session.validated_pose_batch() else {
        report.status = LaneFlowDebugGizmosStatus::NoValidatedBatch;
        return;
    };
    if batch.frame_id() != session.spatial().frame_id()
        || batch.placement_token() != placement.token()
    {
        report.status = LaneFlowDebugGizmosStatus::FrameOrTokenMismatch;
        return;
    }

    let Ok(root) = roots.get(placement.root()) else {
        report.status = LaneFlowDebugGizmosStatus::MissingFrameRootGlobalTransform;
        return;
    };

    report.status = LaneFlowDebugGizmosStatus::Drawn;
    if let Some(centerlines) = centerlines {
        if centerlines.frame_id() != batch.frame_id() {
            report.centerline_status = LaneFlowDebugCenterlineStatus::FrameMismatch;
        } else {
            report.centerline_status = LaneFlowDebugCenterlineStatus::Drawn;
            draw_centerlines(
                &mut gizmos,
                root,
                &centerlines,
                config.centerline_segment_budget,
                &mut report,
            );
        }
    }

    if config.draw_frame_axes {
        draw_frame_axes(&mut gizmos, root, config.frame_axes_length);
        report.frame_axes_drawn = true;
        report.emitted_line_segments += 3;
    }

    for record in batch.records() {
        if !config.vehicle_filter.matches(record.vehicle(), &session) {
            continue;
        }
        report.eligible_vehicle_records += 1;
        if report.drawn_vehicle_records >= config.vehicle_budget {
            report.truncated_vehicle_records += 1;
            continue;
        }

        draw_vehicle_marker(
            &mut gizmos,
            root,
            *record,
            config.position_marker_size,
            config.direction_marker_length,
        );
        report.first_drawn_vehicle.get_or_insert(record.vehicle());
        report.last_drawn_vehicle = Some(record.vehicle());
        report.drawn_vehicle_records += 1;
        report.emitted_line_segments += 5;
    }
}

fn draw_frame_axes(gizmos: &mut Gizmos<'_, '_>, root: &GlobalTransform, length: f32) {
    let origin = root.translation();
    let affine = root.affine();
    gizmos.line(
        origin,
        origin + affine.transform_vector3(Vec3::X).normalize_or_zero() * length,
        RED,
    );
    gizmos.line(
        origin,
        origin + affine.transform_vector3(Vec3::Y).normalize_or_zero() * length,
        GREEN,
    );
    gizmos.line(
        origin,
        origin + affine.transform_vector3(Vec3::Z).normalize_or_zero() * length,
        BLUE,
    );
}

fn draw_vehicle_marker(
    gizmos: &mut Gizmos<'_, '_>,
    root: &GlobalTransform,
    record: CanonicalPoseRecordF32,
    position_marker_size: f32,
    direction_marker_length: f32,
) {
    let pose = record.pose();
    let position = canonical_point(pose.position());
    let world_position = root.transform_point(position);
    let affine = root.affine();
    let half_size = position_marker_size * 0.5;

    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
        let world_axis = affine.transform_vector3(axis).normalize_or_zero() * half_size;
        gizmos.line(
            world_position - world_axis,
            world_position + world_axis,
            WHITE,
        );
    }

    let tangent = pose.tangent();
    let forward = Vec3::new(tangent.x(), tangent.y(), tangent.z());
    gizmos.line(
        world_position,
        world_position
            + affine.transform_vector3(forward).normalize_or_zero() * direction_marker_length,
        LIMEGREEN,
    );

    let up = pose.up();
    let up = Vec3::new(up.x(), up.y(), up.z());
    gizmos.line(
        world_position,
        world_position + affine.transform_vector3(up).normalize_or_zero() * direction_marker_length,
        DEEP_SKY_BLUE,
    );
}

fn draw_centerlines(
    gizmos: &mut Gizmos<'_, '_>,
    root: &GlobalTransform,
    centerlines: &LaneFlowDebugCenterlines,
    budget: usize,
    report: &mut LaneFlowDebugGizmosReport,
) {
    for polyline in centerlines.polylines() {
        for segment in polyline.windows(2) {
            report.available_centerline_segments += 1;
            if report.drawn_centerline_segments >= budget {
                report.truncated_centerline_segments += 1;
                continue;
            }

            gizmos.line(
                root.transform_point(canonical_point(segment[0])),
                root.transform_point(canonical_point(segment[1])),
                YELLOW,
            );
            report.drawn_centerline_segments += 1;
            report.emitted_line_segments += 1;
        }
    }
}

fn canonical_point(point: CanonicalPoint3F32) -> Vec3 {
    Vec3::new(point.x(), point.y(), point.z())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_runtime_disabled_with_zero_budgets() {
        let config = LaneFlowDebugGizmosConfig::default();

        assert!(!config.enabled);
        assert_eq!(config.vehicle_budget, 0);
        assert_eq!(config.centerline_segment_budget, 0);
        assert!(config.has_valid_dimensions());
    }

    #[test]
    fn enabled_config_preserves_explicit_budgets() {
        let config = LaneFlowDebugGizmosConfig::enabled(7, 11);

        assert!(config.enabled);
        assert_eq!(config.vehicle_budget, 7);
        assert_eq!(config.centerline_segment_budget, 11);
        assert!(config.has_valid_dimensions());
    }

    #[test]
    fn non_finite_or_non_positive_dimensions_are_invalid() {
        for value in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let mut config = LaneFlowDebugGizmosConfig::enabled(1, 1);
            config.direction_marker_length = value;
            assert!(!config.has_valid_dimensions());
        }
    }
}
