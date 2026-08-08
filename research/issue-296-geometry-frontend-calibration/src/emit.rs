//! Geometry v1 文档的确定性字节发射：紧凑 JSON、数字最短往返表示、字段顺序固定。
//! internal edge 曲线采用两轮发射：第一轮发射 roads-only 探针文档编译并收获派生 lane
//! 端点（lane 派生不依赖 junction/overlay，探针端点与完整文档逐位一致），第二轮以收获
//! 端点生成真实 line/cubic 连接曲线。

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::corridor::{BandModel, CorridorElementModel, CorridorModel, SectionModel, TurnKind};

/// 一条 internal edge 的显式几何曲线（geometry wire 形状）。
#[derive(Clone, Debug)]
pub enum CurveSegment {
    Line {
        end: [f64; 3],
    },
    Cubic {
        control1: [f64; 3],
        control2: [f64; 3],
        end: [f64; 3],
    },
}

#[derive(Clone, Debug)]
pub struct InternalCurve {
    pub start: [f64; 3],
    pub segments: Vec<CurveSegment>,
}

/// 内部边曲线表：键为 internal laneEdgeKey。
pub type InternalCurves = BTreeMap<String, InternalCurve>;

/// 从收获的派生 lane 端点构造真实连接曲线，三种形态：
/// - 同轴无横向偏移的直行：单段 line；
/// - 同轴有横向偏移的直行（变道）：双臂沿行车轴、臂长为轴向位移 1/3 的 S 形 cubic
///   （参数速度近似线性，曲率平缓）；
/// - 转向：`直线 + 近似 90° 圆弧 cubic + 直线` 三段式。半径 R 取两轴位移分量的较
///   小者（布局保证 ≥10 m，使 Smooth1Deg 细分后规范弦长仍 >0.1 m），较长一轴的
///   剩余位移由直线段承担；圆弧 cubic 控制臂 = `(4/3)·tan(π/8)·R`（单段 cubic 直接
///   连接非等距端点会在中段产生 ~3.7 m 的小半径，违反最小弦长约束）。
pub fn connect_curves(
    model: &CorridorModel,
    endpoints: &BTreeMap<String, ([f32; 3], [f32; 3])>,
) -> InternalCurves {
    model
        .internal_edges
        .iter()
        .map(|edge| {
            let start = endpoints
                .get(&edge.entry_edge)
                .unwrap_or_else(|| panic!("缺少 entry edge {} 端点", edge.entry_edge))
                .1;
            let end = endpoints
                .get(&edge.exit_edge)
                .unwrap_or_else(|| panic!("缺少 exit edge {} 端点", edge.exit_edge))
                .0;
            let start = start.map(f64::from);
            let end = end.map(f64::from);
            let delta = [end[0] - start[0], end[2] - start[2]];
            let comp_entry = delta[0] * edge.entry_axis[0] + delta[1] * edge.entry_axis[1];
            let comp_exit = delta[0] * edge.exit_axis[0] + delta[1] * edge.exit_axis[1];
            let mut segments = Vec::new();
            match edge.turn {
                TurnKind::Straight => {
                    let lateral_squared =
                        delta[0].powi(2) + delta[1].powi(2) - comp_entry.powi(2);
                    if lateral_squared < 1e-9 {
                        segments.push(CurveSegment::Line { end });
                    } else {
                        let arm = comp_entry / 3.0;
                        segments.push(CurveSegment::Cubic {
                            control1: [
                                start[0] + edge.entry_axis[0] * arm,
                                0.0,
                                start[2] + edge.entry_axis[1] * arm,
                            ],
                            control2: [
                                end[0] - edge.exit_axis[0] * arm,
                                0.0,
                                end[2] - edge.exit_axis[1] * arm,
                            ],
                            end,
                        });
                    }
                }
                TurnKind::Left | TurnKind::Right => {
                    assert!(
                        comp_entry > 0.0 && comp_exit > 0.0,
                        "转向连接 {} 的轴向位移分量必须为正（entry {comp_entry} / exit {comp_exit}）",
                        edge.key
                    );
                    let radius = comp_entry.min(comp_exit);
                    let arm = (4.0 / 3.0) * (std::f64::consts::PI / 8.0).tan() * radius;
                    let arc_start = [
                        start[0] + edge.entry_axis[0] * (comp_entry - radius),
                        0.0,
                        start[2] + edge.entry_axis[1] * (comp_entry - radius),
                    ];
                    let arc_end = [
                        end[0] - edge.exit_axis[0] * (comp_exit - radius),
                        0.0,
                        end[2] - edge.exit_axis[1] * (comp_exit - radius),
                    ];
                    // entry/exit_axis 为 [x, z] 水平面单位向量；y（高程）恒为 0。
                    if comp_entry - radius > 1e-9 {
                        segments.push(CurveSegment::Line { end: arc_start });
                    }
                    segments.push(CurveSegment::Cubic {
                        control1: [
                            arc_start[0] + edge.entry_axis[0] * arm,
                            0.0,
                            arc_start[2] + edge.entry_axis[1] * arm,
                        ],
                        control2: [
                            arc_end[0] - edge.exit_axis[0] * arm,
                            0.0,
                            arc_end[2] - edge.exit_axis[1] * arm,
                        ],
                        end: arc_end,
                    });
                    if comp_exit - radius > 1e-9 {
                        segments.push(CurveSegment::Line { end });
                    }
                }
            }
            (edge.key.clone(), InternalCurve { start, segments })
        })
        .collect()
}

