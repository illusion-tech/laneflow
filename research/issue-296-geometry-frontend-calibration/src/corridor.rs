//! #292 信号化走廊语义模型：从冻结 v0.10 traffic fixture 解析拓扑与 overlay 内容，
//! 并叠加 §9.2 要求的 parking / WaitingZone 扩展。它是 geometry 文档发射与 Synthetic
//! 孪生构造的单一事实源，保证两条发射路径消费逐字节相同的语义输入。

use serde_json::Value;

/// 全部 lane 的统一宽度（米）；traffic fixture 不携带宽度，按 §9.2 代表性走廊取常规值。
pub const LANE_WIDTH_METERS: f64 = 3.5;
/// 中央 median facility band 宽度（米）。
pub const MEDIAN_WIDTH_METERS: f64 = 1.0;

/// 轴对齐布局：主走廊沿 x 轴、支路沿 z 轴（y 为高程，恒为 0），坐标均为精确二进制小数。
/// 每项为 `(corridor_key, 参考线起点, 参考线终点)`；参考线方向跟随各 corridor 的
/// reference section 行车方向（main 为 w2e、side 为 n2s）。
/// 路口盒尺寸（32 m × 25 m）为 §6.1 方向档刻意放宽：最紧转向连接在两轴上都有 ≥10 m
/// 位移，近似圆弧半径 ≥10 m，使 Smooth1Deg 细分后的相邻规范弦长仍 >0.1 m。
const ROAD_LAYOUT: [(&str, [f64; 3], [f64; 3]); 7] = [
    ("corridor-main-road-0", [0.0, 0.0, 0.0], [189.5, 0.0, 0.0]),
    ("corridor-main-road-2", [221.5, 0.0, 0.0], [600.5, 0.0, 0.0]),
    ("corridor-main-road-4", [632.5, 0.0, 0.0], [822.0, 0.0, 0.0]),
    (
        "corridor-side-1-road-0",
        [199.5, 0.0, 146.0],
        [199.5, 0.0, 10.0],
    ),
    (
        "corridor-side-1-road-2",
        [199.5, 0.0, -15.0],
        [199.5, 0.0, -151.0],
    ),
    (
        "corridor-side-2-road-0",
        [610.5, 0.0, 146.0],
        [610.5, 0.0, 10.0],
    ),
    (
        "corridor-side-2-road-2",
        [610.5, 0.0, -15.0],
        [610.5, 0.0, -151.0],
    ),
];

/// parking 扩展锚定的长直行车道（middle 段外侧 w2e lane）。
pub const PARKING_EDGE: &str = "edge-main-w2e-lane-2-road-2";
/// WaitingZone 扩展锚定的两 transition 直行路径。
pub const WAITING_ZONE_PATH: &str = "path-junction-1-west-straight-lane-1-to-1";
/// WaitingZone 释放闸所在 internal edge。
pub const WAITING_ZONE_INTERNAL_EDGE: &str = "edge-junction-1-west-straight-lane-1-to-1-internal-0";
/// 复用既有 entry gate 作为 WaitingZone 的 entryGate。
pub const WAITING_ZONE_ENTRY_GATE: &str = "gate-junction-1-west-straight-lane-1-to-1";

#[derive(Clone, Debug)]
pub struct LaneModel {
    pub key: String,
    pub edge_key: String,
    pub lane_group: Option<String>,
    pub width_meters: f64,
    pub speed_mps: f64,
}

#[derive(Clone, Debug)]
pub struct SectionModel {
    pub key: String,
    pub kind_id: String,
    /// 相对本 road 参考线方向是否为 backward。
    pub backward: bool,
    pub lanes: Vec<LaneModel>,
    pub lane_groups: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct BandModel {
    pub key: String,
    pub kind_id: String,
    pub width_meters: f64,
}

#[derive(Clone, Debug)]
pub enum CorridorElementModel {
    RoadSection(String),
    FacilityBand(String),
}

#[derive(Clone, Debug)]
pub struct RoadModel {
    pub road_key: String,
    pub corridor_key: String,
    pub reference_section: String,
    pub reference_lane: String,
    pub elements: Vec<CorridorElementModel>,
    pub sections: Vec<SectionModel>,
    pub bands: Vec<BandModel>,
    pub reference_start: [f64; 3],
    pub reference_end: [f64; 3],
}

/// 路径的转向类别；决定 internal edge 发射为 line 还是 cubic Bézier。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TurnKind {
    Left,
    Straight,
    Right,
}

