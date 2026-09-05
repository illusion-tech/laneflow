//! #543 调研 spike：`LF-CN-URBAN-v1` candidate rules 的 10k/100k exact-object 容量计数。
//!
//! 本二进制是研究代码，不属于生产 crate，也不是 workspace 成员。它通过公开 API 走真实
//! 编译管线（Synthetic DSL 前端 + 道路编辑前端 → Typed AST → HIR → MIR → Canonical LIR
//! → LFCA/LFSM/LFSD file-backed emission → post-emission check → SharedNetworkRevision
//! build → TrafficWorld install），产出逐阶段、逐表、逐关系的 exact-object 证据。
//!
//! 拓扑口径（candidate rules，调用方提供；当前 main 的
//! `docs/design/chinese-style-city-workload.md` 尚未写入 cell/tile 参数，见报告假设节）：
//! 250 m cell、2×5 macro-tile、整数铺设；10k = 100 cells / 10 tiles，
//! 100k = 1,000 cells / 100 tiles。
//!
//! 用法：
//!   run <cells>                   完整管线（LF-COMP-SINGLE-NETWORK-1M-v2）
//!   probe-p100 synthetic <tiles>  逐 tile 加合成模块，P100 v2 下打印首个失败诊断
//!   probe-p100 conflict <tiles>   单冲突模块装 N tiles，P100 v2 下打印首个失败诊断
//!   parts <cells>                 1M profile 分别编译 synthetic-only / conflict-only，
//!                                 打印公开 LIR 锚点用于分解可加性验证
//!   model <cells>                 只打印 #284 前的历史解析模型（不代表当前准入计数）