/// f64 的最短往返十进制表示；`-0.0` 规范为 `0`（station 起点位模式要求正零）。
fn num(value: f64) -> String {
    let value = if value == 0.0 { 0.0 } else { value };
    format!("{value}")
}

fn point(out: &mut String, point: &[f64; 3]) {
    let _ = write!(
        out,
        "[{},{},{}]",
        num(point[0]),
        num(point[1]),
        num(point[2])
    );
}

fn curve(out: &mut String, start: &[f64; 3], segments: &[CurveSegment]) {
    out.push_str("{\"start\":");
    point(out, start);
    out.push_str(",\"segments\":[");
    for (index, segment) in segments.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        match segment {
            CurveSegment::Line { end } => {
                out.push_str("{\"kind\":\"line\",\"end\":");
                point(out, end);
                out.push('}');
            }
            CurveSegment::Cubic {
                control1,
                control2,
                end,
            } => {
                out.push_str("{\"kind\":\"cubicBezier\",\"control1\":");
                point(out, control1);
                out.push_str(",\"control2\":");
                point(out, control2);
                out.push_str(",\"end\":");
                point(out, end);
                out.push('}');
            }
        }
    }
    out.push_str("]}");
}

fn line_curve(out: &mut String, start: &[f64; 3], end: &[f64; 3]) {
    curve(out, start, &[CurveSegment::Line { end: *end }]);
}

fn emit_section(out: &mut String, section: &SectionModel) {
    let _ = write!(
        out,
        "{{\"sectionKey\":\"{}\",\"kindId\":\"{}\",\"lanes\":[",
        section.key, section.kind_id
    );
    for (index, lane) in section.lanes.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let direction = if section.backward {
            "backward"
        } else {
            "forward"
        };
        let group = lane
            .lane_group
            .as_ref()
            .map(|group| format!(",\"laneGroupKey\":\"{group}\""))
            .unwrap_or_default();
        let _ = write!(
            out,
            "{{\"laneKey\":\"{}\",\"laneEdgeKey\":\"{}\",\"direction\":\"{direction}\",\"widthMeters\":{},\"speedLimitMetersPerSecond\":{}{group},\"successors\":[]}}",
            lane.key,
            lane.edge_key,
            num(lane.width_meters),
            num(lane.speed_mps),
        );
    }
    out.push_str("],\"laneGroups\":[");
    for (index, group) in section.lane_groups.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(out, "{{\"laneGroupKey\":\"{group}\"}}");
    }
    out.push_str("]}");
}

fn emit_band(out: &mut String, band: &BandModel) {
    let _ = write!(
        out,
        "{{\"facilityBandKey\":\"{}\",\"kindId\":\"{}\",\"widthMeters\":{}}}",
        band.key,
        band.kind_id,
        num(band.width_meters)
    );
}

/// 发射文档头（module 描述、units、frames）并打开 roads 数组。
fn emit_header(out: &mut String, namespace: &str, document_key: &str, description: &str) {
    out.push_str("{\"geometryVersion\":\"1\",\"module\":{\"namespace\":\"");
    out.push_str(namespace);
    out.push_str("\",\"documentKey\":\"");
    out.push_str(document_key);
    let _ = write!(
        out,
        "\",\"imports\":[],\"provenance\":{{\"kind\":\"direct\",\"description\":\"{description}\"}}}},"
    );
    out.push_str("\"units\":{\"distance\":\"meter\",\"angle\":\"radian\",\"speed\":\"meter-per-second\",\"time\":\"second\"},");
    out.push_str("\"frames\":[{\"frameKey\":\"frame.main\"}],\"roads\":[");
}