#[derive(Clone, Debug)]
pub struct InternalEdgeModel {
    pub key: String,
    pub speed_mps: f64,
    pub turn: TurnKind,
    /// entry 车道行车方向的轴向单位向量（轴对齐布局常量）。
    pub entry_axis: [f64; 2],
    /// exit 车道行车方向的轴向单位向量。
    pub exit_axis: [f64; 2],
    pub entry_edge: String,
    pub exit_edge: String,
}

#[derive(Clone, Debug)]
pub struct ConnectionModel {
    /// geometry 前端每 connection 声明一条 movement，键恒等于 `path_key`。
    pub path_key: String,
    pub entry_edge: String,
    pub internal_edge: String,
    pub exit_edge: String,
}

#[derive(Clone, Debug)]
pub struct JunctionModel {
    pub key: String,
    pub approach_edges: Vec<String>,
    pub connections: Vec<ConnectionModel>,
}

#[derive(Clone, Debug)]
pub struct GateModel {
    pub key: String,
    pub path: String,
    pub transition_index: u32,
    pub stop_line: String,
    /// `Some(group)` 表示 group 控制，`None` 表示无信号控制。
    pub signal_group: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SignalStateModel {
    pub group: String,
    pub aspect: String,
}

#[derive(Clone, Debug)]
pub struct SignalPhaseModel {
    pub key: String,
    pub duration_ms: u64,
    pub states: Vec<SignalStateModel>,
}

#[derive(Clone, Debug)]
pub struct ControllerModel {
    pub key: String,
    pub offset_ms: u64,
    pub groups: Vec<String>,
    pub phases: Vec<SignalPhaseModel>,
}

#[derive(Clone, Debug)]
pub struct ClassModel {
    pub key: String,
    pub extends: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ProfileModel {
    pub key: String,
    pub participant_class: String,
    pub iidm: [f64; 7],
}

#[derive(Clone, Debug)]
pub struct AccessRuleModel {
    pub key: String,
    pub target_lane_group: String,
    pub effect: String,
    pub participant_classes: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct WaitingZoneModel {
    pub key: String,
    pub path: String,
    pub entry_gate: String,
    pub release_gate: String,
    pub max_occupancy: u32,
}

/// 完整走廊语义模型（含 §9.2 parking / WaitingZone 扩展）。
#[derive(Clone, Debug)]
pub struct CorridorModel {
    pub roads: Vec<RoadModel>,
    pub junctions: Vec<JunctionModel>,
    pub internal_edges: Vec<InternalEdgeModel>,
    /// 20 条既有 stop line + 1 条 WaitingZone 扩展（key, lane_edge）。
    pub stop_lines: Vec<(String, String)>,
    pub signal_groups: Vec<String>,
    pub controllers: Vec<ControllerModel>,
    /// 32 条既有 maneuver gate + 1 条 WaitingZone release gate。
    pub gates: Vec<GateModel>,
    pub waiting_zones: Vec<WaitingZoneModel>,
    pub parking_areas: Vec<String>,
    /// (key, area, entry_progress, exit_progress)，锚定边恒为 `PARKING_EDGE`。
    pub parking_spaces: Vec<(String, String, f64, f64)>,
    pub classes: Vec<ClassModel>,
    pub profiles: Vec<ProfileModel>,
    pub access_rules: Vec<AccessRuleModel>,
    /// (key, edge_sequence)。
    pub routes: Vec<(String, Vec<String>)>,
}

fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("fixture 缺少数组字段 {key}"))
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("fixture 缺少字符串字段 {key}"))
}

fn number(value: &Value, key: &str) -> f64 {
    value
        .get(key)
        .and_then(Value::as_f64)
        .unwrap_or_else(|| panic!("fixture 缺少数值字段 {key}"))
}

fn integer(value: &Value, key: &str) -> u64 {
    value
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("fixture 缺少整数字段 {key}"))
}

/// 从 lane edge id 提取轴对齐行车方向（布局常量，与 `ROAD_LAYOUT` 一致）。
/// 返回 `[x, z]` 水平面单位向量；y（高程）分量恒为 0。
fn axis_of_edge(edge_key: &str) -> [f64; 2] {
    if edge_key.contains("-w2e-") {
        [1.0, 0.0]
    } else if edge_key.contains("-e2w-") {
        [-1.0, 0.0]
    } else if edge_key.contains("-n2s-") {
        [0.0, -1.0]
    } else if edge_key.contains("-s2n-") {
        [0.0, 1.0]
    } else {
        panic!("无法从 edge id 推断轴向：{edge_key}")
    }
}