use std::{
    alloc::System,
    fs,
    hint::black_box,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

use laneflow_compiler::road_editing as editing;
use laneflow_compiler::{
    AccessEffect, AccessRuleInput, AccessRuleTargetInput, AuthoringLaneInput, CanonicalFrameInput,
    CanonicalPoint3F32Input, CompilationUnit, CompilationUnitBuilder, CompileLimits, Compiler,
    CorridorElementReference, DiagnosticBundle, DiagnosticPayload, EntityReference,
    GateInterpretation, GateProhibition, GeometryAccuracyProfile, GeometryDirectionProfile,
    IidmVehicleProfileInput, JunctionInput, JunctionReference, LaneEdgeGeometryInput,
    LaneEdgeInput, LaneEdgeReference, ManeuverDirection, ManeuverGateInput, ManeuverGateReference,
    ManeuverPathInput, ManeuverPathReference, MovementInput, MovementReference,
    OwnerQualifiedReference, ParkingFacilityInput, ParkingFacilityReference,
    ParkingLaneAnchorInput, ParkingSpaceGeometryInput, ParkingSpaceInput, ParticipantClassInput,
    ParticipantClassReference, PolicyGateRuleInput, PolicyInputSource, PolicyStreamRuleInput,
    PortableDiffBase, PortableEmissionProvenance, RegulationIdentity, RightOfWayPolicySetInput,
    RoadCorridorInput, RoadSectionInput, RoadSectionReference, SignalAspect, SignalControlInput,
    SignalControllerInput, SignalGroupInput, SignalGroupReference, SignalGroupStateInput,
    SignalPhaseInput, SourceModuleHeader, SourceModuleHeaderInput, StopLineInput,
    StopLineReference, SyntheticModule, SyntheticModuleBuilder, VehicleProfileInput,
    WaitingZoneInput, check_portable_candidate, emit_portable_candidate_to_staging,
};
use laneflow_format::{
    CheckedCanonicalNetworkInput, FormatLimits, ImmutableObjectSource, RegistryCheckedFieldValue,
    RegistryCheckedRowView, ValueCheckedObjectView,
};
use laneflow_runtime::{
    CommittedNetworkSource, PolicyPin, PublishedLfcaReference, TrafficWorld, WorldConfig,
    WorldPolicySelection,
};
use laneflow_static_contract::{
    EntityKind, PortableFieldType, PortableObjectKind, RightOfWayPolicySetId,
    portable_object_schema,
};
use laneflow_static_network::{
    BuildError, BuildStructure, SharedNetworkBuildLimits, SharedNetworkBuildOptions,
    SharedNetworkRevision, SpatialBuildOption, build_shared_network_revision,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

// ---------------------------------------------------------------------------
// 口径常量（candidate rules；报告假设节逐项登记）
// ---------------------------------------------------------------------------

/// cell 边长（米）。
const CELL_SIZE_M: f64 = 250.0;
/// 每个 macro-tile 的 cell 数（2×5）。
const CELLS_PER_TILE: u32 = 10;
/// 每 cell 信号化路口的引道方向（西/东/南/北）。
const APPROACHES: [&str; 4] = ["w", "e", "s", "n"];
/// 每 cell 地面混合停车设施的声明虚拟容量。
const CELL_FACILITY_VIRTUAL_CAPACITY: u32 = 100;
/// 每 macro-tile 一个 virtual-only 地下车库的声明虚拟容量。
const GARAGE_VIRTUAL_CAPACITY: u32 = 1_000;
/// 来源模块头的生成器构建标识。保持 20 字节：`generator_build_id` 计入 source
/// record 长度（`encoded_source_record_len`），改变长度会使既有证据的 source
/// bytes 锚点漂移。
const GENERATOR_BUILD_ID: &str = "laneflow-543-spikev1";
const POLICY_NAMESPACE: &str = "city/lf-cn-urban-543/policy";
const POLICY_KEY: &str = "capacity-install-policy";
const CONFLICT_NAMESPACE: &str = "city/lf-cn-urban-543/conflicts";

/// 一次规模运行的形状。
#[derive(Clone, Copy, Debug)]
struct Shape {
    cells: u32,
    tiles: u32,
}

impl Shape {
    fn for_cells(cells: u32) -> Self {
        assert_eq!(cells % CELLS_PER_TILE, 0, "cells 必须按 2×5 tile 整数铺设");
        Self {
            cells,
            tiles: cells / CELLS_PER_TILE,
        }
    }
}

/// 以整数米铺设 cell 网格：tile 宽 2 cell、高 5 cell；tile 列数取 `ceil(sqrt(2*tiles))`
///（10k → 5 列 × 2 行 tile = 10×10 cell；100k → 15 列 × 7 行 tile = 30×35 cell，
/// 全网格 7,500×8,750 m，处于规范坐标 ±16,448 m 界限内——冒烟实测 50 列布局会触发
/// `CoordinateOutOfRange`，见报告"坐标界限"节）。
fn cell_origin_m(shape: Shape, cell: u32) -> (f64, f64) {
    let tile = cell / CELLS_PER_TILE;
    let within = cell % CELLS_PER_TILE;
    let tile_cols = f64::from(2 * shape.tiles).sqrt().ceil() as u32;
    let tile_col = tile % tile_cols;
    let tile_row = tile / tile_cols;
    let col = tile_col * 2 + within % 2;
    let row = tile_row * 5 + within / 2;
    (f64::from(col) * CELL_SIZE_M, f64::from(row) * CELL_SIZE_M)
}

// ---------------------------------------------------------------------------
// #284 前的历史解析计数模型；尚未针对 turn_direction 和显式策略模块重新标定。
// 冲突（道路编辑）侧计数由 probe/parts 经验锚点取得，不做手工解析假设。
// ---------------------------------------------------------------------------

/// 历史编译准入维度的解析计数；不作为当前编译器预算或安装容量证据。
#[derive(Clone, Copy, Debug, Default)]
struct AdmissionCounts {
    declarations: u64,
    typed_ast_records: u64,
    references: u64,
    relation_occurrences: u64,
    identity_field_occurrences: u64,
    symbols: u64,
    maneuver_gates: u64,
    waiting_zones: u64,
    geometry_points: u64,
}

impl AdmissionCounts {
    fn add(&mut self, other: &Self) {
        self.declarations += other.declarations;
        self.typed_ast_records += other.typed_ast_records;
        self.references += other.references;
        self.relation_occurrences += other.relation_occurrences;
        self.identity_field_occurrences += other.identity_field_occurrences;
        self.symbols += other.symbols;
        self.maneuver_gates += other.maneuver_gates;
        self.waiting_zones += other.waiting_zones;
        self.geometry_points += other.geometry_points;
    }

    /// #284 前的单 cell 合成侧增量，逐项推导见历史报告附录。
    /// 配方：20 边（含 2 条 65m 待转专用出口边）/8 停止线/12 门/2 待转区/8 组/4 相位。
    fn synthetic_cell() -> Self {
        Self {
            declarations: 94,
            typed_ast_records: 782,
            references: 192,
            relation_occurrences: 212,
            identity_field_occurrences: 266,
            symbols: 94,
            maneuver_gates: 12,
            waiting_zones: 2,
            geometry_points: 40,
        }
    }

    /// 每 tile 一个多门地下车库（4 个虚拟锚点）。
    fn garage() -> Self {
        Self {
            declarations: 1,
            typed_ast_records: 11,
            references: 4,
            relation_occurrences: 4,
            identity_field_occurrences: 2,
            symbols: 1,
            ..Self::default()
        }
    }

    /// 每合成模块一组 ParticipantClass + VehicleProfile + AccessRule。
    fn module_shared() -> Self {
        Self {
            declarations: 3,
            typed_ast_records: 20,
            references: 3,
            relation_occurrences: 3,
            identity_field_occurrences: 6,
            symbols: 3,
            ..Self::default()
        }
    }

    fn synthetic_total(shape: Shape) -> Self {
        let mut total = Self::default();
        let cell = Self::synthetic_cell();
        for _ in 0..shape.cells {
            total.add(&cell);
        }
        let garage = Self::garage();
        let shared = Self::module_shared();
        for _ in 0..shape.tiles {
            total.add(&garage);
            total.add(&shared);
        }
        total
    }
}

// ---------------------------------------------------------------------------
// 几何：全部整数值坐标、轴对齐，边长 == 几何弧长（125.0/95.0/30.0 精确），
// 避免 ADR 0028 长度一致性拒绝。f32 精确表示全部用到的整数坐标（< 2^24）。
// ---------------------------------------------------------------------------

/// 引道入射方向（x, z）：w 向东、e 向西、s 向北、n 向南。
fn approach_in_dir(a: usize) -> (f64, f64) {
    [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)][a]
}

/// 左转方向 = 入射方向左旋 90°（(dx,dz) -> (-dz,dx)，右行制左转）。
fn approach_left_dir(a: usize) -> (f64, f64) {
    let (dx, dz) = approach_in_dir(a);
    (-dz, dx)
}

/// 直行出口方位（与 APPROACHES 下标平行）：w→e, e→w, s→n, n→s。
const THROUGH_EXIT: [usize; 4] = [1, 0, 3, 2];
/// 左转出口方位：w→n, e→s, s→w, n→e。
const LEFT_EXIT: [usize; 4] = [3, 2, 0, 1];
/// 配置左转待转区的引道（主横轴 w/e）。
const WAITING_APPROACHES: [usize; 2] = [0, 1];

fn waiting_approach(a: usize) -> bool {
    WAITING_APPROACHES.contains(&a)
}

fn pt(x: f64, z: f64) -> CanonicalPoint3F32Input {
    CanonicalPoint3F32Input {
        x: x as f32,
        y: 0.0,
        z: z as f32,
    }
}

/// cell 内全部 20 条边的两端点几何。
///
/// 编译器验证 path 相邻边几何端点连续（DiscontinuousJoin；冒烟实测拒绝 30m 缺口），
/// 而待转 path 的第二段内部边终止于 C+60m、共享出口边起于 C+30m，因此 w/e 左转各配
/// 一条专用 65m 出口边（不计入任何 lane chain；参照既有测试内部边可不入链）。
struct CellGeometry {
    entry: [(CanonicalPoint3F32Input, CanonicalPoint3F32Input); 4],
    internal_through: [(CanonicalPoint3F32Input, CanonicalPoint3F32Input); 4],
    internal_left1: [(CanonicalPoint3F32Input, CanonicalPoint3F32Input); 4],
    internal_left2: [(CanonicalPoint3F32Input, CanonicalPoint3F32Input); 4],
    exit: [(CanonicalPoint3F32Input, CanonicalPoint3F32Input); 4],
    exit_waiting: [(CanonicalPoint3F32Input, CanonicalPoint3F32Input); 4],
}

fn cell_geometry(shape: Shape, cell: u32) -> CellGeometry {
    let (ox, oz) = cell_origin_m(shape, cell);
    let cx = ox + 125.0;
    let cz = oz + 125.0;
    let zero = (pt(0.0, 0.0), pt(0.0, 0.0));
    let mut geometry = CellGeometry {
        entry: [zero; 4],
        internal_through: [zero; 4],
        internal_left1: [zero; 4],
        internal_left2: [zero; 4],
        exit: [zero; 4],
        exit_waiting: [zero; 4],
    };
    for a in 0..4 {
        let (dx, dz) = approach_in_dir(a);
        let (lx, lz) = approach_left_dir(a);
        geometry.entry[a] = (pt(cx - 125.0 * dx, cz - 125.0 * dz), pt(cx, cz));
        geometry.internal_through[a] = (pt(cx, cz), pt(cx + 30.0 * dx, cz + 30.0 * dz));
        geometry.internal_left1[a] = (pt(cx, cz), pt(cx + 30.0 * lx, cz + 30.0 * lz));
        geometry.internal_left2[a] = (
            pt(cx + 30.0 * lx, cz + 30.0 * lz),
            pt(cx + 60.0 * lx, cz + 60.0 * lz),
        );
        geometry.exit_waiting[a] = (
            pt(cx + 60.0 * lx, cz + 60.0 * lz),
            pt(cx + 125.0 * lx, cz + 125.0 * lz),
        );
    }
    for (x, slot) in geometry.exit.iter_mut().enumerate() {
        // 出口方位 x 的出射方向 = 对面（直行来向）引道的入射方向。
        let (dx, dz) = approach_in_dir(THROUGH_EXIT[x]);
        *slot = (
            pt(cx + 30.0 * dx, cz + 30.0 * dz),
            pt(cx + 125.0 * dx, cz + 125.0 * dz),
        );
    }
    geometry
}

struct CellKeys {
    prefix: String,
}

impl CellKeys {
    fn new(cell: u32) -> Self {
        Self {
            prefix: format!("c{cell:07}"),
        }
    }

    fn k(&self, suffix: &str) -> String {
        format!("{}-{suffix}", self.prefix)
    }
}

// ---------------------------------------------------------------------------
// 合成前端：每 module = 1 macro-tile = 10 cells + 1 地下车库 + 模块级共享三元组。
// ---------------------------------------------------------------------------

/// 向合成模块写入一个 cell（4 引道信号化十字 + 左转待转 + 混合停车）。
/// 所有键先落成 owned String 再取引用，保证输入结构体的借用生命周期。
fn add_synthetic_cell(builder: &mut SyntheticModuleBuilder, shape: Shape, cell: u32) {
    let keys = CellKeys::new(cell);
    let geom = cell_geometry(shape, cell);

    // ---- 20 条车道图边 ----
    let entry_keys: Vec<String> = (0..4)
        .map(|a| keys.k(&format!("{}-entry", APPROACHES[a])))
        .collect();
    let int_t_keys: Vec<String> = (0..4)
        .map(|a| keys.k(&format!("{}-int-t", APPROACHES[a])))
        .collect();
    let int_l1_keys: Vec<String> = (0..4)
        .map(|a| keys.k(&format!("{}-int-l1", APPROACHES[a])))
        .collect();
    let int_l2_keys: Vec<String> = (0..4)
        .map(|a| keys.k(&format!("{}-int-l2", APPROACHES[a])))
        .collect();
    let exit_keys: Vec<String> = (0..4)
        .map(|x| keys.k(&format!("exit-{}", APPROACHES[x])))
        .collect();
    let exit_waiting_keys: Vec<String> = (0..4)
        .map(|a| keys.k(&format!("exit-{}-lw", APPROACHES[a])))
        .collect();
    for a in 0..4 {
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: &entry_keys[a],
                length_meters: 125.0,
                speed_limit_meters_per_second: 13.0,
                successors: &[
                    LaneEdgeReference::local(&int_t_keys[a]),
                    LaneEdgeReference::local(&int_l1_keys[a]),
                ],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: &int_t_keys[a],
                length_meters: 30.0,
                speed_limit_meters_per_second: 8.0,
                successors: &[LaneEdgeReference::local(&exit_keys[THROUGH_EXIT[a]])],
            })
            .unwrap();
        let left_successor: &String = if waiting_approach(a) {
            &int_l2_keys[a]
        } else {
            &exit_keys[LEFT_EXIT[a]]
        };
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: &int_l1_keys[a],
                length_meters: 30.0,
                speed_limit_meters_per_second: 8.0,
                successors: &[LaneEdgeReference::local(left_successor)],
            })
            .unwrap();
        if waiting_approach(a) {
            builder
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: &int_l2_keys[a],
                    length_meters: 30.0,
                    speed_limit_meters_per_second: 8.0,
                    successors: &[LaneEdgeReference::local(&exit_waiting_keys[a])],
                })
                .unwrap()
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: &exit_waiting_keys[a],
                    length_meters: 65.0,
                    speed_limit_meters_per_second: 13.0,
                    successors: &[],
                })
                .unwrap();
        }
    }
    for exit_key in &exit_keys {
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: exit_key,
                length_meters: 95.0,
                speed_limit_meters_per_second: 13.0,
                successors: &[],
            })
            .unwrap();
    }

    // ---- 4 走廊 ×（1 区段 × 2 车道）：lane-in = [entry]，lane-out = [exit-a] ----
    let section_keys: Vec<String> = (0..4)
        .map(|a| keys.k(&format!("section-{}", APPROACHES[a])))
        .collect();
    let lane_in_keys: Vec<String> = (0..4)
        .map(|a| keys.k(&format!("lane-{}-in", APPROACHES[a])))
        .collect();
    let lane_out_keys: Vec<String> = (0..4)
        .map(|a| keys.k(&format!("lane-{}-out", APPROACHES[a])))
        .collect();
    let corridor_keys: Vec<String> = (0..4)
        .map(|a| keys.k(&format!("corridor-{}", APPROACHES[a])))
        .collect();
    for a in 0..4 {
        builder
            .add_road_section(RoadSectionInput {
                road_section_key: &section_keys[a],
                kind_id: "motorLane",
                lanes: &[
                    AuthoringLaneInput {
                        authoring_lane_key: &lane_in_keys[a],
                        edge_chain: &[LaneEdgeReference::local(&entry_keys[a])],
                        lane_group: None,
                    },
                    AuthoringLaneInput {
                        authoring_lane_key: &lane_out_keys[a],
                        edge_chain: &[LaneEdgeReference::local(&exit_keys[a])],
                        lane_group: None,
                    },
                ],
            })
            .unwrap()
            .add_road_corridor(RoadCorridorInput {
                road_corridor_key: &corridor_keys[a],
                reference_section: RoadSectionReference::local(&section_keys[a]),
                elements: &[CorridorElementReference::road_section(
                    RoadSectionReference::local(&section_keys[a]),
                )],
            })
            .unwrap();
    }

    // ---- 路口 + 8 movement + 8 path（w/e 左转 path 携带 2 条内部边以容纳待转区两段门）----
    let junction_key = keys.k("junction");
    builder
        .add_junction(JunctionInput {
            junction_key: &junction_key,
        })
        .unwrap();
    let movement_keys: Vec<String> = (0..4)
        .flat_map(|a| {
            [
                keys.k(&format!("mv-{}-t", APPROACHES[a])),
                keys.k(&format!("mv-{}-l", APPROACHES[a])),
            ]
        })
        .collect();
    let path_keys: Vec<String> = (0..4)
        .flat_map(|a| {
            [
                keys.k(&format!("mp-{}-t", APPROACHES[a])),
                keys.k(&format!("mp-{}-l", APPROACHES[a])),
            ]
        })
        .collect();
    for a in 0..4 {
        for (kind, exit_idx, slot) in [
            ("t", THROUGH_EXIT[a], 2 * a),
            ("l", LEFT_EXIT[a], 2 * a + 1),
        ] {
            let internals: Vec<LaneEdgeReference> = if kind == "t" {
                vec![LaneEdgeReference::local(&int_t_keys[a])]
            } else if waiting_approach(a) {
                vec![
                    LaneEdgeReference::local(&int_l1_keys[a]),
                    LaneEdgeReference::local(&int_l2_keys[a]),
                ]
            } else {
                vec![LaneEdgeReference::local(&int_l1_keys[a])]
            };
            // 待转 path 走专用 65m 出口边（几何端点连续）；其余共享出口边。
            let exit_edge_key: &String = if kind == "l" && waiting_approach(a) {
                &exit_waiting_keys[a]
            } else {
                &exit_keys[exit_idx]
            };
            builder
                .add_movement(MovementInput {
                    movement_key: &movement_keys[slot],
                    junction: JunctionReference::local(&junction_key),
                    directed_entry_approach_key: APPROACHES[a],
                    directed_exit_approach_key: APPROACHES[exit_idx],
                    turn_direction: Some(if kind == "t" {
                        ManeuverDirection::Straight
                    } else {
                        ManeuverDirection::Left
                    }),
                })
                .unwrap()
                .add_maneuver_path(ManeuverPathInput {
                    maneuver_path_key: &path_keys[slot],
                    movement: MovementReference::local(&movement_keys[slot]),
                    entry_edge: LaneEdgeReference::local(&entry_keys[a]),
                    internal_edges: &internals,
                    exit_edge: LaneEdgeReference::local(exit_edge_key),
                })
                .unwrap();
        }
    }

    // ---- 停止线：4 引道 + 每条待转 path 的 int-l1/int-l2 各一 ----
    let stop_keys: Vec<String> = (0..4)
        .map(|a| keys.k(&format!("sl-{}", APPROACHES[a])))
        .collect();
    let zone_stop_in_keys: Vec<String> = (0..4)
        .map(|a| keys.k(&format!("slz-{}-in", APPROACHES[a])))
        .collect();
    let zone_stop_out_keys: Vec<String> = (0..4)
        .map(|a| keys.k(&format!("slz-{}-out", APPROACHES[a])))
        .collect();
    for a in 0..4 {
        builder
            .add_stop_line(StopLineInput {
                stop_line_key: &stop_keys[a],
                lane_edge: LaneEdgeReference::local(&entry_keys[a]),
            })
            .unwrap();
        if waiting_approach(a) {
            builder
                .add_stop_line(StopLineInput {
                    stop_line_key: &zone_stop_in_keys[a],
                    lane_edge: LaneEdgeReference::local(&int_l1_keys[a]),
                })
                .unwrap()
                .add_stop_line(StopLineInput {
                    stop_line_key: &zone_stop_out_keys[a],
                    lane_edge: LaneEdgeReference::local(&int_l2_keys[a]),
                })
                .unwrap();
        }
    }

    // ---- 8 信号组（每引道直行/左转各一）----
    // 规范顺序：[w-t, w-l, e-t, e-l, s-t, s-l, n-t, n-l]
    let group_keys: Vec<String> = (0..4)
        .flat_map(|a| {
            [
                keys.k(&format!("sg-{}-t", APPROACHES[a])),
                keys.k(&format!("sg-{}-l", APPROACHES[a])),
            ]
        })
        .collect();
    for group_key in &group_keys {
        builder
            .add_signal_group(SignalGroupInput {
                signal_group_key: group_key,
            })
            .unwrap();
    }

    // ---- 12 门：8 准入（t=0，停止线在 entry）+ 待转 path 各 2（t=1 入区 / t=2 出区）----
    let zone_gate_in_keys: Vec<String> = (0..4)
        .map(|a| keys.k(&format!("g-{}-lw-in", APPROACHES[a])))
        .collect();
    let zone_gate_out_keys: Vec<String> = (0..4)
        .map(|a| keys.k(&format!("g-{}-lw-out", APPROACHES[a])))
        .collect();
    let zone_keys: Vec<String> = (0..4)
        .map(|a| keys.k(&format!("wz-{}", APPROACHES[a])))
        .collect();
    for a in 0..4 {
        for (kind, slot) in [("t", 2 * a), ("l", 2 * a + 1)] {
            builder
                .add_maneuver_gate(ManeuverGateInput {
                    maneuver_gate_key: &keys.k(&format!("g-{}-{kind}", APPROACHES[a])),
                    maneuver_path: ManeuverPathReference::local(&path_keys[slot]),
                    transition_index: 0,
                    stop_line: StopLineReference::local(&stop_keys[a]),
                    signal_control: SignalControlInput::Group(SignalGroupReference::local(
                        &group_keys[slot],
                    )),
                })
                .unwrap();
        }
        if waiting_approach(a) {
            builder
                .add_maneuver_gate(ManeuverGateInput {
                    maneuver_gate_key: &zone_gate_in_keys[a],
                    maneuver_path: ManeuverPathReference::local(&path_keys[2 * a + 1]),
                    transition_index: 1,
                    stop_line: StopLineReference::local(&zone_stop_in_keys[a]),
                    signal_control: SignalControlInput::Group(SignalGroupReference::local(
                        &group_keys[2 * a + 1],
                    )),
                })
                .unwrap()
                .add_maneuver_gate(ManeuverGateInput {
                    maneuver_gate_key: &zone_gate_out_keys[a],
                    maneuver_path: ManeuverPathReference::local(&path_keys[2 * a + 1]),
                    transition_index: 2,
                    stop_line: StopLineReference::local(&zone_stop_out_keys[a]),
                    signal_control: SignalControlInput::Group(SignalGroupReference::local(
                        &group_keys[2 * a + 1],
                    )),
                })
                .unwrap()
                .add_waiting_zone(WaitingZoneInput {
                    waiting_zone_key: &zone_keys[a],
                    maneuver_path: ManeuverPathReference::local(&path_keys[2 * a + 1]),
                    entry_gate: ManeuverGateReference::local(&zone_gate_in_keys[a]),
                    release_gate: ManeuverGateReference::local(&zone_gate_out_keys[a]),
                    max_occupancy: 4,
                })
                .unwrap();
        }
    }

    // ---- 1 控制器 × 4 相位 × 8 组状态（EW 直行 / EW 左转 / NS 直行 / NS 左转）----
    let controller_key = keys.k("controller");
    // (相位键后缀, 激活引道轴, 激活类型, 时长)
    let phase_defs: [(&str, [usize; 2], &str, u64); 4] = [
        ("phase-ew-t", [0, 1], "t", 35_000),
        ("phase-ew-l", [0, 1], "l", 20_000),
        ("phase-ns-t", [2, 3], "t", 35_000),
        ("phase-ns-l", [2, 3], "l", 20_000),
    ];
    let phase_keys: Vec<String> = phase_defs.iter().map(|(pk, ..)| keys.k(pk)).collect();
    let phase_states: Vec<Vec<SignalGroupStateInput>> = phase_defs
        .iter()
        .map(|(_, axis, kind, _)| {
            (0..4)
                .flat_map(|a| {
                    let active = axis.contains(&a);
                    [
                        SignalGroupStateInput {
                            signal_group: SignalGroupReference::local(&group_keys[2 * a]),
                            aspect: if active && *kind == "t" {
                                SignalAspect::Green
                            } else {
                                SignalAspect::Red
                            },
                        },
                        SignalGroupStateInput {
                            signal_group: SignalGroupReference::local(&group_keys[2 * a + 1]),
                            aspect: if active && *kind == "l" {
                                SignalAspect::Green
                            } else {
                                SignalAspect::Red
                            },
                        },
                    ]
                })
                .collect()
        })
        .collect();
    let phases: Vec<SignalPhaseInput> = phase_defs
        .iter()
        .enumerate()
        .map(|(i, (_, _, _, duration))| SignalPhaseInput {
            signal_phase_key: &phase_keys[i],
            duration_ms: *duration,
            states: &phase_states[i],
        })
        .collect();
    let group_refs: Vec<SignalGroupReference> = group_keys
        .iter()
        .map(|k| SignalGroupReference::local(k))
        .collect();
    builder
        .add_signal_controller(SignalControllerInput {
            signal_controller_key: &controller_key,
            offset_ms: u64::from(cell % 1_000),
            signal_groups: &group_refs,
            phases: &phases,
        })
        .unwrap();

    // ---- 地面混合停车设施（virtual 100 + 1 进 1 出锚点）+ 4 个路侧显式泊位 ----
    let facility_key = keys.k("parking");
    builder
        .add_parking_facility(ParkingFacilityInput {
            parking_facility_key: &facility_key,
            virtual_capacity: CELL_FACILITY_VIRTUAL_CAPACITY,
            virtual_entries: &[ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local(&entry_keys[0]),
                progress_meters: 100.0,
            }],
            virtual_exits: &[ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local(&exit_keys[0]),
                progress_meters: 5.0,
            }],
        })
        .unwrap();
    let space_keys: Vec<String> = (0..4)
        .map(|a| keys.k(&format!("space-{}", APPROACHES[a])))
        .collect();
    for a in 0..4 {
        builder
            .add_parking_space(ParkingSpaceInput {
                parking_space_key: &space_keys[a],
                parking_facility: Some(ParkingFacilityReference::local(&facility_key)),
                entry: ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local(&entry_keys[a]),
                    progress_meters: 110.0,
                },
                exit: ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local(&exit_keys[a]),
                    progress_meters: 15.0,
                },
                geometry: ParkingSpaceGeometryInput {
                    lateral_offset_meters: -3.0,
                    heading_offset_radians: 0.0,
                    length_meters: 5.5,
                    width_meters: 2.6,
                },
            })
            .unwrap();
    }

    // ---- 规范框架：20 边 × 2 整数值点 ----
    let frame_key = keys.k("frame");
    let mut point_pairs: Vec<(String, [CanonicalPoint3F32Input; 2])> = Vec::with_capacity(20);
    for a in 0..4 {
        point_pairs.push((entry_keys[a].clone(), [geom.entry[a].0, geom.entry[a].1]));
        point_pairs.push((
            int_t_keys[a].clone(),
            [geom.internal_through[a].0, geom.internal_through[a].1],
        ));
        point_pairs.push((
            int_l1_keys[a].clone(),
            [geom.internal_left1[a].0, geom.internal_left1[a].1],
        ));
        if waiting_approach(a) {
            point_pairs.push((
                int_l2_keys[a].clone(),
                [geom.internal_left2[a].0, geom.internal_left2[a].1],
            ));
            point_pairs.push((
                exit_waiting_keys[a].clone(),
                [geom.exit_waiting[a].0, geom.exit_waiting[a].1],
            ));
        }
    }
    for x in 0..4 {
        point_pairs.push((exit_keys[x].clone(), [geom.exit[x].0, geom.exit[x].1]));
    }
    let geometries: Vec<LaneEdgeGeometryInput> = point_pairs
        .iter()
        .map(|(edge_key, points)| LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local(edge_key),
            centerline_points: points,
        })
        .collect();
    builder
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: &frame_key,
            lane_edge_geometries: &geometries,
        })
        .unwrap();
}