/// 发射 roads 数组内容（不含外层方括号）。
fn emit_roads(out: &mut String, model: &CorridorModel) {
    for (index, road) in model.roads.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"roadKey\":\"{}\",\"frame\":\"frame.main\",\"referenceLine\":",
            road.road_key
        );
        line_curve(out, &road.reference_start, &road.reference_end);
        let _ = write!(
            out,
            ",\"crossSectionSpans\":[{{\"spanKey\":\"{}/span\",\"corridorKey\":\"{}\",\"startStationMeters\":0,\"endStationMeters\":\"end\",\"referenceSectionKey\":\"{}\",\"referenceLaneKey\":\"{}\",\"elements\":[",
            road.road_key, road.corridor_key, road.reference_section, road.reference_lane
        );
        for (element_index, element) in road.elements.iter().enumerate() {
            if element_index > 0 {
                out.push(',');
            }
            match element {
                CorridorElementModel::RoadSection(section) => {
                    let _ = write!(
                        out,
                        "{{\"kind\":\"roadSection\",\"sectionKey\":\"{section}\"}}"
                    );
                }
                CorridorElementModel::FacilityBand(band) => {
                    let _ = write!(
                        out,
                        "{{\"kind\":\"facilityBand\",\"facilityBandKey\":\"{band}\"}}"
                    );
                }
            }
        }
        out.push_str("],\"roadSections\":[");
        for (section_index, section) in road.sections.iter().enumerate() {
            if section_index > 0 {
                out.push(',');
            }
            emit_section(out, section);
        }
        out.push_str("],\"facilityBands\":[");
        for (band_index, band) in road.bands.iter().enumerate() {
            if band_index > 0 {
                out.push(',');
            }
            emit_band(out, band);
        }
        out.push_str("]}]}");
    }
}

/// 发射全空 overlays 对象（11 个空数组，字段顺序与完整文档一致）。
fn emit_empty_overlays(out: &mut String) {
    out.push_str("\"signalGroups\":[],\"signalControllers\":[],\"parkingAreas\":[],\"parkingSpaces\":[],\"participantClasses\":[],\"vehicleProfiles\":[],\"accessRules\":[],\"staticRoutes\":[],\"stopLines\":[],\"maneuverGates\":[],\"waitingZones\":[]");
}

/// 发射仅含 roads 的第一轮探针文档：在连接曲线存在之前编译并收获派生 lane 端点。
pub fn emit_probe_document(
    model: &CorridorModel,
    namespace: &str,
    document_key: &str,
    description: &str,
) -> String {
    let mut out = String::with_capacity(64 * 1024);
    emit_header(&mut out, namespace, document_key, description);
    emit_roads(&mut out, model);
    out.push_str("],\"junctions\":[],\"overlays\":{");
    emit_empty_overlays(&mut out);
    out.push_str("}}");
    out
}

/// 发射单份走廊的 geometry v1 文档（紧凑单行 JSON）。
pub fn emit_corridor_document(
    model: &CorridorModel,
    namespace: &str,
    document_key: &str,
    description: &str,
    curves: &InternalCurves,
) -> String {
    let mut out = String::with_capacity(64 * 1024);
    emit_header(&mut out, namespace, document_key, description);
    emit_roads(&mut out, model);
    out.push_str("],\"junctions\":[");
    for (index, junction) in model.junctions.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"junctionKey\":\"{}\",\"approachEdges\":[",
            junction.key
        );
        for (edge_index, edge) in junction.approach_edges.iter().enumerate() {
            if edge_index > 0 {
                out.push(',');
            }
            let _ = write!(out, "\"{edge}\"");
        }
        out.push_str("],\"internalEdges\":[");
        let mut emitted: Vec<&str> = Vec::new();
        for connection in &junction.connections {
            let edge_key = connection.internal_edge.as_str();
            if emitted.contains(&edge_key) {
                continue;
            }
            emitted.push(edge_key);
            if emitted.len() > 1 {
                out.push(',');
            }
            let internal = model
                .internal_edges
                .iter()
                .find(|edge| edge.key == *edge_key)
                .expect("connection 的 internal edge 已解析");
            let internal_curve = curves
                .get(edge_key)
                .unwrap_or_else(|| panic!("缺少 internal edge {edge_key} 曲线"));
            let _ = write!(
                out,
                "{{\"laneEdgeKey\":\"{}\",\"speedLimitMetersPerSecond\":{},\"geometry\":",
                internal.key,
                num(internal.speed_mps)
            );
            curve(&mut out, &internal_curve.start, &internal_curve.segments);
            out.push('}');
        }
        out.push_str("],\"connections\":[");
        for (connection_index, connection) in junction.connections.iter().enumerate() {
            if connection_index > 0 {
                out.push(',');
            }
            let _ = write!(
                out,
                "{{\"movementKey\":\"{0}\",\"directedEntryApproachKey\":\"{0}/entry\",\"directedExitApproachKey\":\"{0}/exit\",\"maneuverPathKey\":\"{0}\",\"entryEdge\":\"{1}\",\"internalEdgeSequence\":[\"{2}\"],\"exitEdge\":\"{3}\"}}",
                connection.path_key,
                connection.entry_edge,
                connection.internal_edge,
                connection.exit_edge
            );
        }
        out.push_str("]}");
    }
    out.push_str("],\"overlays\":{");
    emit_overlays(&mut out, model);
    out.push('}');
    out
}