fn turn_kind_of_path(path_key: &str) -> TurnKind {
    if path_key.contains("-left-") {
        TurnKind::Left
    } else if path_key.contains("-straight-") {
        TurnKind::Straight
    } else if path_key.contains("-right-") {
        TurnKind::Right
    } else {
        panic!("无法从 path id 推断转向类别：{path_key}")
    }
}

fn milliseconds_to_seconds(duration_ms: u64, context: &str) -> u64 {
    assert!(
        duration_ms.is_multiple_of(1000),
        "{context} 的毫秒值 {duration_ms} 不能整除为秒"
    );
    duration_ms / 1000
}

impl CorridorModel {
    /// 从冻结 v0.10 traffic fixture 解析走廊语义，并叠加 parking / WaitingZone 扩展。
    pub fn parse(traffic: &Value) -> Self {
        let edges = array(&traffic["laneGraph"], "edges");
        let edge_speed = |edge_key: &str| {
            edges
                .iter()
                .find(|edge| text(edge, "id") == edge_key)
                .map(|edge| number(edge, "speedLimit"))
                .unwrap_or_else(|| panic!("fixture 缺少 edge {edge_key}"))
        };
        let edge_length = |edge_key: &str| {
            edges
                .iter()
                .find(|edge| text(edge, "id") == edge_key)
                .map(|edge| number(edge, "length"))
                .unwrap_or_else(|| panic!("fixture 缺少 edge {edge_key}"))
        };

        let section_records = array(traffic, "roadSections");
        let group_records = array(traffic, "laneGroups");
        let band_records = array(traffic, "facilityBands");

        let mut roads = Vec::new();
        for corridor in array(traffic, "roadCorridors") {
            let corridor_key = text(corridor, "id").to_string();
            let reference_section = text(corridor, "referenceSectionId").to_string();
            let mut elements = Vec::new();
            let mut sections = Vec::new();
            let mut bands = Vec::new();
            for element in array(corridor, "elements") {
                if let Some(section_id) = element.get("sectionId").and_then(Value::as_str) {
                    elements.push(CorridorElementModel::RoadSection(section_id.to_string()));
                    let record = section_records
                        .iter()
                        .find(|record| text(record, "id") == section_id)
                        .unwrap_or_else(|| panic!("fixture 缺少 section {section_id}"));
                    let lane_groups = group_records
                        .iter()
                        .filter(|group| text(group, "roadSectionId") == section_id)
                        .map(|group| text(group, "id").to_string())
                        .collect::<Vec<_>>();
                    let lanes = array(record, "lanes")
                        .iter()
                        .enumerate()
                        .map(|(index, lane)| {
                            let edge_key = array(lane, "edgeIds")[0]
                                .as_str()
                                .expect("lane 恰好一条 edge")
                                .to_string();
                            LaneModel {
                                key: format!("{section_id}/lane/{index}"),
                                speed_mps: edge_speed(&edge_key),
                                edge_key,
                                lane_group: lane
                                    .get("laneGroupId")
                                    .and_then(Value::as_str)
                                    .map(str::to_string),
                                width_meters: LANE_WIDTH_METERS,
                            }
                        })
                        .collect();
                    sections.push(SectionModel {
                        key: section_id.to_string(),
                        kind_id: text(record, "kindId").to_string(),
                        backward: section_id != reference_section,
                        lanes,
                        lane_groups,
                    });
                } else {
                    let band_id = text(element, "bandId");
                    elements.push(CorridorElementModel::FacilityBand(band_id.to_string()));
                    let record = band_records
                        .iter()
                        .find(|record| text(record, "id") == band_id)
                        .unwrap_or_else(|| panic!("fixture 缺少 band {band_id}"));
                    bands.push(BandModel {
                        key: band_id.to_string(),
                        kind_id: text(record, "kindId").to_string(),
                        width_meters: MEDIAN_WIDTH_METERS,
                    });
                }
            }
            // 支路横断面重排（几何布局刻意偏离 fixture 的元素序，语义不变）：
            // [参考向 section, band, 反向 section]，且反向 section 内 lanes 反序，
            // 使反向 lane-0 落到参考线东侧、紧贴 band 对侧，为跨路口转向连接在两轴
            // 腾出 ≥10 m 位移（§6.1 方向档要求近似圆弧半径 ≥10 m）。
            if corridor_key.contains("-side-") {
                elements.sort_by_key(|element| match element {
                    CorridorElementModel::RoadSection(section_key) => {
                        if sections
                            .iter()
                            .any(|section| section.key == *section_key && section.backward)
                        {
                            2
                        } else {
                            0
                        }
                    }
                    CorridorElementModel::FacilityBand(_) => 1,
                });
                for section in sections.iter_mut().filter(|section| section.backward) {
                    section.lanes.reverse();
                }
            }
            let (reference_start, reference_end) = ROAD_LAYOUT
                .iter()
                .find(|(key, _, _)| *key == corridor_key)
                .map(|(_, start, end)| (*start, *end))
                .unwrap_or_else(|| panic!("布局缺少 corridor {corridor_key}"));
            let reference_lane = format!("{reference_section}/lane/0");
            let reference_edge = sections
                .iter()
                .find(|section| section.key == reference_section)
                .expect("reference section 已解析")
                .lanes
                .first()
                .expect("reference section 至少一条 lane")
                .edge_key
                .clone();
            let expected_length = reference_start
                .iter()
                .zip(reference_end.iter())
                .map(|(a, b)| (b - a).powi(2))
                .sum::<f64>()
                .sqrt();
            assert!(
                (edge_length(&reference_edge) - expected_length).abs() < 1e-9,
                "corridor {corridor_key} 参考车道长度与布局不一致"
            );
            roads.push(RoadModel {
                road_key: format!("{corridor_key}/road"),
                corridor_key,
                reference_section,
                reference_lane,
                elements,
                sections,
                bands,
                reference_start,
                reference_end,
            });
        }

        let movement_junction = |movement_key: &str| {
            array(traffic, "movements")
                .iter()
                .find(|movement| text(movement, "id") == movement_key)
                .map(|movement| text(movement, "junctionId").to_string())
                .unwrap_or_else(|| panic!("fixture 缺少 movement {movement_key}"))
        };

        let mut internal_edges = Vec::new();
        let mut junctions: Vec<JunctionModel> = Vec::new();
        for path in array(traffic, "maneuverPaths") {
            let path_key = text(path, "id").to_string();
            let movement_key = text(path, "movementId").to_string();
            let entry_edge = text(path, "entryEdgeId").to_string();
            let exit_edge = text(path, "exitEdgeId").to_string();
            let internals = array(path, "internalEdgeIds");
            assert_eq!(internals.len(), 1, "每条路径恰好一条 internal edge");
            let internal_edge = internals[0].as_str().expect("internal id").to_string();
            internal_edges.push(InternalEdgeModel {
                key: internal_edge.clone(),
                speed_mps: edge_speed(&internal_edge),
                turn: turn_kind_of_path(&path_key),
                entry_axis: axis_of_edge(&entry_edge),
                exit_axis: axis_of_edge(&exit_edge),
                entry_edge: entry_edge.clone(),
                exit_edge: exit_edge.clone(),
            });
            let junction_key = movement_junction(&movement_key);
            let junction = match junctions
                .iter_mut()
                .find(|junction| junction.key == junction_key)
            {
                Some(junction) => junction,
                None => {
                    junctions.push(JunctionModel {
                        key: junction_key,
                        approach_edges: Vec::new(),
                        connections: Vec::new(),
                    });
                    junctions.last_mut().expect("刚插入")
                }
            };
            for edge in [&entry_edge, &exit_edge] {
                if !junction.approach_edges.contains(edge) {
                    junction.approach_edges.push(edge.clone());
                }
            }
            junction.connections.push(ConnectionModel {
                path_key,
                entry_edge,
                internal_edge,
                exit_edge,
            });
        }

        let signals = &traffic["signals"];
        let mut stop_lines: Vec<(String, String)> = array(signals, "stopLines")
            .iter()
            .map(|stop| {
                (
                    text(stop, "id").to_string(),
                    text(stop, "edgeId").to_string(),
                )
            })
            .collect();
        let signal_groups = array(signals, "groups")
            .iter()
            .map(|group| text(group, "id").to_string())
            .collect();
        let controllers = array(signals, "controllers")
            .iter()
            .map(|controller| ControllerModel {
                key: text(controller, "id").to_string(),
                offset_ms: integer(controller, "offsetMs"),
                groups: array(controller, "groupIds")
                    .iter()
                    .map(|group| group.as_str().expect("group id").to_string())
                    .collect(),
                phases: array(controller, "phases")
                    .iter()
                    .map(|phase| SignalPhaseModel {
                        key: text(phase, "id").to_string(),
                        duration_ms: integer(phase, "durationMs"),
                        states: array(phase, "states")
                            .iter()
                            .map(|state| SignalStateModel {
                                group: text(state, "groupId").to_string(),
                                aspect: text(state, "aspect").to_string(),
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect();
        let mut gates: Vec<GateModel> = array(signals, "maneuverGates")
            .iter()
            .map(|gate| {
                let control = &gate["signalControl"];
                let signal_group = match text(control, "kind") {
                    "group" => Some(text(control, "groupId").to_string()),
                    "none" => None,
                    other => panic!("不支持的 signal control kind {other}"),
                };
                GateModel {
                    key: text(gate, "id").to_string(),
                    path: text(gate, "maneuverPathId").to_string(),
                    transition_index: u32::try_from(integer(gate, "transitionIndex"))
                        .expect("transitionIndex 为 u32"),
                    stop_line: text(gate, "stopLineId").to_string(),
                    signal_group,
                }
            })
            .collect();

        let classes = array(traffic, "participantClasses")
            .iter()
            .map(|class| ClassModel {
                key: text(class, "id").to_string(),
                extends: class
                    .get("extendsId")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
            .collect();
        let profiles = array(traffic, "vehicleProfiles")
            .iter()
            .map(|profile| ProfileModel {
                key: text(profile, "id").to_string(),
                participant_class: text(profile, "participantClassId").to_string(),
                iidm: [
                    number(profile, "length"),
                    number(profile, "desiredSpeed"),
                    number(profile, "minGap"),
                    number(profile, "timeHeadway"),
                    number(profile, "maxAcceleration"),
                    number(profile, "comfortableDeceleration"),
                    number(profile, "emergencyDeceleration"),
                ],
            })
            .collect();
        let access_rules = array(traffic, "accessRules")
            .iter()
            .map(|rule| {
                let target = &rule["target"];
                assert_eq!(
                    text(target, "kind"),
                    "laneGroup",
                    "本 fixture 的 access rule 均以 laneGroup 为目标"
                );
                AccessRuleModel {
                    key: text(rule, "id").to_string(),
                    target_lane_group: text(target, "id").to_string(),
                    effect: text(rule, "effect").to_string(),
                    participant_classes: array(rule, "participantClassIds")
                        .iter()
                        .map(|class| class.as_str().expect("class id").to_string())
                        .collect(),
                }
            })
            .collect();
        let routes = array(traffic, "routes")
            .iter()
            .map(|route| {
                (
                    text(route, "id").to_string(),
                    array(route, "edgeIds")
                        .iter()
                        .map(|edge| edge.as_str().expect("edge id").to_string())
                        .collect::<Vec<_>>(),
                )
            })
            .collect();

        // §9.2 扩展：一条 WaitingZone（entry gate 复用既有 transition-0 gate，release gate
        // 新增在 internal edge 的 transition 1 上）与一处带空间的 ParkingArea。
        stop_lines.push((
            "stop-line-junction-1-west-straight-1-1-internal".to_string(),
            WAITING_ZONE_INTERNAL_EDGE.to_string(),
        ));
        gates.push(GateModel {
            key: "gate-junction-1-west-straight-lane-1-to-1-release".to_string(),
            path: WAITING_ZONE_PATH.to_string(),
            transition_index: 1,
            stop_line: "stop-line-junction-1-west-straight-1-1-internal".to_string(),
            signal_group: None,
        });
        let waiting_zones = vec![WaitingZoneModel {
            key: "waiting-zone-junction-1-west-straight".to_string(),
            path: WAITING_ZONE_PATH.to_string(),
            entry_gate: WAITING_ZONE_ENTRY_GATE.to_string(),
            release_gate: "gate-junction-1-west-straight-lane-1-to-1-release".to_string(),
            max_occupancy: 2,
        }];
        let parking_areas = vec!["parking-area-main-road-2".to_string()];
        let parking_spaces = vec![(
            "parking-space-main-road-2-0".to_string(),
            "parking-area-main-road-2".to_string(),
            100.0,
            105.0,
        )];

        Self {
            roads,
            junctions,
            internal_edges,
            stop_lines,
            signal_groups,
            controllers,
            gates,
            waiting_zones,
            parking_areas,
            parking_spaces,
            classes,
            profiles,
            access_rules,
            routes,
        }
    }

    /// 全部 lane edge（road lane + internal）的键与限速，按模型声明序。
    pub fn all_edges(&self) -> impl Iterator<Item = (&str, f64)> + '_ {
        let road_edges = self.roads.iter().flat_map(|road| {
            road.sections.iter().flat_map(|section| {
                section
                    .lanes
                    .iter()
                    .map(|lane| (lane.edge_key.as_str(), lane.speed_mps))
            })
        });
        let internal = self
            .internal_edges
            .iter()
            .map(|edge| (edge.key.as_str(), edge.speed_mps));
        road_edges.chain(internal)
    }

    /// geometry 文档中 signal phase 的秒值；fixture 毫秒值必须整除。
    pub fn phase_seconds(duration_ms: u64) -> u64 {
        milliseconds_to_seconds(duration_ms, "signal phase")
    }
}
