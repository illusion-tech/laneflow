//! 复杂路口拓扑：单四岔信号路口，主路东西 2+2 车道、次路南北 1+1 车道，
//! 含主路保护左转待转区、次路许可左转、冲突区与外围绕行环路。
//!
//! 几何只用 IEEE 精确运算（+−×÷、cubic bezier 求值、cross 积求交），
//! 保证检入制品跨平台逐字节可复现。

use std::collections::{BTreeMap, BTreeSet};

use laneflow_compiler::{ManeuverDirection, SignalAspect};

use crate::Error;
use crate::config::JunctionConfig;

pub const JUNCTION_KEY: &str = "j0";
pub const PARTICIPANT_CLASS_KEY: &str = "motorVehicle";
pub const DOCUMENT_KEY: &str = "complex-junction.document";
pub const GENERATOR_BUILD_ID: &str = "laneflow-junction-generator";
pub(crate) const COMPILER_BUILD_ID: &str = "laneflow-junction-generator-v1";
pub const PROVENANCE: &str = "repository:tools/laneflow-junction-generator";

pub const GROUP_MAIN_THROUGH_RIGHT: &str = "main-through-right";
pub const GROUP_MAIN_LEFT: &str = "main-left";
pub const GROUP_SECONDARY_THROUGH_RIGHT: &str = "secondary-through-right";
pub const GROUP_SECONDARY_LEFT: &str = "secondary-left";
/// 信号组编制顺序；phase state 表按此下标。
pub const SIGNAL_GROUPS: [&str; 4] = [
    GROUP_MAIN_THROUGH_RIGHT,
    GROUP_MAIN_LEFT,
    GROUP_SECONDARY_THROUGH_RIGHT,
    GROUP_SECONDARY_LEFT,
];
pub const CONTROLLER_KEY: &str = "j0-controller";

pub type Point = [f64; 2];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Arm {
    West,
    East,
    North,
    South,
}

impl Arm {
    pub const ALL: [Self; 4] = [Self::West, Self::East, Self::North, Self::South];

    /// 从路口中心指向本臂的单位向量。
    const fn delta(self) -> Point {
        match self {
            Self::West => [-1.0, 0.0],
            Self::East => [1.0, 0.0],
            Self::North => [0.0, -1.0],
            Self::South => [0.0, 1.0],
        }
    }

    pub const fn key(self) -> &'static str {
        match self {
            Self::West => "w",
            Self::East => "e",
            Self::North => "n",
            Self::South => "s",
        }
    }

    const fn main(self) -> bool {
        matches!(self, Self::West | Self::East)
    }

    /// 右侧通行车道中心到臂轴线的横向偏移；主路两车道、次路一车道。
    fn lane_offsets(self, config: &JunctionConfig) -> Vec<f64> {
        let center = config.geometry.center_offset_meters;
        let half = config.geometry.lane_width_meters / 2.0;
        if self.main() {
            vec![center - half, center + half]
        } else {
            vec![center]
        }
    }
}

/// 右侧通行端口：entering 为驶入路口的车道端口，否则为驶出端口。
fn port(arm: Arm, entering: bool, lane_offset: f64, radius: f64) -> Point {
    let [dx, dz] = arm.delta();
    let sign = if entering { -1.0 } else { 1.0 };
    [
        dx * radius - dz * sign * lane_offset,
        dz * radius + dx * sign * lane_offset,
    ]
}