fn emit_overlays(out: &mut String, model: &CorridorModel) {
    out.push_str("\"signalGroups\":[");
    for (index, group) in model.signal_groups.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(out, "{{\"signalGroupKey\":\"{group}\"}}");
    }
    out.push_str("],\"signalControllers\":[");
    for (index, controller) in model.controllers.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let offset_seconds = controller.offset_ms / 1000;
        assert!(
            controller.offset_ms.is_multiple_of(1000),
            "controller offset 必须整除为秒"
        );
        let _ = write!(
            out,
            "{{\"signalControllerKey\":\"{}\",\"offsetSeconds\":{offset_seconds},\"signalGroups\":[",
            controller.key
        );
        for (group_index, group) in controller.groups.iter().enumerate() {
            if group_index > 0 {
                out.push(',');
            }
            let _ = write!(out, "\"{group}\"");
        }
        out.push_str("],\"phases\":[");
        for (phase_index, phase) in controller.phases.iter().enumerate() {
            if phase_index > 0 {
                out.push(',');
            }
            let duration_seconds = phase.duration_ms / 1000;
            assert!(
                phase.duration_ms.is_multiple_of(1000),
                "phase duration 必须整除为秒"
            );
            let _ = write!(
                out,
                "{{\"signalPhaseKey\":\"{}\",\"durationSeconds\":{duration_seconds},\"states\":[",
                phase.key
            );
            for (state_index, state) in phase.states.iter().enumerate() {
                if state_index > 0 {
                    out.push(',');
                }
                let _ = write!(
                    out,
                    "{{\"signalGroup\":\"{}\",\"aspect\":\"{}\"}}",
                    state.group, state.aspect
                );
            }
            out.push_str("]}");
        }
        out.push_str("]}");
    }
    out.push_str("],\"parkingAreas\":[");
    for (index, area) in model.parking_areas.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(out, "{{\"parkingAreaKey\":\"{area}\"}}");
    }
    out.push_str("],\"parkingSpaces\":[");
    for (index, (key, area, entry, exit)) in model.parking_spaces.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"parkingSpaceKey\":\"{key}\",\"parkingArea\":\"{area}\",\"entry\":{{\"laneEdge\":\"{}\",\"progressMeters\":{}}},\"exit\":{{\"laneEdge\":\"{}\",\"progressMeters\":{}}},\"geometry\":{{\"lateralOffsetMeters\":1.5,\"headingOffsetRadians\":0,\"lengthMeters\":6,\"widthMeters\":2.5}}}}",
            crate::corridor::PARKING_EDGE,
            num(*entry),
            crate::corridor::PARKING_EDGE,
            num(*exit),
        );
    }
    out.push_str("],\"participantClasses\":[");
    for (index, class) in model.classes.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let extends = class
            .extends
            .as_ref()
            .map(|base| format!(",\"extends\":\"{base}\""))
            .unwrap_or_default();
        let _ = write!(
            out,
            "{{\"participantClassKey\":\"{}\"{extends}}}",
            class.key
        );
    }
    out.push_str("],\"vehicleProfiles\":[");
    for (index, profile) in model.profiles.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"vehicleProfileKey\":\"{}\",\"participantClass\":\"{}\",\"iidm\":{{\"lengthMeters\":{},\"desiredSpeedMetersPerSecond\":{},\"minGapMeters\":{},\"timeHeadwaySeconds\":{},\"maxAccelerationMetersPerSecondSquared\":{},\"comfortableDecelerationMetersPerSecondSquared\":{},\"emergencyDecelerationMetersPerSecondSquared\":{}}}}}",
            profile.key,
            profile.participant_class,
            num(profile.iidm[0]),
            num(profile.iidm[1]),
            num(profile.iidm[2]),
            num(profile.iidm[3]),
            num(profile.iidm[4]),
            num(profile.iidm[5]),
            num(profile.iidm[6]),
        );
    }
    out.push_str("],\"accessRules\":[");
    for (index, rule) in model.access_rules.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"accessRuleKey\":\"{}\",\"target\":{{\"kind\":\"laneGroup\",\"laneGroup\":\"{}\"}},\"effect\":\"{}\",\"participantClasses\":[",
            rule.key, rule.target_lane_group, rule.effect
        );
        for (class_index, class) in rule.participant_classes.iter().enumerate() {
            if class_index > 0 {
                out.push(',');
            }
            let _ = write!(out, "\"{class}\"");
        }
        out.push_str("],\"priority\":0}");
    }
    out.push_str("],\"staticRoutes\":[");
    for (index, (key, edges)) in model.routes.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(out, "{{\"staticRouteKey\":\"{key}\",\"edgeSequence\":[");
        for (edge_index, edge) in edges.iter().enumerate() {
            if edge_index > 0 {
                out.push(',');
            }
            let _ = write!(out, "\"{edge}\"");
        }
        out.push_str("]}");
    }
    out.push_str("],\"stopLines\":[");
    for (index, (key, edge)) in model.stop_lines.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(out, "{{\"stopLineKey\":\"{key}\",\"laneEdge\":\"{edge}\"}}");
    }
    out.push_str("],\"maneuverGates\":[");
    for (index, gate) in model.gates.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let control = gate
            .signal_group
            .as_ref()
            .map(|group| format!("\"{group}\""))
            .unwrap_or_else(|| "null".to_string());
        let _ = write!(
            out,
            "{{\"maneuverGateKey\":\"{}\",\"maneuverPath\":\"{}\",\"transitionIndex\":{},\"stopLine\":\"{}\",\"signalControl\":{control}}}",
            gate.key, gate.path, gate.transition_index, gate.stop_line
        );
    }
    out.push_str("],\"waitingZones\":[");
    for (index, zone) in model.waiting_zones.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"waitingZoneKey\":\"{}\",\"maneuverPath\":\"{}\",\"entryGate\":\"{}\",\"releaseGate\":\"{}\",\"maxOccupancy\":{}}}",
            zone.key, zone.path, zone.entry_gate, zone.release_gate, zone.max_occupancy
        );
    }
    out.push(']');
    out.push('}');
}

