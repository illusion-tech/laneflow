use std::collections::{HashMap, HashSet};

use laneflow_data::{SPATIAL_PACKAGE_MEDIA_TYPE, TRAFFIC_PACKAGE_MEDIA_TYPE};
use laneflow_scenario::signalized_corridor::{
    CATALOG_VERSION, CorridorCatalog, PortalCatalogEntry, PortalLaneCatalogEntry,
    RouteCatalogEntry, SpawnSlotCatalogEntry, WeightedRouteChoiceCatalogEntry,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::Error;
use crate::config::{
    CorridorConfig, ENDPOINT_CLEARANCE_METERS, MIN_GAP_METERS, MIN_SPAWN_SLOT_COUNT,
    VEHICLE_LENGTH_METERS,
};
use crate::model::{
    AccessRule, AccessTarget, ArtifactDescriptor, Centerline, CorridorElement, FacilityBand,
    Junction, LaneConnection, LaneEdge, LaneGraph, LaneGroup, ManeuverGate, ManeuverPath, Movement,
    Parking, ParticipantClass, RoadCorridor, RoadSection, Route, ScenarioManifest, SectionLane,
    SignalControl, SignalController, SignalGroup, SignalGroupState, SignalPhase, Signals,
    SpatialEdge, SpatialPackage, StopLine, TrafficPackage, Units, VehicleProfile,
};

const TRAFFIC_SCHEMA: &str = include_str!("../../../schemas/laneflow-data-v0.10.schema.json");
const SPATIAL_SCHEMA: &str = include_str!("../../../schemas/laneflow-spatial-v0.1.schema.json");
const MANIFEST_SCHEMA: &str =
    include_str!("../../../schemas/laneflow-scenario-manifest-v0.1.schema.json");
const CURVE_SEGMENT_COUNT: usize = 64;
const MIN_SPATIAL_SEGMENT_METERS: f64 = 0.1;

#[derive(Clone, Debug)]
struct CorridorBuild {
    edges: Vec<EdgeBuild>,
    road_metas: Vec<RoadEdgeMeta>,
    routes: Vec<RouteBuild>,
    connectors: Vec<ConnectorBuild>,
    stop_lines: Vec<StopLineBuild>,
}

/// 道路段 edge 的横断面元数据：与 `build_road_edges` 同序产生，
/// 供 cross-section 派生按（road, direction, segment）分组。
#[derive(Clone, Copy, Debug)]
struct RoadEdgeMeta {
    edge_index: usize,
    road: RoadClass,
    direction: RoadDirection,
    segment: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RoadClass {
    Main,
    Side(usize),
}

impl RoadClass {
    fn id_fragment(self) -> String {
        match self {
            Self::Main => "main".to_owned(),
            Self::Side(junction) => format!("side-{junction}"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RoadDirection {
    WestToEast,
    EastToWest,
    NorthToSouth,
    SouthToNorth,
}

impl RoadDirection {
    const fn id_fragment(self) -> &'static str {
        match self {
            Self::WestToEast => "w2e",
            Self::EastToWest => "e2w",
            Self::NorthToSouth => "n2s",
            Self::SouthToNorth => "s2n",
        }
    }
}

#[derive(Clone, Debug)]
struct RouteBuild {
    route: Route,
    entry_portal_id: String,
    exit_portal_id: String,
    lane_index: usize,
    weight: u64,
}

#[derive(Clone, Debug)]
struct RouteSpec {
    id: &'static str,
    entry_portal_id: &'static str,
    exit_portal_id: &'static str,
    lane_index: usize,
    weight: u64,
    occurrences: Vec<PathKey>,
}

#[derive(Clone, Debug)]
struct EdgeBuild {
    id: String,
    points: Vec<[f32; 3]>,
    speed_limit: f64,
    connections: Vec<String>,
}

#[derive(Clone, Debug)]
struct ConnectorBuild {
    key: PathKey,
    entry_edge_id: String,
    internal_edge_id: String,
    exit_edge_id: String,
    movement_id: String,
    maneuver_path_id: String,
    maneuver_gate_id: String,
    stop_line_id: String,
    signal_group_id: String,
}

#[derive(Clone, Debug)]
struct StopLineBuild {
    id: String,
    edge_id: String,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Approach {
    West,
    East,
    North,
    South,
}

impl Approach {
    const ALL: [Self; 4] = [Self::West, Self::East, Self::North, Self::South];

    const fn as_str(self) -> &'static str {
        match self {
            Self::West => "west",
            Self::East => "east",
            Self::North => "north",
            Self::South => "south",
        }
    }

    const fn is_main(self) -> bool {
        matches!(self, Self::West | Self::East)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Turn {
    Left,
    Straight,
    Right,
}

impl Turn {
    const ALL: [Self; 3] = [Self::Left, Self::Straight, Self::Right];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Straight => "straight",
            Self::Right => "right",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct PathKey {
    junction: usize,
    approach: Approach,
    turn: Turn,
    entry_lane: usize,
    exit_lane: usize,
}

#[derive(Clone, Debug)]
pub struct GeneratedScenario {
    traffic: Vec<u8>,
    spatial: Vec<u8>,
    manifest: Vec<u8>,
    catalog: Vec<u8>,
    counts: ScenarioCounts,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScenarioCounts {
    pub edges: usize,
    pub routes: usize,
    pub junctions: usize,
    pub movements: usize,
    pub maneuver_paths: usize,
    pub stop_lines: usize,
    pub maneuver_gates: usize,
    pub signal_groups: usize,
    pub controllers: usize,
    pub phases: usize,
    pub portals: usize,
    pub spawn_slots: usize,
    pub facility_bands: usize,
    pub road_sections: usize,
    pub lane_groups: usize,
    pub road_corridors: usize,
    pub access_rules: usize,
}

impl GeneratedScenario {
    pub fn traffic_bytes(&self) -> &[u8] {
        &self.traffic
    }

    pub fn spatial_bytes(&self) -> &[u8] {
        &self.spatial
    }

    pub fn manifest_bytes(&self) -> &[u8] {
        &self.manifest
    }

    pub fn catalog_bytes(&self) -> &[u8] {
        &self.catalog
    }

    pub const fn counts(&self) -> ScenarioCounts {
        self.counts
    }
}

pub fn generate(config: &CorridorConfig) -> Result<GeneratedScenario, Error> {
    config.validate()?;
    let corridor = build_corridor(config)?;
    let (traffic, spatial, catalog) = build_documents(config, &corridor)?;

    let traffic_bytes = json_bytes("TrafficPackage", &traffic)?;
    let spatial_bytes = json_bytes("SpatialPackage", &spatial)?;
    validate_schema("TrafficPackage", TRAFFIC_SCHEMA, &traffic_bytes)?;
    validate_schema("SpatialPackage", SPATIAL_SCHEMA, &spatial_bytes)?;

    let manifest = ScenarioManifest {
        format_version: "0.1",
        traffic: descriptor(
            config.output.traffic_artifact_ref.clone(),
            TRAFFIC_PACKAGE_MEDIA_TYPE,
            &traffic_bytes,
        ),
        spatial: descriptor(
            config.output.spatial_artifact_ref.clone(),
            SPATIAL_PACKAGE_MEDIA_TYPE,
            &spatial_bytes,
        ),
    };
    let manifest_bytes = json_bytes("ScenarioManifest", &manifest)?;
    validate_schema("ScenarioManifest", MANIFEST_SCHEMA, &manifest_bytes)?;

    let mut catalog_text = toml::to_string_pretty(&catalog)?;
    while catalog_text.ends_with(['\r', '\n']) {
        catalog_text.pop();
    }
    catalog_text.push('\n');
    let catalog_bytes = catalog_text.into_bytes();

    validate_catalog(&catalog, &corridor)?;

    let counts = ScenarioCounts {
        edges: traffic.lane_graph.edges.len(),
        routes: traffic.routes.len(),
        junctions: traffic.junctions.len(),
        movements: traffic.movements.len(),
        maneuver_paths: traffic.maneuver_paths.len(),
        stop_lines: traffic.signals.stop_lines.len(),
        maneuver_gates: traffic.signals.maneuver_gates.len(),
        signal_groups: traffic.signals.groups.len(),
        controllers: traffic.signals.controllers.len(),
        phases: traffic
            .signals
            .controllers
            .iter()
            .map(|controller| controller.phases.len())
            .sum(),
        portals: catalog.portals.len(),
        spawn_slots: catalog.spawn_slots.len(),
        facility_bands: traffic.facility_bands.len(),
        road_sections: traffic.road_sections.len(),
        lane_groups: traffic.lane_groups.len(),
        road_corridors: traffic.road_corridors.len(),
        access_rules: traffic.access_rules.len(),
    };
    if counts.spawn_slots < MIN_SPAWN_SLOT_COUNT {
        return Err(Error::Config(format!(
            "configuration yields {} spawn slots; at least {MIN_SPAWN_SLOT_COUNT} are required",
            counts.spawn_slots
        )));
    }

    Ok(GeneratedScenario {
        traffic: traffic_bytes,
        spatial: spatial_bytes,
        manifest: manifest_bytes,
        catalog: catalog_bytes,
        counts,
    })
}

fn build_documents(
    config: &CorridorConfig,
    corridor: &CorridorBuild,
) -> Result<(TrafficPackage, SpatialPackage, CorridorCatalog), Error> {
    let mut lane_edges = Vec::new();
    let mut spatial_edges = Vec::new();
    let mut maneuver_paths = Vec::new();
    let mut maneuver_gates = Vec::new();

    for edge in &corridor.edges {
        lane_edges.push(LaneEdge {
            id: edge.id.clone(),
            length: edge.length(),
            speed_limit: edge.speed_limit,
            connections: edge
                .connections
                .iter()
                .map(|to_edge_id| LaneConnection {
                    to_edge_id: to_edge_id.clone(),
                })
                .collect(),
        });
        spatial_edges.push(SpatialEdge {
            traffic_edge_id: edge.id.clone(),
            centerline: Centerline {
                points: edge.points.iter().copied().map(point_f64).collect(),
            },
        });
    }
    for connector in &corridor.connectors {
        maneuver_paths.push(ManeuverPath {
            id: connector.maneuver_path_id.clone(),
            movement_id: connector.movement_id.clone(),
            entry_edge_id: connector.entry_edge_id.clone(),
            internal_edge_ids: vec![connector.internal_edge_id.clone()],
            exit_edge_id: connector.exit_edge_id.clone(),
        });
        maneuver_gates.push(ManeuverGate {
            id: connector.maneuver_gate_id.clone(),
            maneuver_path_id: connector.maneuver_path_id.clone(),
            transition_index: 0,
            stop_line_id: connector.stop_line_id.clone(),
            signal_control: SignalControl {
                kind: "group",
                group_id: connector.signal_group_id.clone(),
            },
        });
    }

    let controllers = (0..2)
        .map(|index| signal_controller(config, index))
        .collect::<Vec<_>>();
    let cross_section = build_cross_section(corridor);
    let signals = Signals {
        stop_lines: corridor
            .stop_lines
            .iter()
            .map(|stop_line| StopLine {
                id: stop_line.id.clone(),
                edge_id: stop_line.edge_id.clone(),
                location: "edgeEnd",
            })
            .collect(),
        maneuver_gates,
        groups: (1..=2)
            .flat_map(|junction| {
                [
                    "main-left",
                    "main-through-right",
                    "secondary-left",
                    "secondary-through-right",
                ]
                .map(|suffix| SignalGroup {
                    id: format!("signal-group-junction-{junction}-{suffix}"),
                })
            })
            .collect(),
        controllers,
    };

    let traffic = TrafficPackage {
        format_version: "0.10",
        units: Units {
            distance: "meter",
            time: "second",
        },
        lane_graph: LaneGraph { edges: lane_edges },
        junctions: (1..=2)
            .map(|junction| Junction {
                id: format!("junction-{junction}"),
            })
            .collect(),
        movements: (1..=2)
            .flat_map(|junction| {
                Approach::ALL.into_iter().flat_map(move |approach| {
                    Turn::ALL.into_iter().map(move |turn| Movement {
                        id: movement_id(junction, approach, turn),
                        junction_id: format!("junction-{junction}"),
                    })
                })
            })
            .collect(),
        maneuver_paths,
        routes: corridor
            .routes
            .iter()
            .map(|item| item.route.clone())
            .collect(),
        vehicle_profiles: vec![
            VehicleProfile {
                id: "passenger-car",
                length: VEHICLE_LENGTH_METERS,
                model: "iidm",
                desired_speed: 20.0,
                min_gap: MIN_GAP_METERS,
                time_headway: 1.5,
                max_acceleration: 1.5,
                comfortable_deceleration: 2.0,
                emergency_deceleration: 6.0,
                participant_class_id: "car",
            },
            VehicleProfile {
                id: "shuttle-bus",
                length: 12.0,
                model: "iidm",
                desired_speed: 15.0,
                min_gap: 3.0,
                time_headway: 2.0,
                max_acceleration: 1.0,
                comfortable_deceleration: 1.5,
                emergency_deceleration: 5.0,
                participant_class_id: "bus",
            },
        ],
        participant_classes: vec![
            ParticipantClass {
                id: "motorVehicle",
                extends_id: None,
            },
            ParticipantClass {
                id: "car",
                extends_id: Some("motorVehicle"),
            },
            ParticipantClass {
                id: "bus",
                extends_id: Some("motorVehicle"),
            },
        ],
        facility_bands: cross_section.facility_bands,
        road_sections: cross_section.road_sections,
        lane_groups: cross_section.lane_groups,
        road_corridors: cross_section.road_corridors,
        access_rules: cross_section.access_rules,
        waiting_zones: Vec::new(),
        signals,
        parking: Parking {
            areas: Vec::new(),
            spaces: Vec::new(),
        },
    };
    let spatial = SpatialPackage {
        format_version: "0.1",
        frame_id: config.frame_id.clone(),
        edges: spatial_edges,
    };
    let catalog = build_catalog(config, corridor)?;
    Ok((traffic, spatial, catalog))
}

fn build_corridor(config: &CorridorConfig) -> Result<CorridorBuild, Error> {
    let (mut edges, road_metas) = build_road_edges(config);
    let mut edge_index_by_id = edges
        .iter()
        .enumerate()
        .map(|(index, edge)| (edge.id.clone(), index))
        .collect::<HashMap<_, _>>();
    let stop_lines = build_stop_lines();
    let path_keys = path_keys();
    let mut connectors = Vec::with_capacity(path_keys.len());

    for key in path_keys {
        let entry_edge_id = entry_edge_id(key);
        let exit_edge_id = exit_edge_id(key);
        let entry_index = *edge_index_by_id
            .get(&entry_edge_id)
            .expect("protected-turning entry road edge exists");
        let exit_index = *edge_index_by_id
            .get(&exit_edge_id)
            .expect("protected-turning exit road edge exists");
        let start = edges[entry_index].end();
        let end = edges[exit_index].start();
        let points = connector_points(key, start, end)?;
        let internal_edge_id = internal_edge_id(key);
        let internal_index = edges.len();

        edges[entry_index]
            .connections
            .push(internal_edge_id.clone());
        edges.push(EdgeBuild {
            id: internal_edge_id.clone(),
            points,
            speed_limit: connector_speed(config, key),
            connections: vec![exit_edge_id.clone()],
        });
        edge_index_by_id.insert(internal_edge_id.clone(), internal_index);
        connectors.push(ConnectorBuild {
            key,
            entry_edge_id,
            internal_edge_id,
            exit_edge_id,
            movement_id: movement_id(key.junction, key.approach, key.turn),
            maneuver_path_id: maneuver_path_id(key),
            maneuver_gate_id: maneuver_gate_id(key),
            stop_line_id: stop_line_id(key.junction, key.approach, key.entry_lane),
            signal_group_id: signal_group_id(key),
        });
    }

    let routes = build_routes(&connectors)?;
    if edges.len() != 66 || routes.len() != 28 || connectors.len() != 32 || stop_lines.len() != 20 {
        return Err(Error::Config(format!(
            "protected-turning topology count mismatch: {} edges, {} routes, {} paths, {} stop lines",
            edges.len(),
            routes.len(),
            connectors.len(),
            stop_lines.len()
        )));
    }

    Ok(CorridorBuild {
        edges,
        road_metas,
        routes,
        connectors,
        stop_lines,
    })
}

fn build_road_edges(config: &CorridorConfig) -> (Vec<EdgeBuild>, Vec<RoadEdgeMeta>) {
    let main_speed =
        kilometers_per_hour_to_meters_per_second(config.speed_limits.main_kilometers_per_hour);
    let secondary_speed =
        kilometers_per_hour_to_meters_per_second(config.speed_limits.secondary_kilometers_per_hour);
    let lane_width = config.geometry.lane_width_meters as f32;
    let main_half = (config.geometry.main_length_meters / 2.0) as f32;
    let [junction_1, junction_2] = config
        .geometry
        .intersection_x_meters
        .map(|value| value as f32);
    let main_connector_half = lane_width * 3.0;
    let secondary_connector_half = lane_width * 4.0;
    let mut edges = Vec::with_capacity(34);
    let mut metas = Vec::with_capacity(34);

    for lane in 0..3 {
        let z = (lane as f32 + 0.5) * lane_width;
        for (road, start, end) in [
            (0, -main_half, junction_1 - main_connector_half),
            (
                2,
                junction_1 + main_connector_half,
                junction_2 - main_connector_half,
            ),
            (4, junction_2 + main_connector_half, main_half),
        ] {
            metas.push(RoadEdgeMeta {
                edge_index: edges.len(),
                road: RoadClass::Main,
                direction: RoadDirection::WestToEast,
                segment: road,
            });
            edges.push(road_edge(
                format!("edge-main-w2e-lane-{lane}-road-{road}"),
                [start, 0.0, z],
                [end, 0.0, z],
                main_speed,
            ));
        }
    }
    for lane in 0..3 {
        let z = -(lane as f32 + 0.5) * lane_width;
        for (road, start, end) in [
            (0, main_half, junction_2 + main_connector_half),
            (
                2,
                junction_2 - main_connector_half,
                junction_1 + main_connector_half,
            ),
            (4, junction_1 - main_connector_half, -main_half),
        ] {
            metas.push(RoadEdgeMeta {
                edge_index: edges.len(),
                road: RoadClass::Main,
                direction: RoadDirection::EastToWest,
                segment: road,
            });
            edges.push(road_edge(
                format!("edge-main-e2w-lane-{lane}-road-{road}"),
                [start, 0.0, z],
                [end, 0.0, z],
                main_speed,
            ));
        }
    }
    for (junction_index, junction_x) in [junction_1, junction_2].into_iter().enumerate() {
        let junction = junction_index + 1;
        let half_length = (config.geometry.secondary_lengths_meters[junction_index] / 2.0) as f32;
        for lane in 0..2 {
            let x = junction_x - (lane as f32 + 0.5) * lane_width;
            for (road, start, end) in [
                (0, -half_length, -secondary_connector_half),
                (2, secondary_connector_half, half_length),
            ] {
                metas.push(RoadEdgeMeta {
                    edge_index: edges.len(),
                    road: RoadClass::Side(junction),
                    direction: RoadDirection::NorthToSouth,
                    segment: road,
                });
                edges.push(road_edge(
                    format!("edge-side-{junction}-n2s-lane-{lane}-road-{road}"),
                    [x, 0.0, start],
                    [x, 0.0, end],
                    secondary_speed,
                ));
            }
        }
        for lane in 0..2 {
            let x = junction_x + (lane as f32 + 0.5) * lane_width;
            for (road, start, end) in [
                (0, half_length, secondary_connector_half),
                (2, -secondary_connector_half, -half_length),
            ] {
                metas.push(RoadEdgeMeta {
                    edge_index: edges.len(),
                    road: RoadClass::Side(junction),
                    direction: RoadDirection::SouthToNorth,
                    segment: road,
                });
                edges.push(road_edge(
                    format!("edge-side-{junction}-s2n-lane-{lane}-road-{road}"),
                    [x, 0.0, start],
                    [x, 0.0, end],
                    secondary_speed,
                ));
            }
        }
    }
    (edges, metas)
}

fn road_edge(id: String, start: [f32; 3], end: [f32; 3], speed_limit: f64) -> EdgeBuild {
    EdgeBuild {
        id,
        points: vec![start, end],
        speed_limit,
        connections: Vec::new(),
    }
}

/// 由 corridor 拓扑显式派生的横断面声明（SSOT §3：lane index 按 corridor
/// reference 方向从左到右；同一 corridor 的元素按 road 分段保持纵向共延伸）。
struct CrossSectionDocs {
    facility_bands: Vec<FacilityBand>,
    road_sections: Vec<RoadSection>,
    lane_groups: Vec<LaneGroup>,
    road_corridors: Vec<RoadCorridor>,
    access_rules: Vec<AccessRule>,
}

fn build_cross_section(corridor: &CorridorBuild) -> CrossSectionDocs {
    // 物理 corridor 单元 = 一条 road 分段（junction 之间/之外）：主干道三段 +
    // 每条支路两段。reference 方向：主干道取 w2e，支路取 n2s。
    // segment 是按方向 traversal order 编号而非物理区间：w2e road-0 是最西段，
    // 而 e2w road-0 是最东段（build_road_edges），因此同一物理单元的对向
    // segment 键必须反转（主干 4−segment、支路 2−segment），否则 corridor 会把
    // 几何上互不相交的两个方向 section 拼在一起，违反纵向共延伸不变量。
    let mut units = Vec::with_capacity(7);
    for segment in [0, 2, 4] {
        units.push((
            RoadClass::Main,
            segment,
            4 - segment,
            RoadDirection::WestToEast,
            RoadDirection::EastToWest,
        ));
    }
    for junction in 1..=2 {
        for segment in [0, 2] {
            units.push((
                RoadClass::Side(junction),
                segment,
                2 - segment,
                RoadDirection::NorthToSouth,
                RoadDirection::SouthToNorth,
            ));
        }
    }

    let mut docs = CrossSectionDocs {
        facility_bands: Vec::new(),
        road_sections: Vec::new(),
        lane_groups: Vec::new(),
        road_corridors: Vec::new(),
        access_rules: Vec::new(),
    };
    for (road, segment, opposite_segment, reference, opposite) in units {
        let road_fragment = road.id_fragment();
        let metas_of = |direction: RoadDirection, direction_segment: usize| {
            corridor
                .road_metas
                .iter()
                .filter(|meta| {
                    meta.road == road
                        && meta.direction == direction
                        && meta.segment == direction_segment
                })
                .collect::<Vec<_>>()
        };
        let reference_metas = metas_of(reference, segment);
        let opposite_metas = metas_of(opposite, opposite_segment);

        // 横向顺序从几何派生：reference 方向切向量 T 与左方向 L = up × T
        // （对齐 spatial-geometry §7 的正横向偏移约定），lane/元素的横向坐标是
        // 其 edge 起点在 L 上的投影；"从左到右"即按投影降序排列。
        let reference_edge = &corridor.edges[reference_metas[0].edge_index];
        let origin = point_f64(reference_edge.start());
        let tangent = normalize3(sub3(point_f64(reference_edge.end()), origin));
        let left = cross3([0.0, 1.0, 0.0], tangent);
        let lateral = |meta: &RoadEdgeMeta| {
            let start = point_f64(corridor.edges[meta.edge_index].start());
            dot3(left, sub3(start, origin))
        };
        let mean_lateral = |metas: &[&RoadEdgeMeta]| {
            metas.iter().map(|meta| lateral(meta)).sum::<f64>() / metas.len() as f64
        };
        let band_lateral = mean_lateral(
            &reference_metas
                .iter()
                .chain(opposite_metas.iter())
                .copied()
                .collect::<Vec<_>>(),
        );

        // 每个方向一个 RoadSection；lanes 按 corridor reference 系从左到右
        // （lateral 降序）。主干道 section 的路缘侧 lane（离中央分隔带横向距离
        // 最远）划为公交专用道 LaneGroup。ID 的 road 键沿用各方向自己的
        // traversal segment 编号（与 edge ID 一致）。
        let mut section_ids = [String::new(), String::new()];
        let mut group_ids = [None, None];
        for (slot, (direction, direction_segment, metas)) in [
            (reference, segment, &reference_metas),
            (opposite, opposite_segment, &opposite_metas),
        ]
        .into_iter()
        .enumerate()
        {
            let mut ordered = metas.clone();
            ordered.sort_by(|left_meta, right_meta| {
                lateral(right_meta).total_cmp(&lateral(left_meta))
            });
            let section_id = format!(
                "section-{road_fragment}-{}-road-{direction_segment}",
                direction.id_fragment()
            );
            let group_id = (road == RoadClass::Main).then(|| {
                format!(
                    "group-{road_fragment}-{}-bus-road-{direction_segment}",
                    direction.id_fragment()
                )
            });
            let bus_lane_position = group_id.as_ref().map(|_| {
                ordered
                    .iter()
                    .enumerate()
                    .max_by(|(_, left_meta), (_, right_meta)| {
                        (lateral(left_meta) - band_lateral)
                            .abs()
                            .total_cmp(&(lateral(right_meta) - band_lateral).abs())
                    })
                    .map(|(index, _)| index)
                    .expect("section has at least one lane")
            });
            let lanes = ordered
                .iter()
                .enumerate()
                .map(|(index, meta)| SectionLane {
                    edge_ids: vec![corridor.edges[meta.edge_index].id.clone()],
                    lane_group_id: (Some(index) == bus_lane_position)
                        .then(|| group_id.clone().expect("bus lane implies group")),
                })
                .collect();
            if let Some(group_id) = &group_id {
                docs.lane_groups.push(LaneGroup {
                    id: group_id.clone(),
                    road_section_id: section_id.clone(),
                });
            }
            docs.road_sections.push(RoadSection {
                id: section_id.clone(),
                kind_id: "motorLane",
                lanes,
            });
            section_ids[slot] = section_id;
            group_ids[slot] = group_id;
        }

        let band_id = format!("band-{road_fragment}-median-road-{segment}");
        docs.facility_bands.push(FacilityBand {
            id: band_id.clone(),
            kind_id: "median",
        });

        // corridor elements 与 lanes 同序派生（reference 系从左到右）。
        let mut elements = [
            (
                mean_lateral(&reference_metas),
                CorridorElement::Section {
                    section_id: section_ids[0].clone(),
                },
            ),
            (
                mean_lateral(&opposite_metas),
                CorridorElement::Section {
                    section_id: section_ids[1].clone(),
                },
            ),
            (
                band_lateral,
                CorridorElement::Band {
                    band_id: band_id.clone(),
                },
            ),
        ];
        elements
            .sort_by(|(left_lateral, _), (right_lateral, _)| right_lateral.total_cmp(left_lateral));
        docs.road_corridors.push(RoadCorridor {
            id: format!("corridor-{road_fragment}-road-{segment}"),
            reference_section_id: section_ids[0].clone(),
            elements: elements.into_iter().map(|(_, element)| element).collect(),
        });

        // 公交专用道组合（SSOT §6.4 范例）：deny motorVehicle + allow bus。
        // 本 corridor 演示车队的 population 全部是 passenger-car，附加显式
        // allow car 豁免使演示车队保持合法；规则即意图，豁免因此可审计。
        for (direction, direction_segment, group_id) in [
            (reference, segment, &group_ids[0]),
            (opposite, opposite_segment, &group_ids[1]),
        ] {
            let Some(group_id) = group_id else { continue };
            let prefix = format!(
                "rule-{road_fragment}-{}-bus-road-{direction_segment}",
                direction.id_fragment()
            );
            for (suffix, effect, classes) in [
                ("deny-motor-vehicle", "deny", vec!["motorVehicle"]),
                ("allow-bus", "allow", vec!["bus"]),
                ("allow-car", "allow", vec!["car"]),
            ] {
                docs.access_rules.push(AccessRule {
                    id: format!("{prefix}-{suffix}"),
                    target: AccessTarget {
                        kind: "laneGroup",
                        id: group_id.clone(),
                    },
                    effect,
                    participant_class_ids: classes,
                });
            }
        }
    }
    docs
}

fn sub3(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn dot3(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn cross3(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn normalize3(vector: [f64; 3]) -> [f64; 3] {
    let length = dot3(vector, vector).sqrt();
    vector.map(|component| component / length)
}

fn path_keys() -> Vec<PathKey> {
    let mut keys = Vec::with_capacity(32);
    for junction in 1..=2 {
        for approach in Approach::ALL {
            let assignments: &[(Turn, usize, usize)] = if approach.is_main() {
                &[
                    (Turn::Left, 0, 0),
                    (Turn::Straight, 1, 0),
                    (Turn::Straight, 1, 1),
                    (Turn::Straight, 2, 2),
                    (Turn::Right, 2, 1),
                ]
            } else {
                &[
                    (Turn::Left, 0, 0),
                    (Turn::Straight, 1, 1),
                    (Turn::Right, 1, 2),
                ]
            };
            keys.extend(
                assignments
                    .iter()
                    .map(|&(turn, entry_lane, exit_lane)| PathKey {
                        junction,
                        approach,
                        turn,
                        entry_lane,
                        exit_lane,
                    }),
            );
        }
    }
    keys
}

fn build_stop_lines() -> Vec<StopLineBuild> {
    (1..=2)
        .flat_map(|junction| {
            Approach::ALL.into_iter().flat_map(move |approach| {
                let lane_count = if approach.is_main() { 3 } else { 2 };
                (0..lane_count).map(move |lane| StopLineBuild {
                    id: stop_line_id(junction, approach, lane),
                    edge_id: entry_edge_id(PathKey {
                        junction,
                        approach,
                        turn: Turn::Straight,
                        entry_lane: lane,
                        exit_lane: lane,
                    }),
                })
            })
        })
        .collect()
}

fn entry_edge_id(key: PathKey) -> String {
    match key.approach {
        Approach::West => format!(
            "edge-main-w2e-lane-{}-road-{}",
            key.entry_lane,
            if key.junction == 1 { 0 } else { 2 }
        ),
        Approach::East => format!(
            "edge-main-e2w-lane-{}-road-{}",
            key.entry_lane,
            if key.junction == 2 { 0 } else { 2 }
        ),
        Approach::North => format!(
            "edge-side-{}-n2s-lane-{}-road-0",
            key.junction, key.entry_lane
        ),
        Approach::South => format!(
            "edge-side-{}-s2n-lane-{}-road-0",
            key.junction, key.entry_lane
        ),
    }
}

fn exit_edge_id(key: PathKey) -> String {
    match (key.approach, key.turn) {
        (Approach::West, Turn::Left) => {
            format!(
                "edge-side-{}-s2n-lane-{}-road-2",
                key.junction, key.exit_lane
            )
        }
        (Approach::West, Turn::Straight) => format!(
            "edge-main-w2e-lane-{}-road-{}",
            key.exit_lane,
            if key.junction == 1 { 2 } else { 4 }
        ),
        (Approach::West, Turn::Right) => {
            format!(
                "edge-side-{}-n2s-lane-{}-road-2",
                key.junction, key.exit_lane
            )
        }
        (Approach::East, Turn::Left) => {
            format!(
                "edge-side-{}-n2s-lane-{}-road-2",
                key.junction, key.exit_lane
            )
        }
        (Approach::East, Turn::Straight) => format!(
            "edge-main-e2w-lane-{}-road-{}",
            key.exit_lane,
            if key.junction == 2 { 2 } else { 4 }
        ),
        (Approach::East, Turn::Right) => {
            format!(
                "edge-side-{}-s2n-lane-{}-road-2",
                key.junction, key.exit_lane
            )
        }
        (Approach::North, Turn::Left) => format!(
            "edge-main-w2e-lane-{}-road-{}",
            key.exit_lane,
            if key.junction == 1 { 2 } else { 4 }
        ),
        (Approach::North, Turn::Straight) => format!(
            "edge-side-{}-n2s-lane-{}-road-2",
            key.junction, key.exit_lane
        ),
        (Approach::North, Turn::Right) => format!(
            "edge-main-e2w-lane-{}-road-{}",
            key.exit_lane,
            if key.junction == 1 { 4 } else { 2 }
        ),
        (Approach::South, Turn::Left) => format!(
            "edge-main-e2w-lane-{}-road-{}",
            key.exit_lane,
            if key.junction == 1 { 4 } else { 2 }
        ),
        (Approach::South, Turn::Straight) => format!(
            "edge-side-{}-s2n-lane-{}-road-2",
            key.junction, key.exit_lane
        ),
        (Approach::South, Turn::Right) => format!(
            "edge-main-w2e-lane-{}-road-{}",
            key.exit_lane,
            if key.junction == 1 { 2 } else { 4 }
        ),
    }
}

fn connector_points(key: PathKey, start: [f32; 3], end: [f32; 3]) -> Result<Vec<[f32; 3]>, Error> {
    let mut points = if key.turn == Turn::Straight && key.entry_lane == key.exit_lane {
        vec![start, end]
    } else {
        (0..=CURVE_SEGMENT_COUNT)
            .map(|index| {
                let t = index as f64 / CURVE_SEGMENT_COUNT as f64;
                let [start_x, _, start_z] = point_f64(start);
                let [end_x, _, end_z] = point_f64(end);
                let (x, z) = match key.turn {
                    Turn::Straight => {
                        let smooth = t * t * (3.0 - 2.0 * t);
                        if key.approach.is_main() {
                            (
                                start_x + (end_x - start_x) * t,
                                start_z + (end_z - start_z) * smooth,
                            )
                        } else {
                            (
                                start_x + (end_x - start_x) * smooth,
                                start_z + (end_z - start_z) * t,
                            )
                        }
                    }
                    Turn::Left | Turn::Right => {
                        let angle = std::f64::consts::FRAC_PI_2 * t;
                        if key.approach.is_main() {
                            (
                                start_x + (end_x - start_x) * angle.sin(),
                                end_z + (start_z - end_z) * angle.cos(),
                            )
                        } else {
                            (
                                end_x + (start_x - end_x) * angle.cos(),
                                start_z + (end_z - start_z) * angle.sin(),
                            )
                        }
                    }
                };
                quantized_point(x, z)
            })
            .collect()
    };
    points[0] = start;
    *points.last_mut().expect("connector has an endpoint") = end;
    let minimum_segment = points
        .windows(2)
        .map(|pair| edge_length(pair[0], pair[1]))
        .fold(f64::INFINITY, f64::min);
    if minimum_segment < MIN_SPATIAL_SEGMENT_METERS {
        return Err(Error::Config(format!(
            "{} has a {minimum_segment:.6} m segment below the {:.3} m spatial minimum",
            internal_edge_id(key),
            MIN_SPATIAL_SEGMENT_METERS
        )));
    }
    Ok(points)
}

fn quantized_point(x: f64, z: f64) -> [f32; 3] {
    [
        ((x * 1_000.0).round() / 1_000.0) as f32,
        0.0,
        ((z * 1_000.0).round() / 1_000.0) as f32,
    ]
}

fn connector_speed(config: &CorridorConfig, key: PathKey) -> f64 {
    let kilometers_per_hour = match key.turn {
        Turn::Left => config.speed_limits.left_kilometers_per_hour,
        Turn::Right => config.speed_limits.right_kilometers_per_hour,
        Turn::Straight if key.approach.is_main() => config.speed_limits.main_kilometers_per_hour,
        Turn::Straight => config.speed_limits.secondary_kilometers_per_hour,
    };
    kilometers_per_hour_to_meters_per_second(kilometers_per_hour)
}

fn build_routes(connectors: &[ConnectorBuild]) -> Result<Vec<RouteBuild>, Error> {
    let connector_by_key = connectors
        .iter()
        .map(|connector| (connector.key, connector))
        .collect::<HashMap<_, _>>();
    let specs = route_specs();
    let occurrence_count = specs
        .iter()
        .map(|spec| spec.occurrences.len())
        .sum::<usize>();
    let coverage = specs
        .iter()
        .flat_map(|spec| spec.occurrences.iter().copied())
        .collect::<HashSet<_>>();
    if specs.len() != 28 || occurrence_count != 44 || coverage.len() != 32 {
        return Err(Error::Config(format!(
            "protected-turning route table mismatch: {} routes, {occurrence_count} occurrences, {} covered paths",
            specs.len(),
            coverage.len()
        )));
    }

    specs
        .into_iter()
        .map(|spec| {
            let mut edge_ids = Vec::with_capacity(spec.occurrences.len() * 2 + 1);
            for key in spec.occurrences {
                let connector = connector_by_key
                    .get(&key)
                    .expect("route table only references declared maneuver paths");
                if let Some(previous_exit) = edge_ids.last() {
                    if previous_exit != &connector.entry_edge_id {
                        return Err(Error::Config(format!(
                            "route {:?} is discontinuous between {:?} and {:?}",
                            spec.id, previous_exit, connector.entry_edge_id
                        )));
                    }
                } else {
                    edge_ids.push(connector.entry_edge_id.clone());
                }
                edge_ids.push(connector.internal_edge_id.clone());
                edge_ids.push(connector.exit_edge_id.clone());
            }
            Ok(RouteBuild {
                route: Route {
                    id: spec.id.to_owned(),
                    edge_ids,
                },
                entry_portal_id: spec.entry_portal_id.to_owned(),
                exit_portal_id: spec.exit_portal_id.to_owned(),
                lane_index: spec.lane_index,
                weight: spec.weight,
            })
        })
        .collect()
}

#[allow(clippy::too_many_lines)]
fn route_specs() -> Vec<RouteSpec> {
    let key = |junction, approach, turn, entry_lane, exit_lane| PathKey {
        junction,
        approach,
        turn,
        entry_lane,
        exit_lane,
    };
    let route = |id, entry_portal_id, exit_portal_id, lane_index, weight, occurrences| RouteSpec {
        id,
        entry_portal_id,
        exit_portal_id,
        lane_index,
        weight,
        occurrences,
    };
    vec![
        route(
            "route-main-west-near-left",
            "portal-main-west",
            "portal-side-1-north",
            0,
            20,
            vec![key(1, Approach::West, Turn::Left, 0, 0)],
        ),
        route(
            "route-main-west-far-left-via-lane-1",
            "portal-main-west",
            "portal-side-2-north",
            1,
            12,
            vec![
                key(1, Approach::West, Turn::Straight, 1, 0),
                key(2, Approach::West, Turn::Left, 0, 0),
            ],
        ),
        route(
            "route-main-west-through-lane-1-to-0",
            "portal-main-west",
            "portal-main-east",
            1,
            12,
            vec![
                key(1, Approach::West, Turn::Straight, 1, 1),
                key(2, Approach::West, Turn::Straight, 1, 0),
            ],
        ),
        route(
            "route-main-west-through-lane-1-to-1",
            "portal-main-west",
            "portal-main-east",
            1,
            12,
            vec![
                key(1, Approach::West, Turn::Straight, 1, 1),
                key(2, Approach::West, Turn::Straight, 1, 1),
            ],
        ),
        route(
            "route-main-west-near-right",
            "portal-main-west",
            "portal-side-1-south",
            2,
            20,
            vec![key(1, Approach::West, Turn::Right, 2, 1)],
        ),
        route(
            "route-main-west-through-lane-2-to-2",
            "portal-main-west",
            "portal-main-east",
            2,
            12,
            vec![
                key(1, Approach::West, Turn::Straight, 2, 2),
                key(2, Approach::West, Turn::Straight, 2, 2),
            ],
        ),
        route(
            "route-main-west-far-right-via-lane-2",
            "portal-main-west",
            "portal-side-2-south",
            2,
            12,
            vec![
                key(1, Approach::West, Turn::Straight, 2, 2),
                key(2, Approach::West, Turn::Right, 2, 1),
            ],
        ),
        route(
            "route-main-east-near-left",
            "portal-main-east",
            "portal-side-2-south",
            0,
            20,
            vec![key(2, Approach::East, Turn::Left, 0, 0)],
        ),
        route(
            "route-main-east-far-left-via-lane-1",
            "portal-main-east",
            "portal-side-1-south",
            1,
            12,
            vec![
                key(2, Approach::East, Turn::Straight, 1, 0),
                key(1, Approach::East, Turn::Left, 0, 0),
            ],
        ),
        route(
            "route-main-east-through-lane-1-to-0",
            "portal-main-east",
            "portal-main-west",
            1,
            12,
            vec![
                key(2, Approach::East, Turn::Straight, 1, 1),
                key(1, Approach::East, Turn::Straight, 1, 0),
            ],
        ),
        route(
            "route-main-east-through-lane-1-to-1",
            "portal-main-east",
            "portal-main-west",
            1,
            12,
            vec![
                key(2, Approach::East, Turn::Straight, 1, 1),
                key(1, Approach::East, Turn::Straight, 1, 1),
            ],
        ),
        route(
            "route-main-east-near-right",
            "portal-main-east",
            "portal-side-2-north",
            2,
            20,
            vec![key(2, Approach::East, Turn::Right, 2, 1)],
        ),
        route(
            "route-main-east-through-lane-2-to-2",
            "portal-main-east",
            "portal-main-west",
            2,
            12,
            vec![
                key(2, Approach::East, Turn::Straight, 2, 2),
                key(1, Approach::East, Turn::Straight, 2, 2),
            ],
        ),
        route(
            "route-main-east-far-right-via-lane-2",
            "portal-main-east",
            "portal-side-1-north",
            2,
            12,
            vec![
                key(2, Approach::East, Turn::Straight, 2, 2),
                key(1, Approach::East, Turn::Right, 2, 1),
            ],
        ),
        route(
            "route-side-1-north-corridor-left-far-left",
            "portal-side-1-north",
            "portal-side-2-north",
            0,
            20,
            vec![
                key(1, Approach::North, Turn::Left, 0, 0),
                key(2, Approach::West, Turn::Left, 0, 0),
            ],
        ),
        route(
            "route-side-1-north-through",
            "portal-side-1-north",
            "portal-side-1-south",
            1,
            60,
            vec![key(1, Approach::North, Turn::Straight, 1, 1)],
        ),
        route(
            "route-side-1-north-away-right",
            "portal-side-1-north",
            "portal-main-west",
            1,
            20,
            vec![key(1, Approach::North, Turn::Right, 1, 2)],
        ),
        route(
            "route-side-1-south-away-left",
            "portal-side-1-south",
            "portal-main-west",
            0,
            20,
            vec![key(1, Approach::South, Turn::Left, 0, 0)],
        ),
        route(
            "route-side-1-south-through",
            "portal-side-1-south",
            "portal-side-1-north",
            1,
            60,
            vec![key(1, Approach::South, Turn::Straight, 1, 1)],
        ),
        route(
            "route-side-1-south-corridor-right-through",
            "portal-side-1-south",
            "portal-main-east",
            1,
            15,
            vec![
                key(1, Approach::South, Turn::Right, 1, 2),
                key(2, Approach::West, Turn::Straight, 2, 2),
            ],
        ),
        route(
            "route-side-1-south-corridor-right-far-right",
            "portal-side-1-south",
            "portal-side-2-south",
            1,
            5,
            vec![
                key(1, Approach::South, Turn::Right, 1, 2),
                key(2, Approach::West, Turn::Right, 2, 1),
            ],
        ),
        route(
            "route-side-2-north-away-left",
            "portal-side-2-north",
            "portal-main-east",
            0,
            20,
            vec![key(2, Approach::North, Turn::Left, 0, 0)],
        ),
        route(
            "route-side-2-north-through",
            "portal-side-2-north",
            "portal-side-2-south",
            1,
            60,
            vec![key(2, Approach::North, Turn::Straight, 1, 1)],
        ),
        route(
            "route-side-2-north-corridor-right-through",
            "portal-side-2-north",
            "portal-main-west",
            1,
            15,
            vec![
                key(2, Approach::North, Turn::Right, 1, 2),
                key(1, Approach::East, Turn::Straight, 2, 2),
            ],
        ),
        route(
            "route-side-2-north-corridor-right-far-right",
            "portal-side-2-north",
            "portal-side-1-north",
            1,
            5,
            vec![
                key(2, Approach::North, Turn::Right, 1, 2),
                key(1, Approach::East, Turn::Right, 2, 1),
            ],
        ),
        route(
            "route-side-2-south-corridor-left-far-left",
            "portal-side-2-south",
            "portal-side-1-south",
            0,
            20,
            vec![
                key(2, Approach::South, Turn::Left, 0, 0),
                key(1, Approach::East, Turn::Left, 0, 0),
            ],
        ),
        route(
            "route-side-2-south-through",
            "portal-side-2-south",
            "portal-side-2-north",
            1,
            60,
            vec![key(2, Approach::South, Turn::Straight, 1, 1)],
        ),
        route(
            "route-side-2-south-away-right",
            "portal-side-2-south",
            "portal-main-east",
            1,
            20,
            vec![key(2, Approach::South, Turn::Right, 1, 2)],
        ),
    ]
}

fn movement_id(junction: usize, approach: Approach, turn: Turn) -> String {
    format!(
        "movement-junction-{junction}-{}-{}",
        approach.as_str(),
        turn.as_str()
    )
}

fn maneuver_path_id(key: PathKey) -> String {
    format!(
        "path-junction-{}-{}-{}-lane-{}-to-{}",
        key.junction,
        key.approach.as_str(),
        key.turn.as_str(),
        key.entry_lane,
        key.exit_lane
    )
}

fn internal_edge_id(key: PathKey) -> String {
    format!(
        "edge-junction-{}-{}-{}-lane-{}-to-{}-internal-0",
        key.junction,
        key.approach.as_str(),
        key.turn.as_str(),
        key.entry_lane,
        key.exit_lane
    )
}

fn maneuver_gate_id(key: PathKey) -> String {
    format!(
        "gate-junction-{}-{}-{}-lane-{}-to-{}",
        key.junction,
        key.approach.as_str(),
        key.turn.as_str(),
        key.entry_lane,
        key.exit_lane
    )
}

fn stop_line_id(junction: usize, approach: Approach, lane: usize) -> String {
    format!(
        "stop-line-junction-{junction}-{}-lane-{lane}",
        approach.as_str()
    )
}

fn signal_group_id(key: PathKey) -> String {
    let suffix = match (key.approach.is_main(), key.turn) {
        (true, Turn::Left) => "main-left",
        (true, Turn::Straight | Turn::Right) => "main-through-right",
        (false, Turn::Left) => "secondary-left",
        (false, Turn::Straight | Turn::Right) => "secondary-through-right",
    };
    format!("signal-group-junction-{}-{suffix}", key.junction)
}

fn signal_controller(config: &CorridorConfig, index: usize) -> SignalController {
    let junction = index + 1;
    let suffixes = [
        "main-left",
        "main-through-right",
        "secondary-left",
        "secondary-through-right",
    ];
    let group_ids = suffixes
        .map(|suffix| format!("signal-group-junction-{junction}-{suffix}"))
        .to_vec();
    let states = |active_index: Option<usize>, active_aspect: &'static str| {
        group_ids
            .iter()
            .enumerate()
            .map(|(group_index, group_id)| SignalGroupState {
                group_id: group_id.clone(),
                aspect: if active_index == Some(group_index) {
                    active_aspect
                } else {
                    "red"
                },
            })
            .collect()
    };
    SignalController {
        id: format!("signal-controller-junction-{junction}"),
        kind: "fixedTime",
        offset_ms: config.signals.controller_offsets_ms[index],
        group_ids: group_ids.clone(),
        phases: vec![
            SignalPhase {
                id: "phase-main-left-green".to_owned(),
                duration_ms: config.signals.main_left_green_ms,
                states: states(Some(0), "green"),
            },
            SignalPhase {
                id: "phase-main-left-yellow".to_owned(),
                duration_ms: config.signals.yellow_ms,
                states: states(Some(0), "yellow"),
            },
            SignalPhase {
                id: "phase-after-main-left-all-red".to_owned(),
                duration_ms: config.signals.all_red_ms,
                states: states(None, "red"),
            },
            SignalPhase {
                id: "phase-main-through-right-green".to_owned(),
                duration_ms: config.signals.main_through_right_green_ms,
                states: states(Some(1), "green"),
            },
            SignalPhase {
                id: "phase-main-through-right-yellow".to_owned(),
                duration_ms: config.signals.yellow_ms,
                states: states(Some(1), "yellow"),
            },
            SignalPhase {
                id: "phase-after-main-through-right-all-red".to_owned(),
                duration_ms: config.signals.all_red_ms,
                states: states(None, "red"),
            },
            SignalPhase {
                id: "phase-secondary-left-green".to_owned(),
                duration_ms: config.signals.secondary_left_green_ms,
                states: states(Some(2), "green"),
            },
            SignalPhase {
                id: "phase-secondary-left-yellow".to_owned(),
                duration_ms: config.signals.yellow_ms,
                states: states(Some(2), "yellow"),
            },
            SignalPhase {
                id: "phase-after-secondary-left-all-red".to_owned(),
                duration_ms: config.signals.all_red_ms,
                states: states(None, "red"),
            },
            SignalPhase {
                id: "phase-secondary-through-right-green".to_owned(),
                duration_ms: config.signals.secondary_through_right_green_ms,
                states: states(Some(3), "green"),
            },
            SignalPhase {
                id: "phase-secondary-through-right-yellow".to_owned(),
                duration_ms: config.signals.yellow_ms,
                states: states(Some(3), "yellow"),
            },
            SignalPhase {
                id: "phase-after-secondary-through-right-all-red".to_owned(),
                duration_ms: config.signals.all_red_ms,
                states: states(None, "red"),
            },
        ],
    }
}

fn build_catalog(
    config: &CorridorConfig,
    corridor: &CorridorBuild,
) -> Result<CorridorCatalog, Error> {
    let routes = &corridor.routes;
    let edge_by_id = corridor.edge_by_id();
    let portal_order = [
        "portal-main-west",
        "portal-main-east",
        "portal-side-1-north",
        "portal-side-1-south",
        "portal-side-2-north",
        "portal-side-2-south",
    ];
    let mut slots = Vec::new();
    let portals = portal_order
        .into_iter()
        .map(|portal_id| -> Result<PortalCatalogEntry, Error> {
            let portal_routes = routes
                .iter()
                .filter(|route| route.entry_portal_id == portal_id)
                .collect::<Vec<_>>();
            let mut lane_indices = portal_routes
                .iter()
                .map(|route| route.lane_index)
                .collect::<Vec<_>>();
            lane_indices.sort_unstable();
            lane_indices.dedup();
            let lanes = lane_indices
                .into_iter()
                .map(|lane_index| -> Result<PortalLaneCatalogEntry, Error> {
                    let lane_routes = portal_routes
                        .iter()
                        .copied()
                        .filter(|route| route.lane_index == lane_index)
                        .collect::<Vec<_>>();
                    let entry_edge_id = lane_routes[0]
                        .route
                        .edge_ids
                        .first()
                        .expect("every route has an entry edge");
                    let edge = edge_by_id
                        .get(entry_edge_id.as_str())
                        .expect("route edge exists in corridor topology");
                    let length = edge.length();
                    let minimum_length = ENDPOINT_CLEARANCE_METERS * 2.0;
                    if length < minimum_length {
                        return Err(Error::Config(format!(
                            "portal {portal_id:?} lane {lane_index} entry edge \
                             {entry_edge_id:?} is {length} m long; at least \
                             {minimum_length} m is required for one spawn slot"
                        )));
                    }
                    let mut progress = ENDPOINT_CLEARANCE_METERS;
                    let mut local_index = 0;
                    let entry_spawn_slot_id = format!(
                        "slot-{}-lane-{lane_index}-000",
                        portal_id.trim_start_matches("portal-")
                    );
                    while progress <= length - ENDPOINT_CLEARANCE_METERS {
                        let slot_id = format!(
                            "slot-{}-lane-{lane_index}-{local_index:03}",
                            portal_id.trim_start_matches("portal-")
                        );
                        slots.push(SpawnSlotCatalogEntry {
                            slot_id,
                            portal_id: portal_id.to_owned(),
                            lane_index,
                            edge_id: edge.id.clone(),
                            progress,
                        });
                        local_index += 1;
                        progress += config.geometry.spawn_slot_pitch_meters;
                    }
                    Ok(PortalLaneCatalogEntry {
                        lane_index,
                        entry_spawn_slot_id,
                        route_choices: lane_routes
                            .into_iter()
                            .map(|route| WeightedRouteChoiceCatalogEntry {
                                route_id: route.route.id.clone(),
                                weight: route.weight,
                            })
                            .collect(),
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(PortalCatalogEntry {
                id: portal_id.to_owned(),
                lanes,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let route_catalog = routes
        .iter()
        .map(|route| RouteCatalogEntry {
            route_id: route.route.id.clone(),
            exit_portal_id: route.exit_portal_id.clone(),
        })
        .collect();

    Ok(CorridorCatalog {
        catalog_version: CATALOG_VERSION.to_owned(),
        portals,
        routes: route_catalog,
        spawn_slots: slots,
    })
}

fn validate_catalog(catalog: &CorridorCatalog, corridor: &CorridorBuild) -> Result<(), Error> {
    let encoded = toml::to_string(catalog)?;
    let decoded: CorridorCatalog =
        toml::from_str(&encoded).map_err(|error| Error::Catalog(error.to_string()))?;
    if decoded != *catalog {
        return Err(Error::Catalog(
            "TOML round trip changed catalog semantics".to_owned(),
        ));
    }

    let route_by_id = corridor
        .routes
        .iter()
        .map(|route| (route.route.id.as_str(), route))
        .collect::<HashMap<_, _>>();
    let edge_by_id = corridor.edge_by_id();
    let portal_ids = catalog
        .portals
        .iter()
        .map(|portal| portal.id.as_str())
        .collect::<HashSet<_>>();
    let lane_by_key = catalog
        .portals
        .iter()
        .flat_map(|portal| {
            portal
                .lanes
                .iter()
                .map(move |lane| ((portal.id.as_str(), lane.lane_index), lane))
        })
        .collect::<HashMap<_, _>>();
    let mut slot_ids = HashSet::new();
    for slot in &catalog.spawn_slots {
        if !slot_ids.insert(slot.slot_id.as_str()) {
            return Err(Error::Catalog(format!(
                "duplicate spawn slot ID {:?}",
                slot.slot_id
            )));
        }
        if !portal_ids.contains(slot.portal_id.as_str()) {
            return Err(Error::Catalog(format!(
                "unknown portal {:?}",
                slot.portal_id
            )));
        }
        let lane = lane_by_key
            .get(&(slot.portal_id.as_str(), slot.lane_index))
            .ok_or_else(|| {
                Error::Catalog(format!(
                    "slot {:?} has no matching portal lane",
                    slot.slot_id
                ))
            })?;
        for choice in &lane.route_choices {
            let route = route_by_id
                .get(choice.route_id.as_str())
                .ok_or_else(|| Error::Catalog(format!("unknown route {:?}", choice.route_id)))?;
            if !route.route.edge_ids.contains(&slot.edge_id) {
                return Err(Error::Catalog(format!(
                    "slot {:?} edge_id is not present in route {:?}",
                    slot.slot_id, choice.route_id
                )));
            }
        }
        let length = edge_by_id
            .get(slot.edge_id.as_str())
            .expect("route edge exists in corridor topology")
            .length();
        if slot.progress < ENDPOINT_CLEARANCE_METERS
            || slot.progress > length - ENDPOINT_CLEARANCE_METERS
        {
            return Err(Error::Catalog(format!(
                "slot {:?} violates endpoint clearance",
                slot.slot_id
            )));
        }
    }
    for portal in &catalog.portals {
        for lane in &portal.lanes {
            if !slot_ids.contains(lane.entry_spawn_slot_id.as_str()) {
                return Err(Error::Catalog(format!(
                    "portal {:?} lane {} has no valid entry_spawn_slot_id",
                    portal.id, lane.lane_index
                )));
            }
        }
    }
    Ok(())
}

fn validate_schema(document: &'static str, schema_source: &str, input: &[u8]) -> Result<(), Error> {
    let schema = serde_json::from_str(schema_source).map_err(|source| Error::Json {
        document: "repository schema",
        source,
    })?;
    let instance =
        serde_json::from_slice(input).map_err(|source| Error::Json { document, source })?;
    jsonschema::draft202012::validate(&schema, &instance).map_err(|error| Error::Schema {
        document,
        message: error.to_string(),
    })
}

fn json_bytes<T: Serialize>(document: &'static str, value: &T) -> Result<Vec<u8>, Error> {
    let mut bytes =
        serde_json::to_vec_pretty(value).map_err(|source| Error::Json { document, source })?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn descriptor(artifact_ref: String, media_type: &'static str, bytes: &[u8]) -> ArtifactDescriptor {
    ArtifactDescriptor {
        artifact_ref,
        media_type,
        digest: format!("sha256:{}", hex_digest(Sha256::digest(bytes).as_slice())),
        size: u64::try_from(bytes.len()).expect("artifact size fits in u64"),
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn point_f64(point: [f32; 3]) -> [f64; 3] {
    point.map(f64::from)
}

impl CorridorBuild {
    fn edge_by_id(&self) -> HashMap<&str, &EdgeBuild> {
        self.edges
            .iter()
            .map(|edge| (edge.id.as_str(), edge))
            .collect()
    }
}

impl EdgeBuild {
    fn start(&self) -> [f32; 3] {
        *self.points.first().expect("edge has a start point")
    }

    fn end(&self) -> [f32; 3] {
        *self.points.last().expect("edge has an end point")
    }

    fn length(&self) -> f64 {
        self.points
            .windows(2)
            .map(|pair| edge_length(pair[0], pair[1]))
            .sum()
    }
}

fn edge_length(start: [f32; 3], end: [f32; 3]) -> f64 {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let dz = end[2] - start[2];
    f64::from(dx.hypot(dy).hypot(dz))
}

fn kilometers_per_hour_to_meters_per_second(value: f64) -> f64 {
    value / 3.6
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_length_sums_every_polyline_segment() {
        let edge = EdgeBuild {
            id: "edge-polyline".to_owned(),
            points: vec![[0.0, 0.0, 0.0], [3.0, 0.0, 4.0], [6.0, 0.0, 8.0]],
            speed_limit: 10.0,
            connections: Vec::new(),
        };

        assert_eq!(edge.length(), 10.0);
    }
}
