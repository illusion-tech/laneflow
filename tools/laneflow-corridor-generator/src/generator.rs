use std::collections::{HashMap, HashSet};

use laneflow_core::CoreWorld;
use laneflow_data::{
    NamedArtifact, SPATIAL_PACKAGE_MEDIA_TYPE, TRAFFIC_PACKAGE_MEDIA_TYPE, from_scenario_json_slice,
};
use laneflow_scenario::signalized_corridor::{
    CorridorCatalog, PortalCatalogEntry, RouteCatalogEntry, SpawnSlotCatalogEntry,
};
use laneflow_spatial::{SpatialEdgeInput, SpatialRegistry};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::Error;
use crate::config::{
    CorridorConfig, ENDPOINT_CLEARANCE_METERS, MIN_GAP_METERS, MIN_SPAWN_SLOT_COUNT,
    VEHICLE_LENGTH_METERS,
};
use crate::model::{
    ArtifactDescriptor, Centerline, LaneConnection, LaneEdge, LaneGraph, MovementGate, Parking,
    Route, ScenarioManifest, SignalControl, SignalController, SignalGroup, SignalGroupState,
    SignalPhase, Signals, SpatialEdge, SpatialPackage, StopLine, TrafficPackage, Units,
    VehicleProfile,
};

const TRAFFIC_SCHEMA: &str = include_str!("../../../schemas/laneflow-data-v0.7.schema.json");
const SPATIAL_SCHEMA: &str = include_str!("../../../schemas/laneflow-spatial-v0.1.schema.json");
const MANIFEST_SCHEMA: &str =
    include_str!("../../../schemas/laneflow-scenario-manifest-v0.1.schema.json");
const CATALOG_VERSION: &str = "0.1";

#[derive(Clone, Debug)]
struct RouteBuild {
    route: Route,
    entry_portal_id: String,
    exit_portal_id: String,
    lane_index: usize,
    edges: Vec<EdgeBuild>,
    connector_indices: Vec<usize>,
}

#[derive(Clone, Debug)]
struct RouteIdentity {
    route_id: String,
    entry_portal_id: String,
    exit_portal_id: String,
    lane_index: usize,
}

#[derive(Clone, Debug)]
struct EdgeBuild {
    id: String,
    start: [f32; 3],
    end: [f32; 3],
    speed_limit: f64,
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
    pub stop_lines: usize,
    pub movement_gates: usize,
    pub signal_groups: usize,
    pub controllers: usize,
    pub phases: usize,
    pub portals: usize,
    pub spawn_slots: usize,
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
    let routes = build_routes(config);
    let (traffic, spatial, catalog) = build_documents(config, &routes)?;

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

    validate_runtime(config, &traffic_bytes, &spatial_bytes, &manifest_bytes)?;
    validate_catalog(&catalog, &routes)?;