/// FNV-1a 64 位混合。仅用于 provenance 摘要的确定性派生，非密码学用途
///（摘要算法契约归生成方，见 `SourceModuleHeaderInput` 字段文档）。
fn fnv1a64(mut state: u64, bytes: &[u8]) -> u64 {
    for &byte in bytes {
        state = (state ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    state
}

/// 把各输入段（带分隔边界）混成 64 位种子，再往返 4 轮摊满定长 32 字节摘要。
/// 任一输入段变化即得到不同摘要；输出定长，不影响 source record 长度与测量值。
fn derive_provenance_digest(parts: &[&[u8]]) -> [u8; 32] {
    let mut state = 0xcbf2_9ce4_8422_2325_u64;
    for part in parts {
        state = fnv1a64(state, part);
        state = fnv1a64(state, &[0xff]);
    }
    let mut digest = [0_u8; 32];
    for (round, chunk) in digest.chunks_exact_mut(8).enumerate() {
        state = fnv1a64(state, &(round as u64).to_le_bytes());
        chunk.copy_from_slice(&state.to_le_bytes());
    }
    digest
}

fn synthetic_module_header(limits: &CompileLimits, shape: Shape, tile: u32) -> SourceModuleHeader {
    // 两个摘要均从模块实际参数/编译选项确定性派生：不同 shape/tile/profile 必不同。
    let parameters_and_inputs_digest = derive_provenance_digest(&[
        &shape.cells.to_le_bytes(),
        &shape.tiles.to_le_bytes(),
        &tile.to_le_bytes(),
        &CELLS_PER_TILE.to_le_bytes(),
        &CELL_FACILITY_VIRTUAL_CAPACITY.to_le_bytes(),
        &GARAGE_VIRTUAL_CAPACITY.to_le_bytes(),
        &543_u64.to_le_bytes(),
    ]);
    let frontend_options_digest =
        derive_provenance_digest(&[b"synthetic".as_slice(), limits.profile_id().as_bytes()]);
    SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: &format!("city/lf-cn-urban-543/t{tile:03}"),
            source_document_key: &format!("t{tile:03}.document"),
            generator_build_id: GENERATOR_BUILD_ID,
            parameters_and_inputs_digest,
            frontend_options_digest,
            random_seed: Some(543),
            provenance: "repository:laneflow",
        },
        limits,
    )
    .unwrap()
}