/// LF-COMP-GEOMETRY-MIN-v1：单 frame、单 road、单 span、单 RoadSection、两条 lane、
/// line-only；参考线为短直线，使九种配置档组合产生逐位相同的规范点。
pub fn emit_min_document(namespace: &str, document_key: &str) -> String {
    let mut out = String::with_capacity(2048);
    out.push_str("{\"geometryVersion\":\"1\",\"module\":{\"namespace\":\"");
    out.push_str(namespace);
    out.push_str("\",\"documentKey\":\"");
    out.push_str(document_key);
    out.push_str("\",\"imports\":[],\"provenance\":{\"kind\":\"direct\",\"description\":\"#296 geometry calibration minimal workload\"}},");
    out.push_str("\"units\":{\"distance\":\"meter\",\"angle\":\"radian\",\"speed\":\"meter-per-second\",\"time\":\"second\"},");
    out.push_str("\"frames\":[{\"frameKey\":\"frame.main\"}],\"roads\":[{");
    out.push_str("\"roadKey\":\"road.main\",\"frame\":\"frame.main\",\"referenceLine\":");
    line_curve(&mut out, &[0.0, 0.0, 0.0], &[20.0, 0.0, 0.0]);
    out.push_str(",\"crossSectionSpans\":[{\"spanKey\":\"span.main\",\"corridorKey\":\"corridor.main\",\"startStationMeters\":0,\"endStationMeters\":\"end\",\"referenceSectionKey\":\"section.main\",\"referenceLaneKey\":\"lane.main.0\",\"elements\":[{\"kind\":\"roadSection\",\"sectionKey\":\"section.main\"}],\"roadSections\":[{\"sectionKey\":\"section.main\",\"kindId\":\"motorLane\",\"lanes\":[");
    out.push_str("{\"laneKey\":\"lane.main.0\",\"laneEdgeKey\":\"edge.main.0\",\"direction\":\"forward\",\"widthMeters\":3.5,\"speedLimitMetersPerSecond\":13.9,\"successors\":[]},");
    out.push_str("{\"laneKey\":\"lane.main.1\",\"laneEdgeKey\":\"edge.main.1\",\"direction\":\"forward\",\"widthMeters\":3.5,\"speedLimitMetersPerSecond\":13.9,\"successors\":[]}");
    out.push_str("],\"laneGroups\":[]}],\"facilityBands\":[]}]}],\"junctions\":[],\"overlays\":{");
    emit_empty_overlays(&mut out);
    out.push_str("}}");
    out
}