    let counts = ScenarioCounts {
        edges: traffic.lane_graph.edges.len(),
        routes: traffic.routes.len(),
        stop_lines: traffic.signals.stop_lines.len(),
        movement_gates: traffic.signals.movement_gates.len(),
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
    routes: &[RouteBuild],
) -> Result<(TrafficPackage, SpatialPackage, CorridorCatalog), Error> {
    let mut lane_edges = Vec::new();
    let mut spatial_edges = Vec::new();
    let mut stop_lines = Vec::new();
    let mut movement_gates = Vec::new();

    for route in routes {
        for (index, edge) in route.edges.iter().enumerate() {
            let connection = route.edges.get(index + 1).map(|next| LaneConnection {
                to_edge_id: next.id.clone(),
            });
            lane_edges.push(LaneEdge {
                id: edge.id.clone(),
                length: edge_length(edge.start, edge.end),
                speed_limit: edge.speed_limit,
                connections: connection.into_iter().collect(),
            });
            spatial_edges.push(SpatialEdge {
                traffic_edge_id: edge.id.clone(),
                centerline: Centerline {
                    points: vec![point_f64(edge.start), point_f64(edge.end)],
                },
            });
        }
        for connector_index in &route.connector_indices {
            let from = &route.edges[connector_index - 1];
            let connector = &route.edges[*connector_index];
            let stop_line_id = format!("stop-{}", from.id);
            let group_id = group_for_connector(&connector.id)?;
            stop_lines.push(StopLine {
                id: stop_line_id.clone(),
                edge_id: from.id.clone(),
                location: "edgeEnd",
            });
            movement_gates.push(MovementGate {
                from_edge_id: from.id.clone(),
                to_edge_id: connector.id.clone(),
                stop_line_id,
                signal_control: SignalControl {
                    kind: "group",
                    group_id,
                },
            });
        }
    }

    let controllers = (0..2)
        .map(|index| signal_controller(config, index))
        .collect::<Vec<_>>();
    let signals = Signals {
        stop_lines,
        movement_gates,
        groups: (1..=2)
            .flat_map(|intersection| {
                ["main", "secondary"].map(|road| SignalGroup {
                    id: format!("group-intersection-{intersection}-{road}"),
                })
            })
            .collect(),
        controllers,
    };

    let traffic = TrafficPackage {
        format_version: "0.7",
        units: Units {
            distance: "meter",
            time: "second",
        },
        lane_graph: LaneGraph { edges: lane_edges },
        routes: routes.iter().map(|item| item.route.clone()).collect(),
        vehicle_profiles: vec![VehicleProfile {
            id: "passenger-car",
            length: VEHICLE_LENGTH_METERS,
            model: "iidm",
            desired_speed: 20.0,
            min_gap: MIN_GAP_METERS,
            time_headway: 1.5,
            max_acceleration: 1.5,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 6.0,
        }],
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
    let catalog = build_catalog(config, routes);
    Ok((traffic, spatial, catalog))
}

fn build_routes(config: &CorridorConfig) -> Vec<RouteBuild> {
    let main_speed =
        kilometers_per_hour_to_meters_per_second(config.speed_limits.main_kilometers_per_hour);
    let secondary_speed =
        kilometers_per_hour_to_meters_per_second(config.speed_limits.secondary_kilometers_per_hour);
    let lane_width = config.geometry.lane_width_meters as f32;
    let main_half = (config.geometry.main_length_meters / 2.0) as f32;
    let [intersection_1, intersection_2] = config
        .geometry
        .intersection_x_meters
        .map(|value| value as f32);
    let connector_half = lane_width * 2.0;
    let main_bounds = [
        -main_half,
        intersection_1 - connector_half,
        intersection_1 + connector_half,
        intersection_2 - connector_half,
        intersection_2 + connector_half,
        main_half,
    ];

    let mut routes = Vec::with_capacity(14);
    for lane in 0..3 {
        let z = (lane as f32 + 0.5) * lane_width;
        routes.push(route_from_points(
            RouteIdentity {
                route_id: format!("route-main-w2e-lane-{lane}"),
                entry_portal_id: "portal-main-west".to_owned(),
                exit_portal_id: "portal-main-east".to_owned(),
                lane_index: lane,
            },
            "main-w2e",
            &main_bounds.map(|x| [x, 0.0, z]),
            main_speed,
            &[1, 3],
        ));
    }
    for lane in 0..3 {
        let z = -(lane as f32 + 0.5) * lane_width;
        let points = main_bounds.map(|x| [x, 0.0, z]);
        let points = points.into_iter().rev().collect::<Vec<_>>();
        routes.push(route_from_points(
            RouteIdentity {
                route_id: format!("route-main-e2w-lane-{lane}"),
                entry_portal_id: "portal-main-east".to_owned(),
                exit_portal_id: "portal-main-west".to_owned(),
                lane_index: lane,
            },
            "main-e2w",
            &points,
            main_speed,
            &[1, 3],
        ));
    }

    for (intersection_index, intersection_x) in
        [intersection_1, intersection_2].into_iter().enumerate()
    {
        let road_number = intersection_index + 1;
        let half_length =
            (config.geometry.secondary_lengths_meters[intersection_index] / 2.0) as f32;
        let main_half_width = lane_width * 3.0;
        let bounds = [-half_length, -main_half_width, main_half_width, half_length];
        for lane in 0..2 {
            let x = intersection_x - (lane as f32 + 0.5) * lane_width;
            routes.push(route_from_points(
                RouteIdentity {
                    route_id: format!("route-side-{road_number}-n2s-lane-{lane}"),
                    entry_portal_id: format!("portal-side-{road_number}-north"),
                    exit_portal_id: format!("portal-side-{road_number}-south"),
                    lane_index: lane,
                },
                &format!("side-{road_number}-n2s"),
                &bounds.map(|z| [x, 0.0, z]),
                secondary_speed,
                &[1],
            ));
        }
        for lane in 0..2 {
            let x = intersection_x + (lane as f32 + 0.5) * lane_width;
            let points = bounds
                .map(|z| [x, 0.0, z])
                .into_iter()
                .rev()
                .collect::<Vec<_>>();
            routes.push(route_from_points(
                RouteIdentity {
                    route_id: format!("route-side-{road_number}-s2n-lane-{lane}"),
                    entry_portal_id: format!("portal-side-{road_number}-south"),
                    exit_portal_id: format!("portal-side-{road_number}-north"),
                    lane_index: lane,
                },
                &format!("side-{road_number}-s2n"),
                &points,
                secondary_speed,
                &[1],
            ));
        }
    }
    routes
}

fn route_from_points(
    identity: RouteIdentity,
    edge_prefix: &str,
    points: &[[f32; 3]],
    speed_limit: f64,
    connector_indices: &[usize],
) -> RouteBuild {
    let connector_set = connector_indices.iter().copied().collect::<HashSet<_>>();
    let mut edges = Vec::with_capacity(points.len() - 1);
    for (index, pair) in points.windows(2).enumerate() {
        let id = if connector_set.contains(&index) {
            let intersection = connector_intersection(edge_prefix, index);
            format!(
                "edge-{edge_prefix}-lane-{}-connector-intersection-{intersection}-straight",
                identity.lane_index
            )
        } else {
            format!(
                "edge-{edge_prefix}-lane-{}-road-{index}",
                identity.lane_index
            )
        };
        edges.push(EdgeBuild {
            id,
            start: pair[0],
            end: pair[1],
            speed_limit,
        });
    }
    RouteBuild {
        route: Route {
            id: identity.route_id,
            edge_ids: edges.iter().map(|edge| edge.id.clone()).collect(),
        },
        entry_portal_id: identity.entry_portal_id,
        exit_portal_id: identity.exit_portal_id,
        lane_index: identity.lane_index,
        edges,
        connector_indices: connector_indices.to_vec(),
    }
}

fn connector_intersection(edge_prefix: &str, edge_index: usize) -> usize {
    if edge_prefix.starts_with("main-") {
        match (edge_prefix, edge_index) {
            ("main-w2e", 1) | ("main-e2w", 3) => 1,
            _ => 2,
        }
    } else if edge_prefix.starts_with("side-1-") {
        1
    } else {
        2
    }
}

fn group_for_connector(connector_id: &str) -> Result<String, Error> {
    let intersection = if connector_id.contains("intersection-1") {
        1
    } else if connector_id.contains("intersection-2") {
        2
    } else {
        return Err(Error::Validation {
            stage: "generator",
            message: format!("connector {connector_id:?} has no intersection identity"),
        });
    };
    let road = if connector_id.contains("edge-main-") {
        "main"
    } else {
        "secondary"
    };
    Ok(format!("group-intersection-{intersection}-{road}"))
}

fn signal_controller(config: &CorridorConfig, index: usize) -> SignalController {
    let intersection = index + 1;
    let main = format!("group-intersection-{intersection}-main");
    let secondary = format!("group-intersection-{intersection}-secondary");
    let states = |main_aspect, secondary_aspect| {
        vec![
            SignalGroupState {
                group_id: main.clone(),
                aspect: main_aspect,
            },
            SignalGroupState {
                group_id: secondary.clone(),
                aspect: secondary_aspect,
            },
        ]
    };
    SignalController {
        id: format!("controller-intersection-{intersection}"),
        kind: "fixedTime",
        offset_ms: config.signals.controller_offsets_ms[index],
        group_ids: vec![main.clone(), secondary.clone()],
        phases: vec![
            SignalPhase {
                id: "main-green".to_owned(),
                duration_ms: config.signals.main_green_ms,
                states: states("green", "red"),
            },
            SignalPhase {
                id: "main-yellow".to_owned(),
                duration_ms: config.signals.yellow_ms,
                states: states("yellow", "red"),
            },
            SignalPhase {
                id: "all-red-before-secondary".to_owned(),
                duration_ms: config.signals.all_red_ms,
                states: states("red", "red"),
            },
            SignalPhase {
                id: "secondary-green".to_owned(),
                duration_ms: config.signals.secondary_green_ms,
                states: states("red", "green"),
            },
            SignalPhase {
                id: "secondary-yellow".to_owned(),
                duration_ms: config.signals.yellow_ms,
                states: states("red", "yellow"),
            },
            SignalPhase {
                id: "all-red-before-main".to_owned(),
                duration_ms: config.signals.all_red_ms,
                states: states("red", "red"),
            },
        ],
    }
}

fn build_catalog(config: &CorridorConfig, routes: &[RouteBuild]) -> CorridorCatalog {
    let portal_order = [
        "portal-main-west",
        "portal-main-east",
        "portal-side-1-north",
        "portal-side-1-south",
        "portal-side-2-north",
        "portal-side-2-south",
    ];
    let portals = portal_order
        .into_iter()
        .map(|id| PortalCatalogEntry {
            id: id.to_owned(),
            entry_route_ids: routes
                .iter()
                .filter(|route| route.entry_portal_id == id)
                .map(|route| route.route.id.clone())
                .collect(),
        })
        .collect();

    let mut slots = Vec::new();
    let mut route_catalog = Vec::with_capacity(routes.len());
    for route in routes {
        let eligible_indices: &[usize] = if route.route.id.starts_with("route-main-") {
            &[0, 2]
        } else {
            &[0]
        };
        let mut first_slot_id = None;
        for edge_index in eligible_indices {
            let edge = &route.edges[*edge_index];
            let length = edge_length(edge.start, edge.end);
            let mut progress = ENDPOINT_CLEARANCE_METERS;
            let mut local_index = 0;
            while progress <= length - ENDPOINT_CLEARANCE_METERS {
                let slot_id = format!(
                    "slot-{}-edge-{}-{local_index:03}",
                    route.route.id.trim_start_matches("route-"),
                    edge_index
                );
                if first_slot_id.is_none() {
                    first_slot_id = Some(slot_id.clone());
                }
                slots.push(SpawnSlotCatalogEntry {
                    slot_id,
                    portal_id: route.entry_portal_id.clone(),
                    route_id: route.route.id.clone(),
                    route_edge_index: *edge_index,
                    edge_id: edge.id.clone(),
                    progress,
                });
                local_index += 1;
                progress += config.geometry.spawn_slot_pitch_meters;
            }
        }
        route_catalog.push(RouteCatalogEntry {
            route_id: route.route.id.clone(),
            entry_portal_id: route.entry_portal_id.clone(),
            exit_portal_id: route.exit_portal_id.clone(),
            lane_index: route.lane_index,
            entry_spawn_slot_id: first_slot_id.unwrap_or_default(),
        });
    }

    CorridorCatalog {
        catalog_version: CATALOG_VERSION.to_owned(),
        portals,
        routes: route_catalog,
        spawn_slots: slots,
    }
}

fn validate_catalog(catalog: &CorridorCatalog, routes: &[RouteBuild]) -> Result<(), Error> {
    let encoded = toml::to_string(catalog)?;
    let decoded: CorridorCatalog =
        toml::from_str(&encoded).map_err(|error| Error::Catalog(error.to_string()))?;
    if decoded != *catalog {
        return Err(Error::Catalog(
            "TOML round trip changed catalog semantics".to_owned(),
        ));
    }

    let route_by_id = routes
        .iter()
        .map(|route| (route.route.id.as_str(), route))
        .collect::<HashMap<_, _>>();
    let portal_ids = catalog
        .portals
        .iter()
        .map(|portal| portal.id.as_str())
        .collect::<HashSet<_>>();
    let mut slot_ids = HashSet::new();
    for slot in &catalog.spawn_slots {
        if !slot_ids.insert(slot.slot_id.as_str()) {
            return Err(Error::Catalog(format!(
                "duplicate spawn slot ID {:?}",
                slot.slot_id
            )));
        }
        let route = route_by_id
            .get(slot.route_id.as_str())
            .ok_or_else(|| Error::Catalog(format!("unknown route {:?}", slot.route_id)))?;
        if !portal_ids.contains(slot.portal_id.as_str()) {
            return Err(Error::Catalog(format!(
                "unknown portal {:?}",
                slot.portal_id
            )));
        }
        let expected_edge = route
            .route
            .edge_ids
            .get(slot.route_edge_index)
            .ok_or_else(|| {
                Error::Catalog(format!(
                    "route_edge_index {} is out of range for {:?}",
                    slot.route_edge_index, slot.route_id
                ))
            })?;
        if expected_edge != &slot.edge_id {
            return Err(Error::Catalog(format!(
                "slot {:?} edge_id does not match route_edge_index",
                slot.slot_id
            )));
        }
        let length = edge_length(
            route.edges[slot.route_edge_index].start,
            route.edges[slot.route_edge_index].end,
        );
        if slot.progress < ENDPOINT_CLEARANCE_METERS
            || slot.progress > length - ENDPOINT_CLEARANCE_METERS
        {
            return Err(Error::Catalog(format!(
                "slot {:?} violates endpoint clearance",
                slot.slot_id
            )));
        }
    }
    for route in &catalog.routes {
        if !slot_ids.contains(route.entry_spawn_slot_id.as_str()) {
            return Err(Error::Catalog(format!(
                "route {:?} has no valid entry_spawn_slot_id",
                route.route_id
            )));
        }
    }
    Ok(())
}

fn validate_runtime(
    config: &CorridorConfig,
    traffic: &[u8],
    spatial: &[u8],
    manifest: &[u8],
) -> Result<(), Error> {
    let loaded = from_scenario_json_slice(
        manifest,
        &[
            NamedArtifact::new(&config.output.traffic_artifact_ref, traffic),
            NamedArtifact::new(&config.output.spatial_artifact_ref, spatial),
        ],
    )
    .map_err(|error| Error::Validation {
        stage: "production scenario loader",
        message: error.to_string(),
    })?;
    let (traffic, spatial) = loaded.into_parts();
    let traffic = traffic.into_initial_traffic_data();
    SpatialRegistry::try_new(
        traffic.lane_graph(),
        spatial.frame_id().clone(),
        spatial
            .edges()
            .iter()
            .map(|edge| SpatialEdgeInput::new(edge.edge(), edge.points())),
    )
    .map_err(|error| Error::Validation {
        stage: "SpatialRegistry",
        message: error.to_string(),
    })?;
    CoreWorld::with_traffic_data(config.fixed_delta_ms, traffic, Vec::new()).map_err(|error| {
        Error::Validation {
            stage: "CoreWorld",
            message: error.to_string(),
        }
    })?;
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

fn edge_length(start: [f32; 3], end: [f32; 3]) -> f64 {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let dz = end[2] - start[2];
    f64::from(dx.hypot(dy).hypot(dz))
}

fn kilometers_per_hour_to_meters_per_second(value: f64) -> f64 {
    value / 3.6
}