/// 一个 macro-tile 的合成模块：10 cells + 1 多门地下车库 + 模块级共享三元组。
fn build_synthetic_tile_module(limits: &CompileLimits, shape: Shape, tile: u32) -> SyntheticModule {
    let header = synthetic_module_header(limits, shape, tile);
    let mut module = SyntheticModuleBuilder::new(header, limits).unwrap();
    for within in 0..CELLS_PER_TILE {
        add_synthetic_cell(&mut module, shape, tile * CELLS_PER_TILE + within);
    }

    // 每 tile 一个 virtual-only 地下车库（2 进 2 出多门锚点，挂在 tile 首个 cell）。
    let first = CellKeys::new(tile * CELLS_PER_TILE);
    let garage_key = format!("t{tile:03}-garage");
    let garage_entry_w = first.k("w-entry");
    let garage_entry_e = first.k("e-entry");
    let garage_exit_w = first.k("exit-w");
    let garage_exit_e = first.k("exit-e");
    module
        .add_parking_facility(ParkingFacilityInput {
            parking_facility_key: &garage_key,
            virtual_capacity: GARAGE_VIRTUAL_CAPACITY,
            virtual_entries: &[
                ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local(&garage_entry_w),
                    progress_meters: 100.0,
                },
                ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local(&garage_entry_e),
                    progress_meters: 100.0,
                },
            ],
            virtual_exits: &[
                ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local(&garage_exit_w),
                    progress_meters: 5.0,
                },
                ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local(&garage_exit_e),
                    progress_meters: 5.0,
                },
            ],
        })
        .unwrap();

    // 模块级共享三元组（避免跨模块 import；见报告假设）。
    module
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "car",
            extends: None,
        })
        .unwrap()
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "vehicle",
            participant_class: ParticipantClassReference::local("car"),
            iidm: IidmVehicleProfileInput {
                length_meters: 4.5,
                desired_speed_meters_per_second: 13.0,
                min_gap_meters: 2.0,
                time_headway_seconds: 1.5,
                max_acceleration_meters_per_second_squared: 1.5,
                comfortable_deceleration_meters_per_second_squared: 2.0,
                emergency_deceleration_meters_per_second_squared: 4.0,
            },
        })
        .unwrap()
        .add_access_rule(AccessRuleInput {
            access_rule_key: "access",
            target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local(&garage_entry_w)),
            effect: AccessEffect::Allow,
            participant_classes: &[ParticipantClassReference::local("car")],
            regulation: None,
            priority: 0,
        })
        .unwrap();
    module.finish().unwrap()
}

// ---------------------------------------------------------------------------
// 道路编辑前端：单模块装全部 cells 的冲突 tile（无保护转向 + ConflictZone +
// ParticipantStream；synthetic 前端无这两类声明，见报告假设）。
// 结构参照 crates/laneflow-compiler/tests/portable_emission_resources.rs 的
// build_conflict_module，加 per-cell 整数 offset。
// ---------------------------------------------------------------------------