fn road_edge_key(arm: Arm, entering: bool, lane: usize, lane_count: usize) -> String {
    let direction = if entering { "in" } else { "out" };
    if lane_count == 1 {
        format!("{}-{direction}", arm.key())
    } else {
        format!("{}-{direction}-i{lane}", arm.key())
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Segment {
    Line { end: Point },
    Bezier { c1: Point, c2: Point, end: Point },
}

#[derive(Clone, Debug)]
pub struct Curve {
    pub start: Point,
    pub segments: Vec<Segment>,
}

/// 90 度圆弧的标准三次 bezier 逼近系数（4/3·tan(22.5°)）；硬编码常量，
/// 不调用三角函数，保证跨平台字节一致。
const CIRCLE_CUBIC_FACTOR: f64 = 0.552_284_749_830_793_6;

impl Curve {
    pub fn line(start: Point, end: Point) -> Self {
        Self {
            start,
            segments: vec![Segment::Line { end }],
        }
    }

    pub fn bezier(start: Point, c1: Point, c2: Point, end: Point) -> Self {
        Self {
            start,
            segments: vec![Segment::Bezier { c1, c2, end }],
        }
    }

    /// 环路回连曲线：三段 90 度圆弧绕角 + 直线段。起止切向分别对齐
    /// `from` 臂驶离方向与到达臂驶入方向，起止点与道路边端口逐位一致，
    /// 保证 LaneEdge 后继焊接。控制腿等长（`CIRCLE_CUBIC_FACTOR·radius`），
    /// 编译器自适应细分在良态控制多边形上不会产生退化弦。
    ///
    /// `widen > 0` 时把折返矩形外扩，用于同角第二条环路（避免两条环路
    /// 几何重叠）；此时末尾多出一段长为 widen 的直线。
    pub fn corner_loop(start: Point, from: Arm, end: Point, radius: f64, widen: f64) -> Self {
        let h0 = from.delta();
        // xz 平面（z 指南）里行进方向右侧的单位向量。
        let right = [-h0[1], h0[0]];
        let along = (start[0] - end[0]) * h0[0] + (start[1] - end[1]) * h0[1];
        let across = (start[0] - end[0]) * right[0] + (start[1] - end[1]) * right[1];
        let l1 = -(across + radius) + widen;
        let l2 = along - radius;
        assert!(l1 > 0.0 && l2 > 0.0, "loop straights must be positive");
        let k = CIRCLE_CUBIC_FACTOR * radius;
        let mut segments = Vec::with_capacity(6);
        // 圆弧 1：h0 右转 90 度到 right。
        let end1 = [
            start[0] + radius * right[0] + radius * h0[0],
            start[1] + radius * right[1] + radius * h0[1],
        ];
        segments.push(Segment::Bezier {
            c1: [start[0] + k * h0[0], start[1] + k * h0[1]],
            c2: [end1[0] - k * right[0], end1[1] - k * right[1]],
            end: end1,
        });
        // 直线 1：沿 right 推进 l1。
        let p2 = [end1[0] + l1 * right[0], end1[1] + l1 * right[1]];
        segments.push(Segment::Line { end: p2 });
        // 圆弧 2：right 右转 90 度到 -h0。
        let end2 = [
            p2[0] - radius * h0[0] + radius * right[0],
            p2[1] - radius * h0[1] + radius * right[1],
        ];
        segments.push(Segment::Bezier {
            c1: [p2[0] + k * right[0], p2[1] + k * right[1]],
            c2: [end2[0] + k * h0[0], end2[1] + k * h0[1]],
            end: end2,
        });
        // 直线 2：沿 -h0 推进 l2。
        let p4 = [end2[0] - l2 * h0[0], end2[1] - l2 * h0[1]];
        segments.push(Segment::Line { end: p4 });
        // 圆弧 3：-h0 右转 90 度到 -right（即到达臂驶入方向）。
        let end3 = [
            p4[0] - radius * right[0] - radius * h0[0],
            p4[1] - radius * right[1] - radius * h0[1],
        ];
        segments.push(Segment::Bezier {
            c1: [p4[0] - k * h0[0], p4[1] - k * h0[1]],
            c2: [end3[0] + k * right[0], end3[1] + k * right[1]],
            end: end3,
        });
        if widen > 0.0 {
            segments.push(Segment::Line { end });
        }
        Self { start, segments }
    }

    /// 把曲线采样为折线点列（含起点）；bezier 段用 `divisions` 等分参数求值。
    fn sample_into(&self, divisions: usize, points: &mut Vec<Point>) {
        points.push(self.start);
        let mut start = self.start;
        for segment in &self.segments {
            match *segment {
                Segment::Line { end } => {
                    points.push(end);
                    start = end;
                }
                Segment::Bezier { c1, c2, end } => {
                    for i in 1..=divisions {
                        let t = i as f64 / divisions as f64;
                        let u = 1.0 - t;
                        points.push([
                            u * u * u * start[0]
                                + 3.0 * u * u * t * c1[0]
                                + 3.0 * u * t * t * c2[0]
                                + t * t * t * end[0],
                            u * u * u * start[1]
                                + 3.0 * u * u * t * c1[1]
                                + 3.0 * u * t * t * c2[1]
                                + t * t * t * end[1],
                        ]);
                    }
                    start = end;
                }
            }
        }
    }

    /// 冲突几何采样：32 等分，与 urban-generator 模板同纪律。
    pub fn conflict_samples(&self) -> Vec<Point> {
        let mut points = Vec::new();
        self.sample_into(32, &mut points);
        points
    }

    /// 折线弦长下界（bezier 段 64 等分），只用于 spawn slot 放置的保守长度估计。
    pub fn chord_length(&self) -> f64 {
        let mut total = 0.0;
        let mut start = self.start;
        for segment in &self.segments {
            match *segment {
                Segment::Line { end } => {
                    total += segment_length(start, end);
                    start = end;
                }
                Segment::Bezier { c1, c2, end } => {
                    let curve = Curve::bezier(start, c1, c2, end);
                    let mut points = Vec::new();
                    curve.sample_into(64, &mut points);
                    total += points
                        .windows(2)
                        .map(|pair| segment_length(pair[0], pair[1]))
                        .sum::<f64>();
                    start = end;
                }
            }
        }
        total
    }
}

fn segment_length(a: Point, b: Point) -> f64 {
    let dx = b[0] - a[0];
    let dz = b[1] - a[1];
    (dx * dx + dz * dz).sqrt()
}

#[derive(Clone, Debug)]
pub struct EdgeBuild {
    pub key: String,
    pub curve: Curve,
    pub speed: f64,
    pub successors: Vec<String>,
    /// 带独立几何的边必须由 RoadSection 派生（编制 corridor/section/lane 链）：
    /// junction approach 边是编译器硬性要求，环路回连边则需要 alignment 提供
    /// canonical frame。只有 junction internal 边保持裸 LaneEdge + 内联几何
    /// （frame 从路径出入口推导，且不允许携带后继）。
    pub section_derived: bool,
}

#[derive(Clone, Debug)]
pub struct MovementBuild {
    pub key: String,
    pub entry: Arm,
    pub exit: Arm,
    pub turn: ManeuverDirection,
    pub waiting: bool,
    pub permissive: bool,
}

#[derive(Clone, Debug)]
pub struct PathBuild {
    pub movement: usize,
    pub key: String,
    pub edges: Vec<String>,
    pub samples: Vec<Point>,
    pub permissive: bool,
    pub turn: ManeuverDirection,
    pub group: &'static str,
}

#[derive(Clone, Debug)]
pub struct GateBuild {
    pub key: &'static str,
    pub path: usize,
    pub transition: u32,
    pub stop_key: String,
    pub group: &'static str,
}

#[derive(Clone, Debug)]
pub struct StopLineBuild {
    pub key: String,
    pub edge: String,
}

#[derive(Clone, Debug)]
pub struct ZoneBuild {
    pub key: String,
    pub center: Point,
    pub paths: [usize; 2],
}

#[derive(Clone, Debug)]
pub struct StreamBuild {
    pub key: String,
    pub path: usize,
    pub zones: Vec<usize>,
    pub priority: i32,
    pub yield_to: Vec<String>,
    pub gap: bool,
}

#[derive(Clone, Debug)]
pub struct PhaseBuild {
    pub key: String,
    pub duration_ms: u64,
    pub aspects: [SignalAspect; 4],
}

#[derive(Clone, Debug)]
pub struct RouteBuild {
    pub id: &'static str,
    pub exit_portal: &'static str,
    pub edges: Vec<String>,
    pub focus: bool,
}

#[derive(Clone, Debug)]
pub struct PortalLaneBuild {
    pub edge: String,
    pub choices: Vec<(&'static str, u64)>,
}

#[derive(Clone, Debug)]
pub struct PortalBuild {
    pub id: &'static str,
    pub lanes: Vec<PortalLaneBuild>,
}

#[derive(Default)]
pub struct TopologyBuild {
    pub edges: Vec<EdgeBuild>,
    pub movements: Vec<MovementBuild>,
    pub paths: Vec<PathBuild>,
    pub gates: Vec<GateBuild>,
    pub stop_lines: Vec<StopLineBuild>,
    pub waiting_zone_path: usize,
    pub junction_approaches: Vec<String>,
    pub junction_internals: Vec<String>,
    pub zones: Vec<ZoneBuild>,
    pub streams: Vec<StreamBuild>,
    pub phases: Vec<PhaseBuild>,
    pub routes: Vec<RouteBuild>,
    pub portals: Vec<PortalBuild>,
}

impl TopologyBuild {
    pub fn edge(&self, key: &str) -> Option<&EdgeBuild> {
        self.edges.iter().find(|edge| edge.key == key)
    }

    pub fn edge_length(&self, key: &str) -> f64 {
        self.edge(key).expect("topology edge").curve.chord_length()
    }
}

/// 一对路径的冲突几何求交；同入口边由上游车道占用负责，同出口边取许可路径末点（合流）。
fn crossing(a: &[Point], b: &[Point]) -> Option<Point> {
    for x in a.windows(2) {
        for y in b.windows(2) {
            let r = [x[1][0] - x[0][0], x[1][1] - x[0][1]];
            let s = [y[1][0] - y[0][0], y[1][1] - y[0][1]];
            let q = [y[0][0] - x[0][0], y[0][1] - x[0][1]];
            let cross = |u: Point, v: Point| u[0] * v[1] - u[1] * v[0];
            let denominator = cross(r, s);
            if denominator.abs() < 1e-9 {
                continue;
            }
            let t = cross(q, s) / denominator;
            let u = cross(q, r) / denominator;
            if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
                return Some([x[0][0] + t * r[0], x[0][1] + t * r[1]]);
            }
        }
    }
    None
}

pub fn build_topology(config: &JunctionConfig) -> Result<TopologyBuild, Error> {
    let geometry = &config.geometry;
    let radius = geometry.junction_radius_meters;
    let arm_length = geometry.arm_length_meters;
    let mut topology = TopologyBuild::default();
    let mut edge_index: BTreeMap<String, usize> = BTreeMap::new();

    let add_edge = |topology: &mut TopologyBuild,
                    key: String,
                    curve: Curve,
                    speed: f64,
                    successors: Vec<String>,
                    section_derived: bool,
                    edge_index: &mut BTreeMap<String, usize>| {
        edge_index.insert(key.clone(), topology.edges.len());
        topology.edges.push(EdgeBuild {
            key,
            curve,
            speed,
            successors,
            section_derived,
        });
    };

    // 四个臂的驶入/驶出道路边；后继在路径链与环路登记时回写。
    for arm in Arm::ALL {
        let offsets = arm.lane_offsets(config);
        let speed = if arm.main() {
            config.speeds.main_meters_per_second
        } else {
            config.speeds.secondary_meters_per_second
        };
        for (lane, offset) in offsets.iter().enumerate() {
            let entry_key = road_edge_key(arm, true, lane, offsets.len());
            let inner = port(arm, true, *offset, radius);
            let outer = port(arm, true, *offset, arm_length);
            add_edge(
                &mut topology,
                entry_key,
                Curve::line(outer, inner),
                speed,
                Vec::new(),
                true,
                &mut edge_index,
            );
            let exit_key = road_edge_key(arm, false, lane, offsets.len());
            let inner = port(arm, false, *offset, radius);
            let outer = port(arm, false, *offset, arm_length);
            add_edge(
                &mut topology,
                exit_key,
                Curve::line(inner, outer),
                speed,
                Vec::new(),
                true,
                &mut edge_index,
            );
        }
    }

    // 环路回连边：每条出口车道顺时针绕角接到下一条入口车道（一对一，不起
    // 分流合流），提供车辆存储与成环路线。几何见 `Curve::corner_loop`。
    let turn_speed = config.speeds.turn_meters_per_second;
    let corner_radius = geometry.loop_corner_radius_meters;
    let widen = geometry.loop_outer_widen_meters;
    let loops: [(&str, Arm, usize, Arm, usize, bool); 8] = [
        ("loop-es-i0", Arm::East, 0, Arm::South, 0, false),
        ("loop-es-i1", Arm::East, 1, Arm::South, 0, true),
        ("loop-sw-i0", Arm::South, 0, Arm::West, 0, false),
        ("loop-sw-i1", Arm::South, 0, Arm::West, 1, true),
        ("loop-wn-i0", Arm::West, 0, Arm::North, 0, false),
        ("loop-wn-i1", Arm::West, 1, Arm::North, 0, true),
        ("loop-ne-i0", Arm::North, 0, Arm::East, 0, false),
        ("loop-ne-i1", Arm::North, 0, Arm::East, 1, true),
    ];
    for &(key, from, from_lane, to, to_lane, wide) in &loops {
        let from_offsets = from.lane_offsets(config);
        let to_offsets = to.lane_offsets(config);
        let start = port(from, false, from_offsets[from_lane], arm_length);
        let end = port(to, true, to_offsets[to_lane], arm_length);
        let curve = Curve::corner_loop(
            start,
            from,
            end,
            corner_radius,
            if wide { widen } else { 0.0 },
        );
        let entry_key = road_edge_key(to, true, to_lane, to_offsets.len());
        add_edge(
            &mut topology,
            key.to_owned(),
            curve,
            turn_speed,
            vec![entry_key],
            true,
            &mut edge_index,
        );
        // 出口边 -> 环路回连边。
        let exit_key = road_edge_key(from, false, from_lane, from_offsets.len());
        let exit_index = edge_index[&exit_key];
        topology.edges[exit_index].successors.push(key.to_owned());
    }

    // 路口机动：主路直行（每车道一条路径）、主路保护左转（带待转区）、
    // 次路许可左转、次路直行、主路右转、次路右转。内部转向曲线的 bezier
    // 控制腿取 curve_control_meters（与 urban-generator 模板同纪律）。
    let control = geometry.curve_control_meters;
    let mut stop_dedup = BTreeSet::new();
    for arm in Arm::ALL {
        let offsets = arm.lane_offsets(config);
        for lane in 0..offsets.len() {
            topology
                .junction_approaches
                .push(road_edge_key(arm, true, lane, offsets.len()));
        }
        for lane in 0..offsets.len() {
            topology
                .junction_approaches
                .push(road_edge_key(arm, false, lane, offsets.len()));
        }
    }

    let add_movement = |topology: &mut TopologyBuild,
                        key: &str,
                        entry: Arm,
                        exit: Arm,
                        turn: ManeuverDirection,
                        waiting: bool,
                        permissive: bool| {
        topology.movements.push(MovementBuild {
            key: key.to_owned(),
            entry,
            exit,
            turn,
            waiting,
            permissive,
        });
        topology.movements.len() - 1
    };

    // 主路直行：W→E 与 E→W 各两条车道级路径。
    for (entry, exit, key) in [(Arm::West, Arm::East, "w-e"), (Arm::East, Arm::West, "e-w")] {
        let movement = add_movement(
            &mut topology,
            key,
            entry,
            exit,
            ManeuverDirection::Straight,
            false,
            false,
        );
        for lane in 0..2 {
            let entry_offsets = entry.lane_offsets(config);
            let exit_offsets = exit.lane_offsets(config);
            let entry_edge = road_edge_key(entry, true, lane, 2);
            let exit_edge = road_edge_key(exit, false, lane, 2);
            let start = port(entry, true, entry_offsets[lane], radius);
            let end = port(exit, false, exit_offsets[lane], radius);
            let internal_key = format!("{key}.i{lane}");
            let path_key = format!("path-i{lane}");
            add_path(
                &mut topology,
                &mut edge_index,
                &mut stop_dedup,
                movement,
                &path_key,
                &entry_edge,
                &exit_edge,
                vec![(internal_key, Curve::line(start, end))],
                config.speeds.main_meters_per_second,
                &[("admission", 0, GROUP_MAIN_THROUGH_RIGHT)],
            );
        }
    }

    // 主路保护左转 W→N：12 m 待转 pocket，admission/waiting-entry/release 三门。
    {
        let movement = add_movement(
            &mut topology,
            "w-n",
            Arm::West,
            Arm::North,
            ManeuverDirection::Left,
            true,
            false,
        );
        let entry_offset = Arm::West.lane_offsets(config)[0];
        let exit_offset = Arm::North.lane_offsets(config)[0];
        let start = port(Arm::West, true, entry_offset, radius);
        let end = port(Arm::North, false, exit_offset, radius);
        let d = [1.0, 0.0];
        let out = [0.0, -1.0];
        let pocket = geometry.pocket_length_meters;
        let offset = geometry.pocket_offset_meters;
        let p1 = [
            start[0] + d[0] * control + d[1] * offset,
            start[1] + d[1] * control - d[0] * offset,
        ];
        let p2 = [p1[0] + d[0] * pocket, p1[1] + d[1] * pocket];
        let half = control / 2.0;
        add_path(
            &mut topology,
            &mut edge_index,
            &mut stop_dedup,
            movement,
            "path",
            &road_edge_key(Arm::West, true, 0, 2),
            &road_edge_key(Arm::North, false, 0, 1),
            vec![
                (
                    "w-n.i0".to_owned(),
                    Curve::bezier(
                        start,
                        [start[0] + d[0] * half, start[1] + d[1] * half],
                        [p1[0] - d[0] * half, p1[1] - d[1] * half],
                        p1,
                    ),
                ),
                ("w-n.i1".to_owned(), Curve::line(p1, p2)),
                (
                    "w-n.i2".to_owned(),
                    Curve::bezier(
                        p2,
                        [p2[0] + d[0] * control, p2[1] + d[1] * control],
                        [end[0] - out[0] * control, end[1] - out[1] * control],
                        end,
                    ),
                ),
            ],
            turn_speed,
            &[
                ("admission", 0, GROUP_MAIN_THROUGH_RIGHT),
                ("waiting-entry", 1, GROUP_MAIN_THROUGH_RIGHT),
                ("release", 2, GROUP_MAIN_LEFT),
            ],
        );
    }

    // 次路许可左转 N→E：无保护，yield 主路对向直行。
    {
        let movement = add_movement(
            &mut topology,
            "n-e",
            Arm::North,
            Arm::East,
            ManeuverDirection::Left,
            false,
            true,
        );
        let entry_offset = Arm::North.lane_offsets(config)[0];
        let exit_offset = Arm::East.lane_offsets(config)[0];
        let start = port(Arm::North, true, entry_offset, radius);
        let end = port(Arm::East, false, exit_offset, radius);
        let d = [0.0, 1.0];
        let out = [1.0, 0.0];
        add_path(
            &mut topology,
            &mut edge_index,
            &mut stop_dedup,
            movement,
            "path",
            &road_edge_key(Arm::North, true, 0, 1),
            &road_edge_key(Arm::East, false, 0, 2),
            vec![(
                "n-e.i0".to_owned(),
                Curve::bezier(
                    start,
                    [start[0] + d[0] * control, start[1] + d[1] * control],
                    [end[0] - out[0] * control, end[1] - out[1] * control],
                    end,
                ),
            )],
            turn_speed,
            &[("admission", 0, GROUP_SECONDARY_LEFT)],
        );
    }

    // 次路直行 S→N。
    {
        let movement = add_movement(
            &mut topology,
            "s-n",
            Arm::South,
            Arm::North,
            ManeuverDirection::Straight,
            false,
            false,
        );
        let offset = Arm::South.lane_offsets(config)[0];
        let start = port(Arm::South, true, offset, radius);
        let end = port(Arm::North, false, offset, radius);
        add_path(
            &mut topology,
            &mut edge_index,
            &mut stop_dedup,
            movement,
            "path",
            &road_edge_key(Arm::South, true, 0, 1),
            &road_edge_key(Arm::North, false, 0, 1),
            vec![("s-n.i0".to_owned(), Curve::line(start, end))],
            config.speeds.secondary_meters_per_second,
            &[("admission", 0, GROUP_SECONDARY_THROUGH_RIGHT)],
        );
    }

    // 主路保护左转 E→S（外侧车道）：东进口朝西行驶转入南出口是左转动作，
    // 挂主路左转组与 W→N 对向保护左转同相位放行（两条对向左转几何不交叉），
    // 并让南出口在环行中可达（支撑焦点路线成环）。
    {
        let movement = add_movement(
            &mut topology,
            "e-s",
            Arm::East,
            Arm::South,
            ManeuverDirection::Left,
            false,
            false,
        );
        let entry_offset = Arm::East.lane_offsets(config)[1];
        let exit_offset = Arm::South.lane_offsets(config)[0];
        let start = port(Arm::East, true, entry_offset, radius);
        let end = port(Arm::South, false, exit_offset, radius);
        let d = [-1.0, 0.0];
        let out = [0.0, 1.0];
        add_path(
            &mut topology,
            &mut edge_index,
            &mut stop_dedup,
            movement,
            "path",
            &road_edge_key(Arm::East, true, 1, 2),
            &road_edge_key(Arm::South, false, 0, 1),
            vec![(
                "e-s.i0".to_owned(),
                Curve::bezier(
                    start,
                    [start[0] + d[0] * control, start[1] + d[1] * control],
                    [end[0] - out[0] * control, end[1] - out[1] * control],
                    end,
                ),
            )],
            turn_speed,
            &[("admission", 0, GROUP_MAIN_LEFT)],
        );
    }

    // 次路右转 N→W。
    {
        let movement = add_movement(
            &mut topology,
            "n-w",
            Arm::North,
            Arm::West,
            ManeuverDirection::Right,
            false,
            false,
        );
        let entry_offset = Arm::North.lane_offsets(config)[0];
        let exit_offset = Arm::West.lane_offsets(config)[1];
        let start = port(Arm::North, true, entry_offset, radius);
        let end = port(Arm::West, false, exit_offset, radius);
        let d = [0.0, 1.0];
        let out = [-1.0, 0.0];
        add_path(
            &mut topology,
            &mut edge_index,
            &mut stop_dedup,
            movement,
            "path",
            &road_edge_key(Arm::North, true, 0, 1),
            &road_edge_key(Arm::West, false, 1, 2),
            vec![(
                "n-w.i0".to_owned(),
                Curve::bezier(
                    start,
                    [start[0] + d[0] * control, start[1] + d[1] * control],
                    [end[0] - out[0] * control, end[1] - out[1] * control],
                    end,
                ),
            )],
            turn_speed,
            &[("admission", 0, GROUP_SECONDARY_THROUGH_RIGHT)],
        );
    }

    topology.phases = signal_phases(config);
    add_zones_and_streams(&mut topology)?;
    topology.routes = routes();
    topology.portals = portals();
    validate_routes(&topology)?;
    Ok(topology)
}

/// 登记一条机动路径：内部边、路径链后继、停止线（按边去重）与机动门。
#[allow(clippy::too_many_arguments)]
fn add_path(
    topology: &mut TopologyBuild,
    edge_index: &mut BTreeMap<String, usize>,
    stop_dedup: &mut BTreeSet<String>,
    movement: usize,
    path_key: &str,
    entry_edge: &str,
    exit_edge: &str,
    internals: Vec<(String, Curve)>,
    internal_speed: f64,
    gates: &[(&'static str, u32, &'static str)],
) {
    let movement_build = &topology.movements[movement];
    let permissive = movement_build.permissive;
    let turn = movement_build.turn;
    let waiting = movement_build.waiting;
    let group = gates
        .first()
        .map(|(_, _, group)| *group)
        .expect("every path declares at least one gate");

    let mut path_edges = vec![entry_edge.to_owned()];
    let mut samples = Vec::new();
    let internal_count = internals.len();
    for (index, (internal_key, curve)) in internals.into_iter().enumerate() {
        if !waiting || index + 1 == internal_count {
            samples.extend(curve.conflict_samples());
        }
        edge_index.insert(internal_key.clone(), topology.edges.len());
        topology.edges.push(EdgeBuild {
            key: internal_key.clone(),
            curve,
            speed: internal_speed,
            successors: Vec::new(),
            section_derived: false,
        });
        topology.junction_internals.push(internal_key.clone());
        path_edges.push(internal_key);
    }
    path_edges.push(exit_edge.to_owned());
    // 注意：路径链不写 LaneEdge 后继。编译器规定涉及 internal 边的转移只能由
    // ManeuverPath 边序承载；环路回连的后继在环路登记处单独回写。

    let path_index = topology.paths.len();
    for &(gate_key, transition, gate_group) in gates {
        let stop_edge = &path_edges[transition as usize];
        let stop_key = format!("{stop_edge}.stop");
        if stop_dedup.insert(stop_key.clone()) {
            topology.stop_lines.push(StopLineBuild {
                key: stop_key.clone(),
                edge: stop_edge.clone(),
            });
        }
        topology.gates.push(GateBuild {
            key: gate_key,
            path: path_index,
            transition,
            stop_key,
            group: gate_group,
        });
    }
    if waiting {
        topology.waiting_zone_path = path_index;
    }
    topology.paths.push(PathBuild {
        movement,
        key: path_key.to_owned(),
        edges: path_edges,
        samples,
        permissive,
        turn,
        group,
    });
}

/// 固定时制相位程序：主路左转独占保护；主路直行与次路许可左转同相位
/// （许可左转必须在主路直行车流中找间隙）；次路直行独占。
fn signal_phases(config: &JunctionConfig) -> Vec<PhaseBuild> {
    let signals = &config.signals;
    use SignalAspect::{Green as G, Red as R, Yellow as Y};
    [
        (
            "p0.main-left-green",
            signals.main_left_green_ms,
            [R, G, R, R],
        ),
        ("p1.main-left-yellow", signals.yellow_ms, [R, Y, R, R]),
        ("p2.all-red-0", signals.all_red_ms, [R, R, R, R]),
        (
            "p3.main-through-green",
            signals.main_through_green_ms,
            [G, R, R, G],
        ),
        ("p4.main-through-yellow", signals.yellow_ms, [Y, R, R, Y]),
        ("p5.all-red-1", signals.all_red_ms, [R, R, R, R]),
        (
            "p6.secondary-through-green",
            signals.secondary_through_green_ms,
            [R, R, G, R],
        ),
        (
            "p7.secondary-through-yellow",
            signals.yellow_ms,
            [R, R, Y, R],
        ),
        ("p8.all-red-2", signals.all_red_ms, [R, R, R, R]),
    ]
    .into_iter()
    .map(|(key, duration_ms, aspects)| PhaseBuild {
        key: key.to_owned(),
        duration_ms,
        aspects,
    })
    .collect()
}

fn groups_concurrent(phases: &[PhaseBuild], first: &str, second: &str) -> bool {
    let first_index = SIGNAL_GROUPS
        .iter()
        .position(|group| *group == first)
        .expect("declared signal group");
    let second_index = SIGNAL_GROUPS
        .iter()
        .position(|group| *group == second)
        .expect("declared signal group");
    phases.iter().any(|phase| {
        phase.aspects[first_index] != SignalAspect::Red
            && phase.aspects[second_index] != SignalAspect::Red
    })
}

/// 冲突区只为「许可门路径 × 同相位并发直行路径」的几何交叉对编制；
/// 信号分离的直行对（次路直行）与转向对不占冲突区。
fn add_zones_and_streams(topology: &mut TopologyBuild) -> Result<(), Error> {
    let path_count = topology.paths.len();
    for i in 0..path_count {
        for j in (i + 1)..path_count {
            let (permissive, other) = {
                let a = &topology.paths[i];
                let b = &topology.paths[j];
                match (a.permissive, b.permissive) {
                    (true, false) => (a, b),
                    (false, true) => (b, a),
                    _ => continue,
                }
            };
            if other.turn != ManeuverDirection::Straight {
                continue;
            }
            if !groups_concurrent(&topology.phases, permissive.group, other.group) {
                continue;
            }
            if permissive.edges.first() == other.edges.first() {
                continue;
            }
            let center = if permissive.edges.last() == other.edges.last() {
                *permissive.samples.last().expect("merge path has samples")
            } else {
                let Some(center) = crossing(&permissive.samples, &other.samples) else {
                    continue;
                };
                center
            };
            let key = format!("cz-{}", topology.zones.len());
            topology.zones.push(ZoneBuild {
                key,
                center,
                paths: [i, j],
            });
        }
    }

    for (path_index, path) in topology.paths.iter().enumerate() {
        let zones: Vec<usize> = topology
            .zones
            .iter()
            .enumerate()
            .filter(|(_, zone)| zone.paths.contains(&path_index))
            .map(|(index, _)| index)
            .collect();
        if zones.is_empty() {
            continue;
        }
        let movement_key = &topology.movements[path.movement].key;
        let key = if topology.paths[path_index].key == "path" {
            format!("stream.{movement_key}")
        } else {
            format!(
                "stream.{movement_key}.{}",
                path.key.trim_start_matches("path-")
            )
        };
        let yield_to = if path.permissive {
            zones
                .iter()
                .map(|&zone| {
                    let zone_build = &topology.zones[zone];
                    let opponent = if zone_build.paths[0] == path_index {
                        zone_build.paths[1]
                    } else {
                        zone_build.paths[0]
                    };
                    stream_key(topology, opponent)
                })
                .collect()
        } else {
            Vec::new()
        };
        topology.streams.push(StreamBuild {
            key,
            path: path_index,
            zones,
            priority: if path.permissive { 0 } else { 100 },
            yield_to,
            gap: path.permissive,
        });
    }
    if topology.zones.len() != 3 {
        return Err(Error::Config(format!(
            "conflict zone count mismatch: {} zones",
            topology.zones.len()
        )));
    }
    Ok(())
}

fn stream_key(topology: &TopologyBuild, path: usize) -> String {
    let path_build = &topology.paths[path];
    let movement_key = &topology.movements[path_build.movement].key;
    if path_build.key == "path" {
        format!("stream.{movement_key}")
    } else {
        format!(
            "stream.{movement_key}.{}",
            path_build.key.trim_start_matches("path-")
        )
    }
}

/// catalog 路线表；环路让焦点路线成环并多次穿过同一机动门。
fn routes() -> Vec<RouteBuild> {
    let route = |id, exit_portal, edges: &[&str], focus| RouteBuild {
        id,
        exit_portal,
        edges: edges.iter().map(|edge| (*edge).to_owned()).collect(),
        focus,
    };
    vec![
        route(
            "route-w-through",
            "portal-loop-to-s",
            &["loop-sw-i1", "w-in-i1", "w-e.i1", "e-out-i1", "loop-es-i1"],
            false,
        ),
        route(
            "route-w-left-waiting",
            "portal-loop-to-e",
            &[
                "loop-sw-i0",
                "w-in-i0",
                "w-n.i0",
                "w-n.i1",
                "w-n.i2",
                "n-out",
                "loop-ne-i0",
            ],
            false,
        ),
        route(
            "route-w-through-circuit",
            "portal-loop-to-s",
            &[
                "loop-sw-i0",
                "w-in-i0",
                "w-e.i0",
                "e-out-i0",
                "loop-es-i0",
                "s-in",
                "s-n.i0",
                "n-out",
                "loop-ne-i1",
                "e-in-i1",
                "e-s.i0",
                "s-out",
                "loop-sw-i0",
                "w-in-i0",
                "w-e.i0",
                "e-out-i0",
                "loop-es-i0",
            ],
            true,
        ),
        route(
            "route-e-through",
            "portal-loop-to-n",
            &["loop-ne-i0", "e-in-i0", "e-w.i0", "w-out-i0", "loop-wn-i0"],
            false,
        ),
        route(
            "route-e-left",
            "portal-loop-to-w",
            &["loop-ne-i1", "e-in-i1", "e-s.i0", "s-out", "loop-sw-i0"],
            false,
        ),
        route(
            "route-e-through-lane1",
            "portal-loop-to-n",
            &["loop-ne-i1", "e-in-i1", "e-w.i1", "w-out-i1", "loop-wn-i1"],
            false,
        ),
        route(
            "route-n-permissive-left",
            "portal-loop-to-s",
            &["loop-wn-i0", "n-in", "n-e.i0", "e-out-i0", "loop-es-i0"],
            false,
        ),
        route(
            "route-n-left-circuit",
            "portal-loop-to-s",
            &[
                "loop-wn-i1",
                "n-in",
                "n-e.i0",
                "e-out-i0",
                "loop-es-i0",
                "s-in",
                "s-n.i0",
                "n-out",
                "loop-ne-i0",
                "e-in-i0",
                "e-w.i0",
                "w-out-i0",
                "loop-wn-i0",
                "n-in",
                "n-e.i0",
                "e-out-i0",
                "loop-es-i0",
            ],
            true,
        ),
        route(
            "route-s-through",
            "portal-loop-to-e",
            &["loop-es-i0", "s-in", "s-n.i0", "n-out", "loop-ne-i0"],
            false,
        ),
        route(
            "route-s-mix",
            "portal-loop-to-w",
            &[
                "loop-es-i1",
                "s-in",
                "s-n.i0",
                "n-out",
                "loop-ne-i1",
                "e-in-i1",
                "e-s.i0",
                "s-out",
                "loop-sw-i0",
            ],
            false,
        ),
        route(
            "route-s-n-w-circuit",
            "portal-loop-to-n",
            &[
                "loop-es-i1",
                "s-in",
                "s-n.i0",
                "n-out",
                "loop-ne-i0",
                "e-in-i0",
                "e-w.i0",
                "w-out-i0",
                "loop-wn-i0",
                "n-in",
                "n-w.i0",
                "w-out-i1",
                "loop-wn-i1",
            ],
            false,
        ),
    ]
}

fn portals() -> Vec<PortalBuild> {
    let lane = |edge: &str, choices: &[(&'static str, u64)]| PortalLaneBuild {
        edge: edge.to_owned(),
        choices: choices.to_vec(),
    };
    vec![
        PortalBuild {
            id: "portal-loop-to-w",
            lanes: vec![
                lane(
                    "loop-sw-i0",
                    &[
                        ("route-w-left-waiting", 50),
                        ("route-w-through-circuit", 50),
                    ],
                ),
                lane("loop-sw-i1", &[("route-w-through", 100)]),
            ],
        },
        PortalBuild {
            id: "portal-loop-to-e",
            lanes: vec![
                lane("loop-ne-i0", &[("route-e-through", 100)]),
                lane(
                    "loop-ne-i1",
                    &[("route-e-left", 50), ("route-e-through-lane1", 50)],
                ),
            ],
        },
        PortalBuild {
            id: "portal-loop-to-n",
            lanes: vec![
                lane("loop-wn-i0", &[("route-n-permissive-left", 100)]),
                lane("loop-wn-i1", &[("route-n-left-circuit", 100)]),
            ],
        },
        PortalBuild {
            id: "portal-loop-to-s",
            lanes: vec![
                lane("loop-es-i0", &[("route-s-through", 100)]),
                lane(
                    "loop-es-i1",
                    &[("route-s-mix", 50), ("route-s-n-w-circuit", 50)],
                ),
            ],
        },
    ]
}

/// 每条 catalog 路线必须边边相继且从某条 portal 环路车道出发。相继 = 车道后继
/// 连通，或相邻两边是同一机动路径的顺次边（涉及 internal 边的转移不写后继）。
fn validate_routes(topology: &TopologyBuild) -> Result<(), Error> {
    let portal_edges: BTreeSet<&str> = topology
        .portals
        .iter()
        .flat_map(|portal| portal.lanes.iter().map(|lane| lane.edge.as_str()))
        .collect();
    let path_transitions: BTreeSet<(&str, &str)> = topology
        .paths
        .iter()
        .flat_map(|path| {
            path.edges
                .windows(2)
                .map(|pair| (pair[0].as_str(), pair[1].as_str()))
        })
        .collect();
    for route in &topology.routes {
        let entry_edge = route.edges.first().expect("non-empty route");
        if !portal_edges.contains(entry_edge.as_str()) {
            return Err(Error::Config(format!(
                "route {:?} does not start on a portal loop edge",
                route.id
            )));
        }
        for pair in route.edges.windows(2) {
            let from = topology.edge(&pair[0]).ok_or_else(|| {
                Error::Config(format!(
                    "route {:?} references unknown edge {:?}",
                    route.id, pair[0]
                ))
            })?;
            let connected = from.successors.iter().any(|next| next == &pair[1])
                || path_transitions.contains(&(pair[0].as_str(), pair[1].as_str()));
            if !connected {
                return Err(Error::Config(format!(
                    "route {:?} is disconnected between {:?} and {:?}",
                    route.id, pair[0], pair[1]
                )));
            }
        }
    }
    Ok(())
}