fn build_conflict_module(
    limits: &CompileLimits,
    shape: Shape,
) -> editing::OwnedRoadEditingSourceBuffer {
    let header = editing::RoadEditingModuleHeader::try_new(
        "city/lf-cn-urban-543/conflicts",
        "conflicts.document",
        Vec::new(),
        editing::RoadEditingProvenance::direct("issue-543 capacity spike conflict fixtures")
            .unwrap(),
    )
    .unwrap();
    let mut builder = editing::RoadEditingSourceModuleBuilder::new(
        header,
        GeometryAccuracyProfile::Balanced5Cm,
        GeometryDirectionProfile::Balanced2Deg,
        limits,
    )
    .unwrap();

    for cell in 0..shape.cells {
        let (ox, oz) = cell_origin_m(shape, cell);
        let keys = CellKeys::new(cell);
        let frame_key = keys.k("x-frame");
        let junction_key = keys.k("x-junction");
        let junction = editing::JunctionReference::local(&junction_key).unwrap();
        let zone = editing::ConflictZoneReference::owner_scoped(vec![junction_key.clone()], "zone")
            .unwrap();
        let curve = |start_x: f64, end_x: f64| {
            editing::RoadEditingCurveProgram::try_new(
                editing::RoadEditingPoint3::try_new(ox + start_x, 0.0, oz + 125.0).unwrap(),
                vec![editing::RoadEditingCurveSegment::line(
                    editing::RoadEditingPoint3::try_new(ox + end_x, 0.0, oz + 125.0).unwrap(),
                )],
            )
            .unwrap()
        };

        builder
            .add_declaration(editing::RoadEditingDeclaration::CanonicalFrame(
                editing::CanonicalFrameInput::try_new(&frame_key).unwrap(),
            ))
            .unwrap();

        for suffix in ["entry-a", "exit-a", "entry-b", "exit-b"] {
            let alignment_key = keys.k(&format!("x-alignment-{suffix}"));
            let corridor_key = keys.k(&format!("x-corridor-{suffix}"));
            let section_key = keys.k(&format!("x-section-{suffix}"));
            let lane_key = keys.k(&format!("x-lane-{suffix}"));
            let edge_key = keys.k(&format!("x-edge-{suffix}"));
            let corridor = editing::RoadCorridorReference::local(&corridor_key).unwrap();
            let section = editing::RoadSectionReference::owner_scoped(
                vec![corridor_key.clone()],
                &section_key,
            )
            .unwrap();
            let lane = editing::AuthoringLaneReference::owner_scoped(
                vec![corridor_key.clone(), section_key.clone()],
                &lane_key,
            )
            .unwrap();
            let alignment_curve = if suffix.starts_with("entry") {
                curve(0.0, 30.0)
            } else {
                curve(60.0, 90.0)
            };
            builder
                .add_alignment(
                    editing::RoadAlignmentInput::try_new(
                        &alignment_key,
                        editing::CanonicalFrameReference::local(&frame_key).unwrap(),
                        alignment_curve,
                    )
                    .unwrap(),
                )
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::RoadCorridor(
                    editing::RoadCorridorInput::try_new(
                        &corridor_key,
                        editing::RoadAlignmentReference::try_new(&alignment_key).unwrap(),
                        0.0,
                        editing::RoadEditingStationEnd::AlignmentEnd,
                        section.clone(),
                        lane.clone(),
                        vec![editing::RoadEditingCorridorElement::RoadSection(
                            section.clone(),
                        )],
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::RoadSection(
                    editing::RoadSectionInput::try_new(
                        &section_key,
                        "motorLane",
                        vec![lane],
                        corridor,
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::AuthoringLane(
                    editing::AuthoringLaneInput::try_new(
                        &lane_key,
                        editing::LaneEdgeReference::local(&edge_key).unwrap(),
                        editing::RoadEditingLaneDirection::Forward,
                        editing::LinearWidthProfile::try_new(3.5, 3.5).unwrap(),
                        None,
                        section,
                    )
                    .unwrap(),
                ))
                .unwrap();
        }

        let internal_a_key = keys.k("x-internal-a");
        let internal_b_key = keys.k("x-internal-b");
        let edge_entry_a_key = keys.k("x-edge-entry-a");
        let edge_exit_a_key = keys.k("x-edge-exit-a");
        let edge_entry_b_key = keys.k("x-edge-entry-b");
        let edge_exit_b_key = keys.k("x-edge-exit-b");
        for (edge_key, explicit_geometry) in [
            (&edge_entry_a_key, None),
            (&internal_a_key, Some(curve(30.0, 60.0))),
            (&edge_exit_a_key, None),
            (&edge_entry_b_key, None),
            (&internal_b_key, Some(curve(30.0, 60.0))),
            (&edge_exit_b_key, None),
        ] {
            builder
                .add_declaration(editing::RoadEditingDeclaration::LaneEdge(
                    editing::LaneEdgeInput::try_new(edge_key, 13.0, Vec::new(), explicit_geometry)
                        .unwrap(),
                ))
                .unwrap();
        }

        let boundary_edges = [
            &edge_entry_a_key,
            &edge_exit_a_key,
            &edge_entry_b_key,
            &edge_exit_b_key,
        ];
        let internal_edges = [&internal_a_key, &internal_b_key];
        builder
            .add_declaration(editing::RoadEditingDeclaration::Junction(
                editing::JunctionInput::try_new(
                    &junction_key,
                    boundary_edges
                        .into_iter()
                        .map(|key| editing::LaneEdgeReference::local(key).unwrap())
                        .collect(),
                    internal_edges
                        .into_iter()
                        .map(|key| editing::LaneEdgeReference::local(key).unwrap())
                        .collect(),
                )
                .unwrap(),
            ))
            .unwrap()
            .add_declaration(editing::RoadEditingDeclaration::ConflictZone(
                editing::ConflictZoneInput::try_new("zone", junction.clone()).unwrap(),
            ))
            .unwrap();

        for suffix in ["a", "b"] {
            let movement_key = format!("movement-{suffix}");
            let stream_key = format!("stream-{suffix}");
            let stop_key = keys.k(&format!("x-stop-{suffix}"));
            let (entry_key, internal_key, exit_key) = match suffix {
                "a" => (&edge_entry_a_key, &internal_a_key, &edge_exit_a_key),
                _ => (&edge_entry_b_key, &internal_b_key, &edge_exit_b_key),
            };
            let movement = editing::MovementReference::owner_scoped(
                vec![junction_key.clone()],
                movement_key.clone(),
            )
            .unwrap();
            let path = editing::ManeuverPathReference::owner_scoped(
                vec![junction_key.clone(), movement_key.clone()],
                "path",
            )
            .unwrap();
            let admission_gate = editing::ManeuverGateReference::owner_scoped(
                vec![
                    junction_key.clone(),
                    movement_key.clone(),
                    "path".to_owned(),
                ],
                "admission",
            )
            .unwrap();
            let stop_line = editing::StopLineReference::local(&stop_key).unwrap();
            builder
                .add_declaration(editing::RoadEditingDeclaration::Movement(
                    editing::MovementInput::try_new(
                        &movement_key,
                        junction.clone(),
                        format!("entry-{suffix}"),
                        format!("exit-{suffix}"),
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::ManeuverPath(
                    editing::ManeuverPathInput::try_new(
                        "path",
                        movement,
                        editing::LaneEdgeReference::local(entry_key).unwrap(),
                        vec![editing::LaneEdgeReference::local(internal_key).unwrap()],
                        editing::LaneEdgeReference::local(exit_key).unwrap(),
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::StopLine(
                    editing::StopLineInput::try_new(
                        &stop_key,
                        editing::LaneEdgeReference::local(entry_key).unwrap(),
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::ManeuverGate(
                    editing::ManeuverGateInput::try_new(
                        "admission",
                        path.clone(),
                        0,
                        stop_line,
                        editing::RoadEditingSignalControl::None,
                    )
                    .unwrap(),
                ))
                .unwrap()
                .add_declaration(editing::RoadEditingDeclaration::ParticipantStream(
                    editing::ParticipantStreamInput::try_new(
                        &stream_key,
                        junction.clone(),
                        path,
                        vec![editing::ConflictPassageInput::new(
                            zone.clone(),
                            editing::PathAnchorInput::gate(admission_gate),
                            editing::PathAnchorInput::edge_boundary(2),
                        )],
                    )
                    .unwrap(),
                ))
                .unwrap();
        }
        builder
            .add_conflict_zone_region(
                editing::ConflictZoneRegionInput::try_new(
                    zone,
                    editing::CanonicalFrameReference::local(&frame_key).unwrap(),
                    -1.0,
                    1.0,
                    vec![
                        editing::RoadEditingPoint2::try_new(ox + 119.0, oz + 119.0).unwrap(),
                        editing::RoadEditingPoint2::try_new(ox + 131.0, oz + 119.0).unwrap(),
                        editing::RoadEditingPoint2::try_new(ox + 131.0, oz + 131.0).unwrap(),
                        editing::RoadEditingPoint2::try_new(ox + 119.0, oz + 131.0).unwrap(),
                    ],
                )
                .unwrap(),
            )
            .unwrap();
    }
    editing::RoadEditingSourceWriter::new(limits)
        .write(builder.finish().unwrap())
        .unwrap()
}

// ---------------------------------------------------------------------------
// 编译单元组装。
// ---------------------------------------------------------------------------

fn add_conflict_to_unit(
    unit: &mut CompilationUnitBuilder,
    conflicts: &editing::OwnedRoadEditingSourceBuffer,
) {
    unit.add_road_editing_module(
        editing::RoadEditingModuleInput::try_new("conflicts.document", conflicts.as_bytes(), None)
            .unwrap(),
    )
    .unwrap();
}

/// 研究宿主的显式安装策略。按生成器已声明的键组装，不从已编译网络猜测规则。
/// 这里只恢复空世界安装测量；不声称覆盖中国道路规则或运行层容量。
fn build_install_policy(limits: &CompileLimits, shape: Shape) -> SyntheticModule {
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: POLICY_NAMESPACE,
            source_document_key: "policy.document",
            generator_build_id: "laneflow-543-policy-v1",
            parameters_and_inputs_digest: derive_provenance_digest(&[&shape.cells.to_le_bytes()]),
            frontend_options_digest: derive_provenance_digest(&[b"capacity-install-policy-v1"]),
            random_seed: Some(543),
            provenance: "repository:issue-543-capacity-install-policy",
        },
        limits,
    )
    .unwrap();
    let mut builder = SyntheticModuleBuilder::new(header, limits).unwrap();
    builder.add_import(CONFLICT_NAMESPACE).unwrap();
    let namespaces: Vec<_> = (0..shape.tiles)
        .map(|tile| format!("city/lf-cn-urban-543/t{tile:03}"))
        .collect();
    for namespace in &namespaces {
        builder.add_import(namespace).unwrap();
    }

    let mut signal_gates = Vec::new();
    let mut conflict_keys = Vec::new();
    for cell in 0..shape.cells {
        let keys = CellKeys::new(cell);
        for (approach, name) in APPROACHES.iter().enumerate() {
            for kind in ["t", "l"] {
                signal_gates.push((cell / CELLS_PER_TILE, keys.k(&format!("g-{name}-{kind}"))));
            }
            if waiting_approach(approach) {
                for kind in ["in", "out"] {
                    signal_gates.push((
                        cell / CELLS_PER_TILE,
                        keys.k(&format!("g-{name}-lw-{kind}")),
                    ));
                }
            }
        }
        for suffix in ["a", "b"] {
            conflict_keys.push((
                keys.k(&format!("x-rule-{suffix}")),
                keys.k("x-junction"),
                format!("movement-{suffix}"),
                format!("stream-{suffix}"),
            ));
        }
    }
    let gate_owners: Vec<_> = conflict_keys
        .iter()
        .map(|(_, junction, movement, _)| [junction.as_str(), movement.as_str(), "path"])
        .collect();
    let stream_owners: Vec<_> = conflict_keys
        .iter()
        .map(|(_, junction, _, _)| [junction.as_str()])
        .collect();
    let span = builder.policy_source_span();
    let source = PolicyInputSource {
        primary: &span,
        contributing: &[],
    };
    let mut gates: Vec<_> = signal_gates
        .iter()
        .map(|(tile, key)| PolicyGateRuleInput {
            rule_key: key,
            gate: OwnerQualifiedReference {
                target: ManeuverGateReference::imported(&namespaces[*tile as usize], key),
                owner_keys: &[],
            },
            participant_classes: None,
            interpretation: GateInterpretation::ProtectedGroup,
            prohibition: GateProhibition::None,
            evidence_keys: &[],
            source,
        })
        .collect();
    gates.extend(
        conflict_keys
            .iter()
            .zip(&gate_owners)
            .map(|((key, _, _, _), owners)| PolicyGateRuleInput {
                rule_key: key,
                gate: OwnerQualifiedReference {
                    target: ManeuverGateReference::imported(CONFLICT_NAMESPACE, "admission"),
                    owner_keys: owners,
                },
                participant_classes: None,
                interpretation: GateInterpretation::Uncontrolled,
                prohibition: GateProhibition::None,
                evidence_keys: &[],
                source,
            }),
    );
    let streams: Vec<_> = conflict_keys
        .iter()
        .zip(&stream_owners)
        .map(|((key, _, _, stream), owners)| PolicyStreamRuleInput {
            rule_key: key,
            stream: OwnerQualifiedReference {
                target: EntityReference::imported(CONFLICT_NAMESPACE, stream),
                owner_keys: owners,
            },
            participant_classes: None,
            priority: if stream == "stream-a" { 0 } else { 1 },
            yield_to_streams: &[],
            gap_profile_key: None,
            evidence_keys: &[],
            source,
        })
        .collect();
    builder
        .add_right_of_way_policy_set(RightOfWayPolicySetInput {
            policy_set_key: POLICY_KEY,
            regulation: RegulationIdentity {
                jurisdiction: "engineering",
                version: "capacity-install-v1",
                source: Some("repository:issue-543-capacity-install-policy"),
            },
            evidence: &[],
            gap_profiles: &[],
            stream_rules: &streams,
            gate_rules: &gates,
            source,
        })
        .unwrap();
    builder.finish().unwrap()
}

fn build_full_unit(limits: &CompileLimits, shape: Shape) -> CompilationUnit {
    let mut unit = CompilationUnitBuilder::new(limits.clone());
    for tile in 0..shape.tiles {
        unit.add_synthetic_module(build_synthetic_tile_module(limits, shape, tile))
            .unwrap();
    }
    let conflicts = build_conflict_module(limits, shape);
    add_conflict_to_unit(&mut unit, &conflicts);
    unit.add_synthetic_module(build_install_policy(limits, shape))
        .unwrap();
    unit.build().unwrap()
}

fn build_synthetic_only_unit(limits: &CompileLimits, shape: Shape) -> CompilationUnit {
    let mut unit = CompilationUnitBuilder::new(limits.clone());
    for tile in 0..shape.tiles {
        unit.add_synthetic_module(build_synthetic_tile_module(limits, shape, tile))
            .unwrap();
    }
    unit.build().unwrap()
}

fn build_conflict_only_unit(limits: &CompileLimits, shape: Shape) -> CompilationUnit {
    let mut unit = CompilationUnitBuilder::new(limits.clone());
    let conflicts = build_conflict_module(limits, shape);
    add_conflict_to_unit(&mut unit, &conflicts);
    unit.build().unwrap()
}

// ---------------------------------------------------------------------------
// 堆/耗时测量（stats_alloc + 1ms 采样线程；与既有
// crates/laneflow-compiler/tests/portable_emission_resources.rs 相同模式）。
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct Measurement {
    elapsed_ns: u128,
    stats: Stats,
    sampled_heap_peak_delta_bytes: u64,
}

impl Measurement {
    fn live_delta_bytes(self) -> i128 {
        self.stats.bytes_allocated as i128 - self.stats.bytes_deallocated as i128
    }

    fn positive_live_delta_bytes(self) -> u64 {
        u64::try_from(self.live_delta_bytes().max(0)).expect("positive live delta fits u64")
    }

    fn sampled_transient_heap_peak_bytes(self) -> u64 {
        self.sampled_heap_peak_delta_bytes
            .saturating_sub(self.positive_live_delta_bytes())
    }
}

fn allocator_live_bytes() -> u64 {
    let stats = GLOBAL.stats();
    u64::try_from(
        stats
            .bytes_allocated
            .saturating_sub(stats.bytes_deallocated),
    )
    .unwrap_or(u64::MAX)
}

fn measure<T>(operation: impl FnOnce() -> T) -> (T, Measurement) {
    let baseline = allocator_live_bytes();
    let region = Region::new(GLOBAL);
    let started = Instant::now();
    let output = operation();
    let elapsed_ns = started.elapsed().as_nanos();
    black_box(&output);
    let stats = black_box(region.change());
    let sampled_heap_peak_delta_bytes = allocator_live_bytes().saturating_sub(baseline);
    (
        output,
        Measurement {
            elapsed_ns,
            stats,
            sampled_heap_peak_delta_bytes,
        },
    )
}

fn measure_with_heap_peak<T>(operation: impl FnOnce() -> T) -> (T, Measurement) {
    struct StopSampler<'a>(&'a AtomicBool);
    impl Drop for StopSampler<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    let stop = AtomicBool::new(false);
    let peak = AtomicU64::new(0);
    thread::scope(|scope| {
        scope.spawn(|| {
            while !stop.load(Ordering::Acquire) {
                peak.fetch_max(allocator_live_bytes(), Ordering::Relaxed);
                thread::sleep(Duration::from_millis(1));
            }
            peak.fetch_max(allocator_live_bytes(), Ordering::Relaxed);
        });
        let baseline = allocator_live_bytes();
        peak.store(baseline, Ordering::Relaxed);
        let stop_sampler = StopSampler(&stop);
        let region = Region::new(GLOBAL);
        let started = Instant::now();
        let output = operation();
        let elapsed_ns = started.elapsed().as_nanos();
        black_box(&output);
        let stats = black_box(region.change());
        peak.fetch_max(allocator_live_bytes(), Ordering::Relaxed);
        drop(stop_sampler);
        (
            output,
            Measurement {
                elapsed_ns,
                stats,
                sampled_heap_peak_delta_bytes: peak
                    .load(Ordering::Relaxed)
                    .saturating_sub(baseline),
            },
        )
    })
}

fn print_measurement(stage: &str, cells: u32, measurement: &Measurement) {
    println!(
        "lf543-measure stage={stage} cells={cells} elapsed_ns={} allocations={} reallocations={} allocated_bytes={} deallocated_bytes={} reallocated_delta_bytes={} live_delta_bytes={} sampled_heap_peak_delta_bytes={} sampled_transient_heap_peak_bytes={}",
        measurement.elapsed_ns,
        measurement.stats.allocations,
        measurement.stats.reallocations,
        measurement.stats.bytes_allocated,
        measurement.stats.bytes_deallocated,
        measurement.stats.bytes_reallocated,
        measurement.live_delta_bytes(),
        measurement.sampled_heap_peak_delta_bytes,
        measurement.sampled_transient_heap_peak_bytes(),
    );
}

// ---------------------------------------------------------------------------
// 具名配置档上限快照（crates/laneflow-compiler/src/limits.rs:176-260；字段私有、
// 无公开 getter，故在此逐项镜像，供对照打印）。
// ---------------------------------------------------------------------------

struct LimitRow {
    dimension: &'static str,
    p100_v2: u64,
    net1m_v2: u64,
}

const LIMIT_ROWS: &[LimitRow] = &[
    LimitRow {
        dimension: "max_module_count",
        p100_v2: 522,
        net1m_v2: 65_536,
    },
    LimitRow {
        dimension: "max_source_document_count",
        p100_v2: 1_566,
        net1m_v2: 196_608,
    },
    LimitRow {
        dimension: "max_import_edge_count",
        p100_v2: 1_032,
        net1m_v2: 262_144,
    },
    LimitRow {
        dimension: "max_source_bytes_per_module",
        p100_v2: 542_741,
        net1m_v2: 536_870_912,
    },
    LimitRow {
        dimension: "max_source_bytes_total",
        p100_v2: 542_741,
        net1m_v2: 536_870_912,
    },
    LimitRow {
        dimension: "max_declaration_count",
        p100_v2: 11_265,
        net1m_v2: 1_500_000,
    },
    LimitRow {
        dimension: "max_stable_entity_count",
        p100_v2: 11_265,
        net1m_v2: 1_000_000,
    },
    LimitRow {
        dimension: "max_typed_ast_record_count",
        p100_v2: 58_387,
        net1m_v2: 8_000_000,
    },
    LimitRow {
        dimension: "max_hir_record_count",
        p100_v2: 58_387,
        net1m_v2: 8_000_000,
    },
    LimitRow {
        dimension: "max_mir_record_count",
        p100_v2: 38_112,
        net1m_v2: 8_000_000,
    },
    LimitRow {
        dimension: "max_lir_record_count",
        p100_v2: 38_112,
        net1m_v2: 8_000_000,
    },
    LimitRow {
        dimension: "max_reference_count",
        p100_v2: 37_920,
        net1m_v2: 16_000_000,
    },
    LimitRow {
        dimension: "max_relation_occurrence_count",
        p100_v2: 10_032,
        net1m_v2: 16_000_000,
    },
    LimitRow {
        dimension: "max_identity_field_occurrence_count",
        p100_v2: 29_184,
        net1m_v2: 8_000_000,
    },
    LimitRow {
        dimension: "max_maneuver_gate_count",
        p100_v2: 2_304,
        net1m_v2: 1_000_000,
    },
    LimitRow {
        dimension: "max_waiting_zone_count",
        p100_v2: 1_536,
        net1m_v2: 1_000_000,
    },
    LimitRow {
        dimension: "max_geometry_point_count",
        p100_v2: 22_368,
        net1m_v2: 16_000_000,
    },
    LimitRow {
        dimension: "max_symbol_count",
        p100_v2: 11_265,
        net1m_v2: 2_000_000,
    },
    LimitRow {
        dimension: "max_string_item_count",
        p100_v2: 36_894,
        net1m_v2: 8_000_000,
    },
    LimitRow {
        dimension: "max_single_string_bytes",
        p100_v2: 53,
        net1m_v2: 4_096,
    },
    LimitRow {
        dimension: "max_total_string_bytes",
        p100_v2: 991_537,
        net1m_v2: 536_870_912,
    },
    LimitRow {
        dimension: "max_stage_scratch_bytes",
        p100_v2: 304_896,
        net1m_v2: 2_147_483_648,
    },
    LimitRow {
        dimension: "max_output_bytes",
        p100_v2: 2_782_758,
        net1m_v2: 1_073_741_824,
    },
    LimitRow {
        dimension: "max_compiler_controlled_live_bytes",
        p100_v2: 43_269_120,
        net1m_v2: 6_442_450_944,
    },
    LimitRow {
        dimension: "max_retained_capacity_bytes",
        p100_v2: 36_925_688,
        net1m_v2: 536_870_912,
    },
];

// ---------------------------------------------------------------------------
// 诊断打印（probe 的失败 oracle：CompileLimitExceeded 携带 dimension/limit/observed）。
// ---------------------------------------------------------------------------

fn print_diagnostics(stage: &str, bundle: &DiagnosticBundle) {
    for diagnostic in bundle.diagnostics() {
        match diagnostic.payload() {
            DiagnosticPayload::CompileLimitExceeded {
                dimension,
                limit,
                observed,
            } => {
                println!(
                    "lf543-limit-exceeded stage={stage} dimension={dimension:?} limit={limit} observed={observed}"
                );
            }
            payload => {
                println!(
                    "lf543-diagnostic stage={stage} code={:?} stable_key={:?} payload={payload:?}",
                    diagnostic.code(),
                    diagnostic.stable_key(),
                );
            }
        }
    }
    println!(
        "lf543-diagnostic-summary stage={stage} count={} truncated={}",
        bundle.diagnostics().len(),
        bundle.diagnostics_truncated(),
    );
}

// ---------------------------------------------------------------------------
// LIR 公开锚点（ValidatedCanonicalLir 的 ExactSizeIterator 视图）。
// ---------------------------------------------------------------------------

fn print_lir_anchors(output: &laneflow_compiler::CompilationOutput, cells: u32, profile_id: &str) {
    let lir = output.lir();
    let metrics = output.metrics();
    println!(
        "lf543-metrics cells={cells} profile={profile_id} lir_record_count={} output_logical_bytes={} compiler_controlled_peak_bytes={} semantic_fingerprint={}",
        metrics.lir_record_count(),
        metrics.output_logical_bytes(),
        metrics.compiler_controlled_peak_bytes(),
        metrics
            .semantic_fingerprint()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
    );
    let successors_total: usize = lir.lane_edges().map(|e| e.successors().len()).sum();
    let lane_geometry_points_total: usize = lir
        .lane_edges()
        .filter_map(|e| e.spatial_geometry().map(|g| g.points().len()))
        .sum();
    let lane_geometry_segments_total: usize = lir
        .lane_edges()
        .filter_map(|e| e.spatial_geometry().map(|g| g.segments().len()))
        .sum();
    let corridor_elements_total: usize = lir.road_corridors().map(|c| c.elements().len()).sum();
    let junction_movements_total: usize = lir.junctions().map(|j| j.movements().len()).sum();
    let movement_paths_total: usize = lir.movements().map(|m| m.maneuver_paths().len()).sum();
    let path_edges_total: usize = lir.maneuver_paths().map(|p| p.edges().len()).sum();
    let path_gates_total: usize = lir.maneuver_paths().map(|p| p.maneuver_gates().len()).sum();
    let path_waiting_zones_total: usize =
        lir.maneuver_paths().map(|p| p.waiting_zones().len()).sum();
    let stop_line_gates_total: usize = lir.stop_lines().map(|s| s.maneuver_gates().len()).sum();
    let signal_group_gates_total: usize =
        lir.signal_groups().map(|g| g.maneuver_gates().len()).sum();
    let controller_groups_total: usize = lir
        .signal_controllers()
        .map(|c| c.signal_groups().len())
        .sum();
    let controller_phases_total: usize = lir.signal_controllers().map(|c| c.phases().len()).sum();
    let phase_states_total: usize = lir.signal_phases().map(|p| p.states().len()).sum();
    let parking_entries_total: usize = lir
        .parking_facilities()
        .map(|f| f.virtual_entries().len())
        .sum();
    let parking_exits_total: usize = lir
        .parking_facilities()
        .map(|f| f.virtual_exits().len())
        .sum();
    let access_rule_classes_total: usize = lir
        .access_rules()
        .map(|r| r.participant_classes().len())
        .sum();
    println!(
        "lf543-lir cells={cells} lane_edges={} successors_total={} lane_geometry_points_total={} lane_geometry_segments_total={} road_corridors={} corridor_elements_total={} road_sections={} authoring_lanes={} lane_groups={} facility_bands={} junctions={} junction_movements_total={} movements={} movement_maneuver_paths_total={} maneuver_paths={} maneuver_path_edges_total={} maneuver_path_gates_total={} maneuver_path_waiting_zones_total={} stop_lines={} stop_line_maneuver_gates_total={} maneuver_gates={} waiting_zones={} signal_groups={} signal_group_maneuver_gates_total={} signal_controllers={} signal_controller_groups_total={} signal_controller_phases_total={} signal_phases={} signal_phase_states_total={} parking_facilities={} parking_virtual_entries_total={} parking_virtual_exits_total={} parking_spaces={} participant_classes={} vehicle_profiles={} canonical_frames={} access_rules={} access_rule_classes_total={} junction_internal_edges={}",
        lir.lane_edges().len(),
        successors_total,
        lane_geometry_points_total,
        lane_geometry_segments_total,
        lir.road_corridors().len(),
        corridor_elements_total,
        lir.road_sections().len(),
        lir.authoring_lanes().len(),
        lir.lane_groups().len(),
        lir.facility_bands().len(),
        lir.junctions().len(),
        junction_movements_total,
        lir.movements().len(),
        movement_paths_total,
        lir.maneuver_paths().len(),
        path_edges_total,
        path_gates_total,
        path_waiting_zones_total,
        lir.stop_lines().len(),
        stop_line_gates_total,
        lir.maneuver_gates().len(),
        lir.waiting_zones().len(),
        lir.signal_groups().len(),
        signal_group_gates_total,
        lir.signal_controllers().len(),
        controller_groups_total,
        controller_phases_total,
        lir.signal_phases().len(),
        phase_states_total,
        lir.parking_facilities().len(),
        parking_entries_total,
        parking_exits_total,
        lir.parking_spaces().len(),
        lir.participant_classes().len(),
        lir.vehicle_profiles().len(),
        lir.canonical_frames().len(),
        lir.access_rules().len(),
        access_rule_classes_total,
        lir.junction_internal_edges().len(),
    );
}

// ---------------------------------------------------------------------------
// LFCA 逐表 dump（registry view × portable_object_schema 位置配对 + nested
// record-vector 求和 + chunk 目录）。
// ---------------------------------------------------------------------------

fn record_vector_len(row: RegistryCheckedRowView<'_>, tag: u16) -> u64 {
    let Some(field) = row.field_by_tag(tag) else {
        return 0;
    };
    match field.value().expect("checked record vector") {
        RegistryCheckedFieldValue::RecordVector(records) => u64::from(records.len()),
        _ => panic!("field tag {tag} must be a record vector"),
    }
}

fn print_lfca_tables(view: ValueCheckedObjectView<'_>, cells: u32) {
    let registry = view.registry_view();
    let schema = portable_object_schema(PortableObjectKind::CanonicalArtifact);
    let sections: Vec<_> = registry.sections().collect();
    assert_eq!(
        sections.len(),
        schema.sections.len(),
        "registry section 数与 schema 不一致"
    );
    for (section_view, section_schema) in sections.iter().zip(schema.sections) {
        let tables: Vec<_> = section_view.tables().collect();
        assert_eq!(
            tables.len(),
            section_schema.tables.len(),
            "section {} 的 table 数与 schema 不一致",
            section_schema.name
        );
        for (table_view, table_schema) in tables.iter().zip(section_schema.tables) {
            let mut max_chunk_rows = 0_u32;
            let mut max_chunk_bytes = 0_u64;
            for chunk in 0..table_view.chunk_count() {
                max_chunk_rows =
                    max_chunk_rows.max(table_view.chunk_row_count(chunk).expect("checked chunk"));
                max_chunk_bytes = max_chunk_bytes.max(
                    table_view
                        .chunk_exact_byte_length(chunk)
                        .expect("checked chunk"),
                );
            }
            let mut nested = String::new();
            for field in table_schema.row.fields {
                if field.field_type == PortableFieldType::RecordVector {
                    let sum: u64 = table_view
                        .rows()
                        .map(|row| record_vector_len(row, field.tag))
                        .sum();
                    nested.push_str(&format!(" nested.{}={sum}", field.name));
                }
            }
            println!(
                "lf543-lfca cells={cells} section={} section_name={} table_kind={} table={} rows={} chunks={} max_chunk_rows={} max_chunk_bytes={}{}",
                section_schema.kind,
                section_schema.name,
                table_schema.kind,
                table_schema.name,
                table_view.row_count(),
                table_view.chunk_count(),
                max_chunk_rows,
                max_chunk_bytes,
                nested,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 子命令：model / probe-p100 / parts / run
// ---------------------------------------------------------------------------

fn print_model(shape: Shape) {
    let synthetic = AdmissionCounts::synthetic_total(shape);
    let module = AdmissionCounts::module_shared();
    let _ = module;
    println!(
        "lf543-historical-model cells={} tiles={} synthetic_declarations={} synthetic_typed_ast={} synthetic_references={} synthetic_relations={} synthetic_identity_fields={} synthetic_symbols={} synthetic_gates={} synthetic_waiting_zones={} synthetic_geometry_points={}",
        shape.cells,
        shape.tiles,
        synthetic.declarations,
        synthetic.typed_ast_records,
        synthetic.references,
        synthetic.relation_occurrences,
        synthetic.identity_field_occurrences,
        synthetic.symbols,
        synthetic.maneuver_gates,
        synthetic.waiting_zones,
        synthetic.geometry_points,
    );
    print_parking_summary(shape);
    for row in LIMIT_ROWS {
        let observed = match row.dimension {
            "max_declaration_count" | "max_stable_entity_count" => synthetic.declarations,
            "max_typed_ast_record_count" => synthetic.typed_ast_records,
            "max_reference_count" => synthetic.references,
            "max_relation_occurrence_count" => synthetic.relation_occurrences,
            "max_identity_field_occurrence_count" => synthetic.identity_field_occurrences,
            "max_symbol_count" => synthetic.symbols,
            "max_maneuver_gate_count" => synthetic.maneuver_gates,
            "max_waiting_zone_count" => synthetic.waiting_zones,
            "max_geometry_point_count" => synthetic.geometry_points,
            _ => continue,
        };
        println!(
            "lf543-historical-model-limit cells={} dimension={} observed_synthetic_only={observed} p100_v2={} net1m_v2={}",
            shape.cells, row.dimension, row.p100_v2, row.net1m_v2,
        );
    }
}

/// 按 `docs/design/chinese-style-city-workload.md` §2 的停车计数块汇总本 workload 的
/// 停车声明量。`C_parking_virtual_declared` 是设施声明的虚拟容量上界之和
/// （cells×100 + tiles×1,000），不物化为 ParkingSpace/LFCA 行/Runtime slot——
/// 与名义 10k/100k 拓扑档明确区分（§3 的 10k/100k 首先描述 runtime 个体/停车容量目标，
/// 本 spike 的 100/1,000-cell 是拓扑档）。这些配方值同时被实测锚点覆盖：
/// shared `entity_counts()` 的 ParkingFacility/ParkingSpace 与 LFCA 逐表
/// nested.virtualEntries/virtualExits 与之逐项相等。
fn print_parking_summary(shape: Shape) {
    let facilities = u64::from(shape.cells) + u64::from(shape.tiles);
    let explicit_spaces = u64::from(shape.cells) * 4;
    let virtual_anchors = u64::from(shape.cells) * 2 + u64::from(shape.tiles) * 4;
    let declared_virtual = u64::from(shape.cells) * u64::from(CELL_FACILITY_VIRTUAL_CAPACITY)
        + u64::from(shape.tiles) * u64::from(GARAGE_VIRTUAL_CAPACITY);
    println!(
        "lf543-parking cells={} tiles={} n_parking_facility={facilities} n_parking_space_explicit={explicit_spaces} n_parking_virtual_anchor={virtual_anchors} c_parking_virtual_declared={declared_virtual}",
        shape.cells, shape.tiles,
    );
}

fn probe_p100_synthetic(max_tiles: u32) {
    let limits = CompileLimits::p100_initial_v2();
    println!(
        "lf543-probe front=synthetic profile={} max_tiles={max_tiles}",
        limits.profile_id()
    );
    let mut unit = CompilationUnitBuilder::new(limits.clone());
    for tile in 0..max_tiles {
        let shape = Shape {
            cells: (tile + 1) * CELLS_PER_TILE,
            tiles: tile + 1,
        };
        let module = build_synthetic_tile_module(&limits, shape, tile);
        match unit.add_synthetic_module(module) {
            Ok(_) => println!("lf543-probe front=synthetic tile={tile} admission=ok"),
            Err(bundle) => {
                println!("lf543-probe front=synthetic tile={tile} admission=REJECTED");
                print_diagnostics("admission", &bundle);
                return;
            }
        }
    }
    match unit.build() {
        Ok(unit) => {
            println!("lf543-probe front=synthetic unit-build=ok tiles={max_tiles}");
            match Compiler::new().compile(unit) {
                Ok(output) => {
                    println!(
                        "lf543-probe front=synthetic compile=ok lir_record_count={}",
                        output.metrics().lir_record_count()
                    );
                }
                Err(bundle) => {
                    println!("lf543-probe front=synthetic compile=REJECTED");
                    print_diagnostics("compile", &bundle);
                }
            }
        }
        Err(bundle) => {
            println!("lf543-probe front=synthetic unit-build=REJECTED");
            print_diagnostics("unit-build", &bundle);
        }
    }
}

fn probe_p100_conflict(max_cells: u32) {
    let limits = CompileLimits::p100_initial_v2();
    println!(
        "lf543-probe front=conflict profile={} max_cells={max_cells}",
        limits.profile_id()
    );
    for cells in 1..=max_cells {
        let shape = Shape {
            cells,
            tiles: cells.div_ceil(CELLS_PER_TILE),
        };
        let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            build_conflict_module(&limits, shape)
        }));
        let conflicts = match attempt {
            Ok(buffer) => buffer,
            Err(payload) => {
                println!(
                    "lf543-probe front=conflict cells={cells} module-build=REJECTED panic={:?}",
                    payload
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                        .unwrap_or_else(|| "<non-string panic>".to_owned())
                );
                return;
            }
        };
        let mut unit = CompilationUnitBuilder::new(limits.clone());
        let added = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            unit.add_road_editing_module(
                editing::RoadEditingModuleInput::try_new(
                    "conflicts.document",
                    conflicts.as_bytes(),
                    None,
                )
                .unwrap(),
            )
            .map(|_| ())
        }));
        match added {
            Ok(Ok(_)) => println!("lf543-probe front=conflict cells={cells} admission=ok"),
            Ok(Err(bundle)) => {
                println!("lf543-probe front=conflict cells={cells} admission=REJECTED");
                print_diagnostics("admission", &bundle);
                return;
            }
            Err(payload) => {
                println!(
                    "lf543-probe front=conflict cells={cells} admission=PANIC {:?}",
                    payload
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                        .unwrap_or_else(|| "<non-string panic>".to_owned())
                );
                return;
            }
        }
    }
    println!("lf543-probe front=conflict all-ok-through cells={max_cells}");
}

fn parts(shape: Shape) {
    let limits = CompileLimits::single_network_1m_v2();
    let (synthetic_output, m) = measure_with_heap_peak(|| {
        Compiler::new()
            .compile(build_synthetic_only_unit(&limits, shape))
            .unwrap_or_else(|d| panic!("synthetic-only compile: {d}"))
    });
    print_measurement("parts-synthetic-compile", shape.cells, &m);
    print_lir_anchors(&synthetic_output, shape.cells, limits.profile_id());
    drop(synthetic_output);
    let (conflict_output, m) = measure_with_heap_peak(|| {
        Compiler::new()
            .compile(build_conflict_only_unit(&limits, shape))
            .unwrap_or_else(|d| panic!("conflict-only compile: {d}"))
    });
    print_measurement("parts-conflict-compile", shape.cells, &m);
    print_lir_anchors(&conflict_output, shape.cells, limits.profile_id());
    drop(conflict_output);
}

fn required_shared_network_scratch_bytes(
    input: &CheckedCanonicalNetworkInput<ImmutableObjectSource>,
    spatial: SpatialBuildOption,
    retained_budget: u64,
) -> u64 {
    let limits = SharedNetworkBuildLimits::new(retained_budget, 0);
    match build_shared_network_revision(
        input.clone(),
        SharedNetworkBuildOptions::new(spatial, limits),
    ) {
        Err(BuildError::BudgetExceeded {
            structure: BuildStructure::BuilderScratch,
            required,
            limit: 0,
        }) => required,
        Err(error) => panic!("zero-scratch probe failed before the scratch budget: {error:?}"),
        Ok(_) => panic!("workload unexpectedly requires zero builder scratch bytes"),
    }
}

/// 刻意以 8 槽空世界（`vehicle_capacity` = 8、无车辆、无停车绑定）安装：本 spike 只测
/// 静态网络安装/驻留分配增量。运行层 `N_individual`/停车绑定随档缩放的实测证据归
/// #544/#545；该 install delta 不得被引用为运行层容量证据。
fn install_world(revision: std::sync::Arc<SharedNetworkRevision>) -> TrafficWorld {
    let origin = *revision.canonical_origin();
    TrafficWorld::install(
        revision,
        WorldConfig::new(8, 4, 1_024, 1_024, 1, 100),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://lf-cn-urban-543",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("source"),
        },
        543,
        WorldPolicySelection::Pinned(PolicyPin {
            policy: RightOfWayPolicySetId::from_untyped(
                laneflow_compiler::derive_canonical_stable_id_v1(
                    EntityKind::RightOfWayPolicySet,
                    POLICY_NAMESPACE,
                    POLICY_KEY,
                    &CompileLimits::single_network_1m_v2(),
                )
                .unwrap(),
            ),
        }),
    )
    .expect("install")
}

fn run(shape: Shape) {
    let limits = CompileLimits::single_network_1m_v2();
    println!(
        "lf543-run cells={} tiles={} profile={}",
        shape.cells,
        shape.tiles,
        limits.profile_id()
    );
    print_parking_summary(shape);
    let provenance = PortableEmissionProvenance::try_new("laneflow-issue-543-spike-v1").unwrap();

    let (unit, m) = measure_with_heap_peak(|| build_full_unit(&limits, shape));
    print_measurement("source-build", shape.cells, &m);

    let (compile_result, m) = measure_with_heap_peak(|| Compiler::new().compile(unit));
    print_measurement("compile", shape.cells, &m);
    let output = match compile_result {
        Ok(output) => output,
        Err(bundle) => {
            print_diagnostics("compile", &bundle);
            panic!("compile failed");
        }
    };
    print_lir_anchors(&output, shape.cells, limits.profile_id());

    let staging_directory = std::env::temp_dir().join(format!(
        "laneflow-issue-543-{}-{}",
        std::process::id(),
        shape.cells
    ));
    fs::create_dir(&staging_directory).unwrap();
    let (candidate, m) = measure_with_heap_peak(|| {
        emit_portable_candidate_to_staging(
            &output,
            &provenance,
            FormatLimits::HARD,
            PortableDiffBase::Genesis,
            &staging_directory,
        )
        .expect("file-backed candidate")
    });
    print_measurement("file-backed-emit", shape.cells, &m);
    let lfca_exact_bytes = candidate.canonical_artifact().byte_length().get();
    let lfsm_exact_bytes = candidate.source_map().byte_length().get();
    let lfsd_exact_bytes = candidate.semantic_diff().byte_length().get();
    println!(
        "lf543-artifact cells={} lfca_exact_bytes={lfca_exact_bytes} lfsm_exact_bytes={lfsm_exact_bytes} lfsd_exact_bytes={lfsd_exact_bytes} bundle_exact_bytes={}",
        shape.cells,
        lfca_exact_bytes + lfsm_exact_bytes + lfsd_exact_bytes,
    );
    drop(output);

    let (checked, m) = measure(|| {
        check_portable_candidate(candidate, FormatLimits::HARD).expect("checked bundle")
    });
    print_measurement("post-emission-check", shape.cells, &m);
    assert_eq!(m.stats.allocations, 0, "post-emission check 必须零分配");
    println!(
        "lf543-digest cells={} lfca_digest={:x} lfsm_digest={:x} lfsd_digest={:x}",
        shape.cells,
        checked.canonical_artifact_digest(),
        checked.source_map_digest(),
        checked.semantic_diff_digest(),
    );
    print_lfca_tables(checked.canonical_artifact_view(), shape.cells);

    let canonical_input = checked.canonical_network_input();
    const SHARED_BUILD_LIMITS: SharedNetworkBuildLimits =
        SharedNetworkBuildLimits::new(2 * 1024 * 1024 * 1024, 2 * 1024 * 1024 * 1024);
    let mut full_revision = None;
    for spatial in [
        SpatialBuildOption::Omit,
        SpatialBuildOption::RetainAvailable,
    ] {
        let required_scratch_bytes = required_shared_network_scratch_bytes(
            &canonical_input,
            spatial,
            SHARED_BUILD_LIMITS.max_retained_bytes(),
        );
        println!(
            "lf543-shared-scratch cells={} spatial={spatial:?} required_scratch_bytes={required_scratch_bytes}",
            shape.cells,
        );
        let (revision, m) = measure_with_heap_peak(|| {
            build_shared_network_revision(
                canonical_input.clone(),
                SharedNetworkBuildOptions::new(spatial, SHARED_BUILD_LIMITS),
            )
            .expect("shared network revision")
        });
        print_measurement(
            match spatial {
                SpatialBuildOption::Omit => "shared-network-build-headless",
                SpatialBuildOption::RetainAvailable => "shared-network-build-spatial",
            },
            shape.cells,
            &m,
        );
        println!(
            "lf543-shared cells={} spatial={spatial:?} retained_logical_bytes={}",
            shape.cells,
            revision.retained_logical_bytes(),
        );
        if spatial == SpatialBuildOption::RetainAvailable {
            full_revision = Some(revision);
        }
    }

    let revision = full_revision.expect("full spatial revision");
    let counts = revision.traffic().entity_counts();
    let mut entity_line = format!("lf543-entities cells={}", shape.cells);
    let mut stable_total = 0_u32;
    for kind in EntityKind::ALL {
        let count = counts.count(kind);
        stable_total += count;
        entity_line.push_str(&format!(" {kind:?}={count}"));
    }
    entity_line.push_str(&format!(" stable_total={stable_total}"));
    println!("{entity_line}");
    println!(
        "lf543-network-revision cells={} network_revision={:x}",
        shape.cells,
        canonical_input.network_revision().into_digest(),
    );

    let (world, m) = measure_with_heap_peak(|| install_world(revision));
    print_measurement("traffic-world-install", shape.cells, &m);
    drop(world);
    drop(canonical_input);
    fs::remove_dir_all(&staging_directory).ok();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let usage = "usage: run <cells> | model <cells> | parts <cells> | probe-p100 synthetic <tiles> | probe-p100 conflict <cells>";
    let Some(command) = args.get(1).map(String::as_str) else {
        eprintln!("{usage}");
        std::process::exit(2);
    };
    let parse_u32 = |value: Option<&String>| -> u32 {
        value.and_then(|v| v.parse().ok()).unwrap_or_else(|| {
            eprintln!("{usage}");
            std::process::exit(2);
        })
    };
    match command {
        "run" => run(Shape::for_cells(parse_u32(args.get(2)))),
        "model" => print_model(Shape::for_cells(parse_u32(args.get(2)))),
        "parts" => parts(Shape::for_cells(parse_u32(args.get(2)))),
        "probe-p100" => match args.get(2).map(String::as_str) {
            Some("synthetic") => probe_p100_synthetic(parse_u32(args.get(3))),
            Some("conflict") => probe_p100_conflict(parse_u32(args.get(3))),
            _ => {
                eprintln!("{usage}");
                std::process::exit(2);
            }
        },
        _ => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    }
}
