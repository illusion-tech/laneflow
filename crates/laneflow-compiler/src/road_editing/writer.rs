use laneflow_road_editing_wire::generated::lane_flow::road_editing::v1 as wire;
use laneflow_road_editing_wire::runtime;
use laneflow_static_contract::{AccessEffect, EntityKindMarker, SignalAspect};

use super::builder::{RoadEditingSourceModule, RoadEditingSourceModuleParts};
use super::model::*;
use super::rules::input_error;
use crate::{
    CompileLimitDimension, CompileLimits, Diagnostic, DiagnosticBundle, GeometryAccuracyProfile,
    GeometryDirectionProfile, RoadEditingInputViolation,
};

const FORMAT_VERSION: u32 = 2;

/// 直接拥有 FlatBuffers storage 与有效尾部起点的 size-prefixed 来源缓冲区。
pub struct OwnedRoadEditingSourceBuffer {
    storage: Vec<u8>,
    start: usize,
}

impl OwnedRoadEditingSourceBuffer {
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.storage[self.start..]
    }

    #[must_use]
    pub fn retained_capacity_bytes(&self) -> usize {
        self.storage.capacity()
    }
}

/// 把字段私有 authoring model 确定性编码为 `LFRE` size-prefixed FlatBuffers。
pub struct RoadEditingSourceWriter<'limits> {
    limits: &'limits CompileLimits,
}

impl<'limits> RoadEditingSourceWriter<'limits> {
    #[must_use]
    pub const fn new(limits: &'limits CompileLimits) -> Self {
        Self { limits }
    }

    pub fn write(
        self,
        module: RoadEditingSourceModule,
    ) -> Result<OwnedRoadEditingSourceBuffer, DiagnosticBundle> {
        let capacity_limit = self
            .limits
            .value(CompileLimitDimension::SourceBytesPerModule);
        let RoadEditingSourceModuleParts {
            header,
            geometry_accuracy_profile: accuracy,
            geometry_direction_profile: direction,
            road_alignments: alignments,
            declarations,
            wire_upper_bound,
        } = module.into_parts();
        if wire_upper_bound > capacity_limit {
            return Err(DiagnosticBundle::single(
                Diagnostic::compile_limit_exceeded(
                    CompileLimitDimension::SourceBytesPerModule,
                    capacity_limit,
                    wire_upper_bound,
                ),
            ));
        }
        let capacity = usize::try_from(wire_upper_bound).map_err(|_| {
            DiagnosticBundle::single(Diagnostic::compile_limit_exceeded(
                CompileLimitDimension::SourceBytesPerModule,
                capacity_limit,
                wire_upper_bound,
            ))
        })?;
        let mut fbb = runtime::FlatBufferBuilder::with_capacity(capacity);

        let current_namespace = header.authoring_namespace_id();
        let module_header = encode_module_header(&mut fbb, &header);
        let road_alignments = alignments
            .iter()
            .map(|value| encode_road_alignment(&mut fbb, value))
            .collect::<Vec<_>>();

        let mut road_corridors = Vec::new();
        let mut road_sections = Vec::new();
        let mut authoring_lanes = Vec::new();
        let mut lane_edges = Vec::new();
        let mut junctions = Vec::new();
        let mut movements = Vec::new();
        let mut maneuver_paths = Vec::new();
        let mut maneuver_gates = Vec::new();
        let mut waiting_zones = Vec::new();
        let mut stop_lines = Vec::new();
        let mut signal_groups = Vec::new();
        let mut signal_controllers = Vec::new();
        let mut signal_phases = Vec::new();
        let mut parking_areas = Vec::new();
        let mut parking_spaces = Vec::new();
        let mut lane_groups = Vec::new();
        let mut facility_bands = Vec::new();
        let mut participant_classes = Vec::new();
        let mut access_rules = Vec::new();
        let mut vehicle_profiles = Vec::new();
        let mut canonical_frames = Vec::new();

        for declaration in &declarations {
            match declaration {
                RoadEditingDeclaration::RoadCorridor(value) => {
                    road_corridors.push(encode_road_corridor(&mut fbb, value));
                }
                RoadEditingDeclaration::RoadSection(value) => {
                    road_sections.push(encode_road_section(&mut fbb, value));
                }
                RoadEditingDeclaration::AuthoringLane(value) => {
                    authoring_lanes.push(encode_authoring_lane(&mut fbb, value));
                }
                RoadEditingDeclaration::LaneEdge(value) => {
                    lane_edges.push(encode_lane_edge(&mut fbb, value, current_namespace));
                }
                RoadEditingDeclaration::Junction(value) => {
                    junctions.push(encode_junction(&mut fbb, value, current_namespace));
                }
                RoadEditingDeclaration::Movement(value) => {
                    movements.push(encode_movement(&mut fbb, value));
                }
                RoadEditingDeclaration::ManeuverPath(value) => {
                    maneuver_paths.push(encode_maneuver_path(&mut fbb, value));
                }
                RoadEditingDeclaration::ManeuverGate(value) => {
                    maneuver_gates.push(encode_maneuver_gate(&mut fbb, value));
                }
                RoadEditingDeclaration::WaitingZone(value) => {
                    waiting_zones.push(encode_waiting_zone(&mut fbb, value));
                }
                RoadEditingDeclaration::StopLine(value) => {
                    stop_lines.push(encode_stop_line(&mut fbb, value));
                }
                RoadEditingDeclaration::SignalGroup(value) => {
                    signal_groups.push(encode_signal_group(&mut fbb, value));
                }
                RoadEditingDeclaration::SignalController(value) => {
                    signal_controllers.push(encode_signal_controller(
                        &mut fbb,
                        value,
                        current_namespace,
                    ));
                }
                RoadEditingDeclaration::SignalPhase(value) => {
                    signal_phases.push(encode_signal_phase(&mut fbb, value, current_namespace));
                }
                RoadEditingDeclaration::ParkingArea(value) => {
                    parking_areas.push(encode_parking_area(&mut fbb, value));
                }
                RoadEditingDeclaration::ParkingSpace(value) => {
                    parking_spaces.push(encode_parking_space(&mut fbb, value));
                }
                RoadEditingDeclaration::LaneGroup(value) => {
                    lane_groups.push(encode_lane_group(&mut fbb, value));
                }
                RoadEditingDeclaration::FacilityBand(value) => {
                    facility_bands.push(encode_facility_band(&mut fbb, value));
                }
                RoadEditingDeclaration::ParticipantClass(value) => {
                    participant_classes.push(encode_participant_class(&mut fbb, value));
                }
                RoadEditingDeclaration::AccessRule(value) => {
                    access_rules.push(encode_access_rule(&mut fbb, value, current_namespace));
                }
                RoadEditingDeclaration::VehicleProfile(value) => {
                    vehicle_profiles.push(encode_vehicle_profile(&mut fbb, value));
                }
                RoadEditingDeclaration::CanonicalFrame(value) => {
                    canonical_frames.push(encode_canonical_frame(&mut fbb, value));
                }
            }
        }

        let road_alignments = fbb.create_vector(&road_alignments);
        let road_corridors = fbb.create_vector(&road_corridors);
        let road_sections = fbb.create_vector(&road_sections);
        let authoring_lanes = fbb.create_vector(&authoring_lanes);
        let lane_edges = fbb.create_vector(&lane_edges);
        let junctions = fbb.create_vector(&junctions);
        let movements = fbb.create_vector(&movements);
        let maneuver_paths = fbb.create_vector(&maneuver_paths);
        let maneuver_gates = fbb.create_vector(&maneuver_gates);
        let waiting_zones = fbb.create_vector(&waiting_zones);
        let stop_lines = fbb.create_vector(&stop_lines);
        let signal_groups = fbb.create_vector(&signal_groups);
        let signal_controllers = fbb.create_vector(&signal_controllers);
        let signal_phases = fbb.create_vector(&signal_phases);
        let parking_areas = fbb.create_vector(&parking_areas);
        let parking_spaces = fbb.create_vector(&parking_spaces);
        let lane_groups = fbb.create_vector(&lane_groups);
        let facility_bands = fbb.create_vector(&facility_bands);
        let participant_classes = fbb.create_vector(&participant_classes);
        let access_rules = fbb.create_vector(&access_rules);
        let vehicle_profiles = fbb.create_vector(&vehicle_profiles);
        let canonical_frames = fbb.create_vector(&canonical_frames);

        let root = wire::RoadEditingSource::create(
            &mut fbb,
            &wire::RoadEditingSourceArgs {
                format_version: FORMAT_VERSION,
                module_header: Some(module_header),
                geometry_accuracy_profile: encode_accuracy(accuracy),
                geometry_direction_profile: encode_direction(direction),
                road_alignments: Some(road_alignments),
                road_corridors: Some(road_corridors),
                road_sections: Some(road_sections),
                authoring_lanes: Some(authoring_lanes),
                lane_edges: Some(lane_edges),
                junctions: Some(junctions),
                movements: Some(movements),
                maneuver_paths: Some(maneuver_paths),
                maneuver_gates: Some(maneuver_gates),
                waiting_zones: Some(waiting_zones),
                stop_lines: Some(stop_lines),
                signal_groups: Some(signal_groups),
                signal_controllers: Some(signal_controllers),
                signal_phases: Some(signal_phases),
                parking_areas: Some(parking_areas),
                parking_spaces: Some(parking_spaces),
                lane_groups: Some(lane_groups),
                facility_bands: Some(facility_bands),
                participant_classes: Some(participant_classes),
                access_rules: Some(access_rules),
                vehicle_profiles: Some(vehicle_profiles),
                canonical_frames: Some(canonical_frames),
            },
        );
        wire::finish_size_prefixed_road_editing_source_buffer(&mut fbb, root);
        let (storage, start) = fbb.collapse();
        if start > storage.len() {
            return Err(input_error(
                "roadEditingWriter.bufferStart",
                RoadEditingInputViolation::InvalidCombination,
            ));
        }
        let observed = u64::try_from(storage.len().saturating_sub(start)).unwrap_or(u64::MAX);
        if observed > wire_upper_bound {
            return Err(DiagnosticBundle::single(
                Diagnostic::compile_limit_exceeded(
                    CompileLimitDimension::SourceBytesPerModule,
                    wire_upper_bound,
                    observed,
                ),
            ));
        }
        if observed > capacity_limit {
            return Err(DiagnosticBundle::single(
                Diagnostic::compile_limit_exceeded(
                    CompileLimitDimension::SourceBytesPerModule,
                    capacity_limit,
                    observed,
                ),
            ));
        }
        let bytes = &storage[start..];
        let Some(prefix) = bytes.get(..4) else {
            return Err(input_error(
                "roadEditingWriter.sizePrefix",
                RoadEditingInputViolation::InvalidCombination,
            ));
        };
        let prefix = u32::from_le_bytes(prefix.try_into().expect("four-byte prefix"));
        if usize::try_from(prefix).ok() != Some(bytes.len() - 4) {
            return Err(input_error(
                "roadEditingWriter.sizePrefix",
                RoadEditingInputViolation::InvalidCombination,
            ));
        }
        Ok(OwnedRoadEditingSourceBuffer { storage, start })
    }
}

fn encode_accuracy(value: GeometryAccuracyProfile) -> wire::GeometryAccuracyProfile {
    match value {
        GeometryAccuracyProfile::Fine2Cm => wire::GeometryAccuracyProfile::Fine2Cm,
        GeometryAccuracyProfile::Balanced5Cm => wire::GeometryAccuracyProfile::Balanced5Cm,
        GeometryAccuracyProfile::Compact10Cm => wire::GeometryAccuracyProfile::Compact10Cm,
    }
}

fn encode_direction(value: GeometryDirectionProfile) -> wire::GeometryDirectionProfile {
    match value {
        GeometryDirectionProfile::Smooth1Deg => wire::GeometryDirectionProfile::Smooth1Deg,
        GeometryDirectionProfile::Balanced2Deg => wire::GeometryDirectionProfile::Balanced2Deg,
        GeometryDirectionProfile::Compact5Deg => wire::GeometryDirectionProfile::Compact5Deg,
    }
}

fn encode_point(value: RoadEditingPoint3) -> wire::Vec3F64 {
    wire::Vec3F64::new(value.x(), value.y(), value.z())
}

fn encode_width(value: LinearWidthProfile) -> wire::LinearWidthProfile {
    wire::LinearWidthProfile::new(value.start_width_meters(), value.end_width_meters())
}

fn create_reference<'fbb, K: EntityKindMarker>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &RoadEditingReference<K>,
) -> runtime::WIPOffset<&'fbb str> {
    let spelling = value.wire_spelling();
    fbb.create_string(&spelling)
}

fn create_reference_vector<'fbb, K: EntityKindMarker + Ord>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    values: &[RoadEditingReference<K>],
) -> runtime::WIPOffset<runtime::Vector<'fbb, runtime::ForwardsUOffset<&'fbb str>>> {
    let spellings = values
        .iter()
        .map(RoadEditingReference::wire_spelling)
        .collect::<Vec<_>>();
    let offsets = spellings
        .iter()
        .map(|value| fbb.create_string(value))
        .collect::<Vec<_>>();
    fbb.create_vector(&offsets)
}

fn create_sorted_reference_vector<'fbb, K: EntityKindMarker + Ord>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    values: &[RoadEditingReference<K>],
    current_namespace: &str,
) -> runtime::WIPOffset<runtime::Vector<'fbb, runtime::ForwardsUOffset<&'fbb str>>> {
    let mut values = values.iter().collect::<Vec<_>>();
    values.sort_unstable_by(|left, right| left.canonical_target_cmp(right, current_namespace));
    let spellings = values
        .into_iter()
        .map(RoadEditingReference::wire_spelling)
        .collect::<Vec<_>>();
    let offsets = spellings
        .iter()
        .map(|value| fbb.create_string(value))
        .collect::<Vec<_>>();
    fbb.create_vector(&offsets)
}

fn create_canvas<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: Option<&str>,
) -> Option<runtime::WIPOffset<&'fbb str>> {
    value.map(|value| fbb.create_string(value))
}

fn encode_module_header<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &RoadEditingModuleHeader,
) -> runtime::WIPOffset<wire::ModuleHeader<'fbb>> {
    let provenance = value.provenance();
    let generator_build_id = fbb.create_string(provenance.generator_build_id());
    let description = fbb.create_string(provenance.description());
    let inputs_digest = wire::Digest256::new(provenance.parameters_and_inputs_digest());
    let options_digest = wire::Digest256::new(provenance.frontend_options_digest());
    let random_seed = provenance.random_seed().map(wire::OptionalU64::new);
    let provenance = wire::Provenance::create(
        fbb,
        &wire::ProvenanceArgs {
            kind: match provenance.kind() {
                RoadEditingProvenanceKind::Direct => wire::ProvenanceKind::Direct,
                RoadEditingProvenanceKind::Generated => wire::ProvenanceKind::Generated,
            },
            generator_build_id: Some(generator_build_id),
            parameters_and_inputs_digest: Some(&inputs_digest),
            frontend_options_digest: Some(&options_digest),
            random_seed: random_seed.as_ref(),
            description: Some(description),
        },
    );
    let namespace = fbb.create_string(value.authoring_namespace_id());
    let source_document_key = fbb.create_string(value.source_document_key());
    let mut imports = value.imports().collect::<Vec<_>>();
    imports.sort_unstable();
    let imports = imports
        .into_iter()
        .map(|value| fbb.create_string(value))
        .collect::<Vec<_>>();
    let imports = fbb.create_vector(&imports);
    wire::ModuleHeader::create(
        fbb,
        &wire::ModuleHeaderArgs {
            authoring_namespace_id: Some(namespace),
            source_document_key: Some(source_document_key),
            imports: Some(imports),
            provenance: Some(provenance),
        },
    )
}

fn encode_curve_program<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &RoadEditingCurveProgram,
) -> runtime::WIPOffset<wire::CurveProgram<'fbb>> {
    let mut segments = Vec::with_capacity(value.segments().len());
    for segment in value.segments() {
        let canvas = create_canvas(fbb, segment.canvas_selection());
        let (geometry_type, geometry) = match segment.geometry() {
            RoadEditingCurveSegmentGeometry::Line { end } => {
                let end = encode_point(end);
                let value =
                    wire::LineSegment::create(fbb, &wire::LineSegmentArgs { end: Some(&end) });
                (
                    wire::CurveSegmentGeometry::LineSegment,
                    value.as_union_value(),
                )
            }
            RoadEditingCurveSegmentGeometry::CubicBezier {
                control_1,
                control_2,
                end,
            } => {
                let control_1 = encode_point(control_1);
                let control_2 = encode_point(control_2);
                let end = encode_point(end);
                let value = wire::CubicBezierSegment::create(
                    fbb,
                    &wire::CubicBezierSegmentArgs {
                        control_1: Some(&control_1),
                        control_2: Some(&control_2),
                        end: Some(&end),
                    },
                );
                (
                    wire::CurveSegmentGeometry::CubicBezierSegment,
                    value.as_union_value(),
                )
            }
        };
        segments.push(wire::CurveSegment::create(
            fbb,
            &wire::CurveSegmentArgs {
                geometry_type,
                geometry: Some(geometry),
                canvas_selection: canvas,
            },
        ));
    }
    let segments = fbb.create_vector(&segments);
    let start = encode_point(value.start());
    wire::CurveProgram::create(
        fbb,
        &wire::CurveProgramArgs {
            start: Some(&start),
            segments: Some(segments),
        },
    )
}

fn encode_road_alignment<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &RoadAlignmentInput,
) -> runtime::WIPOffset<wire::RoadAlignment<'fbb>> {
    let key = fbb.create_string(value.road_alignment_key());
    let frame = create_reference(fbb, value.canonical_frame());
    let reference_line = encode_curve_program(fbb, value.reference_line());
    let canvas = create_canvas(fbb, value.canvas_selection());
    wire::RoadAlignment::create(
        fbb,
        &wire::RoadAlignmentArgs {
            road_alignment_key: Some(key),
            canonical_frame: Some(frame),
            reference_line: Some(reference_line),
            canvas_selection: canvas,
        },
    )
}

fn encode_road_corridor<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &RoadCorridorInput,
) -> runtime::WIPOffset<wire::RoadCorridor<'fbb>> {
    let key = fbb.create_string(value.road_corridor_key());
    let alignment = fbb.create_string(value.road_alignment().key());
    let reference_section = create_reference(fbb, value.reference_section());
    let reference_lane = create_reference(fbb, value.reference_lane());
    let elements = value
        .elements()
        .iter()
        .map(|element| {
            let (kind, reference) = match element {
                RoadEditingCorridorElement::RoadSection(reference) => (
                    wire::CorridorElementKind::RoadSection,
                    create_reference(fbb, reference),
                ),
                RoadEditingCorridorElement::FacilityBand(reference) => (
                    wire::CorridorElementKind::FacilityBand,
                    create_reference(fbb, reference),
                ),
            };
            wire::CorridorElement::create(
                fbb,
                &wire::CorridorElementArgs {
                    kind,
                    entity_reference: Some(reference),
                },
            )
        })
        .collect::<Vec<_>>();
    let elements = fbb.create_vector(&elements);
    let canvas = create_canvas(fbb, value.canvas_selection());
    let (end_station_kind, end_station_meters) = match value.end_station() {
        RoadEditingStationEnd::Finite(value) => (wire::StationEndKind::Finite, value),
        RoadEditingStationEnd::AlignmentEnd => (wire::StationEndKind::AlignmentEnd, 0.0),
    };
    wire::RoadCorridor::create(
        fbb,
        &wire::RoadCorridorArgs {
            road_corridor_key: Some(key),
            road_alignment_key: Some(alignment),
            start_station_meters: value.start_station_meters(),
            end_station_kind,
            end_station_meters,
            reference_section: Some(reference_section),
            reference_lane: Some(reference_lane),
            elements: Some(elements),
            canvas_selection: canvas,
        },
    )
}

fn encode_road_section<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &RoadSectionInput,
) -> runtime::WIPOffset<wire::RoadSection<'fbb>> {
    let key = fbb.create_string(value.road_section_key());
    let kind_id = fbb.create_string(value.kind_id());
    let lanes = create_reference_vector(fbb, value.authoring_lanes());
    let corridor = create_reference(fbb, value.road_corridor());
    let canvas = create_canvas(fbb, value.canvas_selection());
    wire::RoadSection::create(
        fbb,
        &wire::RoadSectionArgs {
            road_section_key: Some(key),
            kind_id: Some(kind_id),
            authoring_lanes: Some(lanes),
            canvas_selection: canvas,
            road_corridor: Some(corridor),
        },
    )
}

fn encode_authoring_lane<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &AuthoringLaneInput,
) -> runtime::WIPOffset<wire::AuthoringLane<'fbb>> {
    let key = fbb.create_string(value.authoring_lane_key());
    let lane_edge = create_reference(fbb, value.lane_edge());
    let lane_group = value
        .lane_group()
        .map(|reference| create_reference(fbb, reference));
    let road_section = create_reference(fbb, value.road_section());
    let canvas = create_canvas(fbb, value.canvas_selection());
    let width = encode_width(value.width_profile());
    wire::AuthoringLane::create(
        fbb,
        &wire::AuthoringLaneArgs {
            authoring_lane_key: Some(key),
            lane_edge: Some(lane_edge),
            direction: match value.direction() {
                RoadEditingLaneDirection::Forward => wire::LaneDirection::Forward,
                RoadEditingLaneDirection::Backward => wire::LaneDirection::Backward,
            },
            width_profile: Some(&width),
            lane_group,
            canvas_selection: canvas,
            road_section: Some(road_section),
        },
    )
}

fn encode_lane_edge<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &LaneEdgeInput,
    current_namespace: &str,
) -> runtime::WIPOffset<wire::LaneEdge<'fbb>> {
    let key = fbb.create_string(value.lane_edge_key());
    let successors = create_sorted_reference_vector(fbb, value.successors(), current_namespace);
    let explicit_geometry = value
        .explicit_geometry()
        .map(|curve| encode_curve_program(fbb, curve));
    let canvas = create_canvas(fbb, value.canvas_selection());
    wire::LaneEdge::create(
        fbb,
        &wire::LaneEdgeArgs {
            lane_edge_key: Some(key),
            speed_limit_meters_per_second: value.speed_limit_meters_per_second(),
            successors: Some(successors),
            explicit_geometry,
            canvas_selection: canvas,
        },
    )
}

fn encode_junction<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &JunctionInput,
    current_namespace: &str,
) -> runtime::WIPOffset<wire::Junction<'fbb>> {
    let key = fbb.create_string(value.junction_key());
    let approach_edges =
        create_sorted_reference_vector(fbb, value.approach_edges(), current_namespace);
    let internal_edges =
        create_sorted_reference_vector(fbb, value.internal_edges(), current_namespace);
    let canvas = create_canvas(fbb, value.canvas_selection());
    wire::Junction::create(
        fbb,
        &wire::JunctionArgs {
            junction_key: Some(key),
            approach_edges: Some(approach_edges),
            internal_edges: Some(internal_edges),
            canvas_selection: canvas,
        },
    )
}

fn encode_movement<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &MovementInput,
) -> runtime::WIPOffset<wire::Movement<'fbb>> {
    let key = fbb.create_string(value.movement_key());
    let junction = create_reference(fbb, value.junction());
    let entry = fbb.create_string(value.directed_entry_approach_key());
    let exit = fbb.create_string(value.directed_exit_approach_key());
    let canvas = create_canvas(fbb, value.canvas_selection());
    wire::Movement::create(
        fbb,
        &wire::MovementArgs {
            movement_key: Some(key),
            junction: Some(junction),
            directed_entry_approach_key: Some(entry),
            directed_exit_approach_key: Some(exit),
            canvas_selection: canvas,
        },
    )
}

fn encode_maneuver_path<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &ManeuverPathInput,
) -> runtime::WIPOffset<wire::ManeuverPath<'fbb>> {
    let key = fbb.create_string(value.maneuver_path_key());
    let movement = create_reference(fbb, value.movement());
    let entry = create_reference(fbb, value.entry_edge());
    let internal = create_reference_vector(fbb, value.internal_edges());
    let exit = create_reference(fbb, value.exit_edge());
    let canvas = create_canvas(fbb, value.canvas_selection());
    wire::ManeuverPath::create(
        fbb,
        &wire::ManeuverPathArgs {
            maneuver_path_key: Some(key),
            movement: Some(movement),
            entry_edge: Some(entry),
            internal_edges: Some(internal),
            exit_edge: Some(exit),
            canvas_selection: canvas,
        },
    )
}

fn encode_maneuver_gate<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &ManeuverGateInput,
) -> runtime::WIPOffset<wire::ManeuverGate<'fbb>> {
    let key = fbb.create_string(value.maneuver_gate_key());
    let path = create_reference(fbb, value.maneuver_path());
    let stop_line = create_reference(fbb, value.stop_line());
    let (signal_control, signal_group) = match value.signal_control() {
        RoadEditingSignalControl::None => (wire::SignalControlKind::None, None),
        RoadEditingSignalControl::SignalGroup(reference) => (
            wire::SignalControlKind::SignalGroup,
            Some(create_reference(fbb, reference)),
        ),
    };
    let canvas = create_canvas(fbb, value.canvas_selection());
    wire::ManeuverGate::create(
        fbb,
        &wire::ManeuverGateArgs {
            maneuver_gate_key: Some(key),
            maneuver_path: Some(path),
            transition_index: value.transition_index(),
            stop_line: Some(stop_line),
            signal_control,
            signal_group,
            canvas_selection: canvas,
        },
    )
}

fn encode_waiting_zone<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &WaitingZoneInput,
) -> runtime::WIPOffset<wire::WaitingZone<'fbb>> {
    let key = fbb.create_string(value.waiting_zone_key());
    let path = create_reference(fbb, value.maneuver_path());
    let entry = create_reference(fbb, value.entry_gate());
    let release = create_reference(fbb, value.release_gate());
    let canvas = create_canvas(fbb, value.canvas_selection());
    wire::WaitingZone::create(
        fbb,
        &wire::WaitingZoneArgs {
            waiting_zone_key: Some(key),
            maneuver_path: Some(path),
            entry_gate: Some(entry),
            release_gate: Some(release),
            max_occupancy: value.max_occupancy(),
            canvas_selection: canvas,
        },
    )
}

fn encode_stop_line<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &StopLineInput,
) -> runtime::WIPOffset<wire::StopLine<'fbb>> {
    let key = fbb.create_string(value.stop_line_key());
    let edge = create_reference(fbb, value.lane_edge());
    let canvas = create_canvas(fbb, value.canvas_selection());
    wire::StopLine::create(
        fbb,
        &wire::StopLineArgs {
            stop_line_key: Some(key),
            lane_edge: Some(edge),
            canvas_selection: canvas,
        },
    )
}

fn encode_signal_group<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &SignalGroupInput,
) -> runtime::WIPOffset<wire::SignalGroup<'fbb>> {
    let key = fbb.create_string(value.signal_group_key());
    let canvas = create_canvas(fbb, value.canvas_selection());
    wire::SignalGroup::create(
        fbb,
        &wire::SignalGroupArgs {
            signal_group_key: Some(key),
            canvas_selection: canvas,
        },
    )
}

fn encode_signal_controller<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &SignalControllerInput,
    current_namespace: &str,
) -> runtime::WIPOffset<wire::SignalController<'fbb>> {
    let key = fbb.create_string(value.signal_controller_key());
    let groups = create_sorted_reference_vector(fbb, value.signal_groups(), current_namespace);
    let phases = create_reference_vector(fbb, value.signal_phases());
    let canvas = create_canvas(fbb, value.canvas_selection());
    wire::SignalController::create(
        fbb,
        &wire::SignalControllerArgs {
            signal_controller_key: Some(key),
            offset_milliseconds: value.offset_milliseconds(),
            signal_groups: Some(groups),
            signal_phases: Some(phases),
            canvas_selection: canvas,
        },
    )
}

fn encode_signal_phase<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &SignalPhaseInput,
    current_namespace: &str,
) -> runtime::WIPOffset<wire::SignalPhase<'fbb>> {
    let key = fbb.create_string(value.signal_phase_key());
    let mut states = value.states().iter().collect::<Vec<_>>();
    states.sort_unstable_by(|left, right| {
        left.signal_group()
            .canonical_target_cmp(right.signal_group(), current_namespace)
    });
    let states = states
        .into_iter()
        .map(|state| {
            let group = create_reference(fbb, state.signal_group());
            wire::SignalPhaseState::create(
                fbb,
                &wire::SignalPhaseStateArgs {
                    signal_group: Some(group),
                    aspect: match state.aspect() {
                        SignalAspect::Red => wire::SignalAspect::Red,
                        SignalAspect::Yellow => wire::SignalAspect::Yellow,
                        SignalAspect::Green => wire::SignalAspect::Green,
                        _ => unreachable!("authoring model rejected unknown signal aspect"),
                    },
                },
            )
        })
        .collect::<Vec<_>>();
    let states = fbb.create_vector(&states);
    let controller = create_reference(fbb, value.signal_controller());
    let canvas = create_canvas(fbb, value.canvas_selection());
    wire::SignalPhase::create(
        fbb,
        &wire::SignalPhaseArgs {
            signal_phase_key: Some(key),
            duration_milliseconds: value.duration_milliseconds(),
            states: Some(states),
            canvas_selection: canvas,
            signal_controller: Some(controller),
        },
    )
}

fn encode_parking_area<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &ParkingAreaInput,
) -> runtime::WIPOffset<wire::ParkingArea<'fbb>> {
    let key = fbb.create_string(value.parking_area_key());
    let canvas = create_canvas(fbb, value.canvas_selection());
    wire::ParkingArea::create(
        fbb,
        &wire::ParkingAreaArgs {
            parking_area_key: Some(key),
            canvas_selection: canvas,
        },
    )
}

fn encode_parking_anchor<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &ParkingLaneAnchor,
) -> runtime::WIPOffset<wire::ParkingLaneAnchor<'fbb>> {
    let edge = create_reference(fbb, value.lane_edge());
    wire::ParkingLaneAnchor::create(
        fbb,
        &wire::ParkingLaneAnchorArgs {
            lane_edge: Some(edge),
            progress_meters: value.progress_meters(),
        },
    )
}

fn encode_parking_space<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &ParkingSpaceInput,
) -> runtime::WIPOffset<wire::ParkingSpace<'fbb>> {
    let key = fbb.create_string(value.parking_space_key());
    let area = value
        .parking_area()
        .map(|reference| create_reference(fbb, reference));
    let entry = encode_parking_anchor(fbb, value.entry());
    let exit = encode_parking_anchor(fbb, value.exit());
    let geometry = value.geometry();
    let geometry = wire::ParkingSpaceGeometry::create(
        fbb,
        &wire::ParkingSpaceGeometryArgs {
            lateral_offset_meters: geometry.lateral_offset_meters(),
            heading_offset_radians: geometry.heading_offset_radians(),
            length_meters: geometry.length_meters(),
            width_meters: geometry.width_meters(),
        },
    );
    let canvas = create_canvas(fbb, value.canvas_selection());
    wire::ParkingSpace::create(
        fbb,
        &wire::ParkingSpaceArgs {
            parking_space_key: Some(key),
            parking_area: area,
            entry: Some(entry),
            exit: Some(exit),
            geometry: Some(geometry),
            canvas_selection: canvas,
        },
    )
}

fn encode_lane_group<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &LaneGroupInput,
) -> runtime::WIPOffset<wire::LaneGroup<'fbb>> {
    let key = fbb.create_string(value.lane_group_key());
    let section = create_reference(fbb, value.road_section());
    let canvas = create_canvas(fbb, value.canvas_selection());
    wire::LaneGroup::create(
        fbb,
        &wire::LaneGroupArgs {
            lane_group_key: Some(key),
            road_section: Some(section),
            canvas_selection: canvas,
        },
    )
}

fn encode_facility_band<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &FacilityBandInput,
) -> runtime::WIPOffset<wire::FacilityBand<'fbb>> {
    let key = fbb.create_string(value.facility_band_key());
    let kind_id = fbb.create_string(value.kind_id());
    let corridor = create_reference(fbb, value.road_corridor());
    let width = encode_width(value.width_profile());
    let canvas = create_canvas(fbb, value.canvas_selection());
    wire::FacilityBand::create(
        fbb,
        &wire::FacilityBandArgs {
            facility_band_key: Some(key),
            kind_id: Some(kind_id),
            width_profile: Some(&width),
            canvas_selection: canvas,
            road_corridor: Some(corridor),
        },
    )
}

fn encode_participant_class<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &ParticipantClassInput,
) -> runtime::WIPOffset<wire::ParticipantClass<'fbb>> {
    let key = fbb.create_string(value.participant_class_key());
    let extends = value
        .extends()
        .map(|reference| create_reference(fbb, reference));
    let canvas = create_canvas(fbb, value.canvas_selection());
    wire::ParticipantClass::create(
        fbb,
        &wire::ParticipantClassArgs {
            participant_class_key: Some(key),
            extends,
            canvas_selection: canvas,
        },
    )
}

fn encode_access_regulation<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &AccessRegulationInput,
) -> runtime::WIPOffset<wire::AccessRegulation<'fbb>> {
    let jurisdiction = fbb.create_string(value.jurisdiction());
    let version = fbb.create_string(value.version());
    let source = value.source().map(|value| fbb.create_string(value));
    wire::AccessRegulation::create(
        fbb,
        &wire::AccessRegulationArgs {
            jurisdiction: Some(jurisdiction),
            version: Some(version),
            source,
        },
    )
}

fn encode_access_rule<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &AccessRuleInput,
    current_namespace: &str,
) -> runtime::WIPOffset<wire::AccessRule<'fbb>> {
    let key = fbb.create_string(value.access_rule_key());
    let (target_kind, target_reference) = match value.target() {
        RoadEditingAccessTarget::LaneEdge(reference) => (
            wire::AccessTargetKind::LaneEdge,
            create_reference(fbb, reference),
        ),
        RoadEditingAccessTarget::LaneGroup(reference) => (
            wire::AccessTargetKind::LaneGroup,
            create_reference(fbb, reference),
        ),
        RoadEditingAccessTarget::RoadSection(reference) => (
            wire::AccessTargetKind::RoadSection,
            create_reference(fbb, reference),
        ),
        RoadEditingAccessTarget::ManeuverPath(reference) => (
            wire::AccessTargetKind::ManeuverPath,
            create_reference(fbb, reference),
        ),
    };
    let participants =
        create_sorted_reference_vector(fbb, value.participant_classes(), current_namespace);
    let regulation = value
        .regulation()
        .map(|value| encode_access_regulation(fbb, value));
    let canvas = create_canvas(fbb, value.canvas_selection());
    wire::AccessRule::create(
        fbb,
        &wire::AccessRuleArgs {
            access_rule_key: Some(key),
            target_kind,
            target_reference: Some(target_reference),
            effect: match value.effect() {
                AccessEffect::Allow => wire::AccessEffect::Allow,
                AccessEffect::Deny => wire::AccessEffect::Deny,
                _ => unreachable!("authoring model rejected unknown access effect"),
            },
            participant_classes: Some(participants),
            regulation,
            priority: value.priority(),
            canvas_selection: canvas,
        },
    )
}

fn encode_vehicle_profile<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &VehicleProfileInput,
) -> runtime::WIPOffset<wire::VehicleProfile<'fbb>> {
    let key = fbb.create_string(value.vehicle_profile_key());
    let participant = create_reference(fbb, value.participant_class());
    let iidm = value.iidm();
    let iidm = wire::IidmVehicleProfile::create(
        fbb,
        &wire::IidmVehicleProfileArgs {
            length_meters: iidm.length_meters(),
            desired_speed_meters_per_second: iidm.desired_speed_meters_per_second(),
            min_gap_meters: iidm.min_gap_meters(),
            time_headway_seconds: iidm.time_headway_seconds(),
            max_acceleration_meters_per_second_squared: iidm
                .max_acceleration_meters_per_second_squared(),
            comfortable_deceleration_meters_per_second_squared: iidm
                .comfortable_deceleration_meters_per_second_squared(),
            emergency_deceleration_meters_per_second_squared: iidm
                .emergency_deceleration_meters_per_second_squared(),
        },
    );
    let canvas = create_canvas(fbb, value.canvas_selection());
    wire::VehicleProfile::create(
        fbb,
        &wire::VehicleProfileArgs {
            vehicle_profile_key: Some(key),
            participant_class: Some(participant),
            iidm: Some(iidm),
            canvas_selection: canvas,
        },
    )
}

fn encode_canonical_frame<'fbb>(
    fbb: &mut runtime::FlatBufferBuilder<'fbb>,
    value: &CanonicalFrameInput,
) -> runtime::WIPOffset<wire::CanonicalFrame<'fbb>> {
    let key = fbb.create_string(value.canonical_frame_key());
    let canvas = create_canvas(fbb, value.canvas_selection());
    wire::CanonicalFrame::create(
        fbb,
        &wire::CanonicalFrameArgs {
            canonical_frame_key: Some(key),
            canvas_selection: canvas,
        },
    )
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::road_editing::RoadEditingSourceModuleBuilder;

    fn module_with_frames(limits: &CompileLimits, keys: &[&str]) -> RoadEditingSourceModule {
        let header = RoadEditingModuleHeader::try_new(
            "city",
            "road-editing",
            Vec::new(),
            RoadEditingProvenance::direct("editor save").expect("provenance"),
        )
        .expect("header");
        let mut builder = RoadEditingSourceModuleBuilder::new(
            header,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            limits,
        )
        .expect("builder");
        for key in keys {
            builder
                .add_declaration(RoadEditingDeclaration::CanonicalFrame(
                    CanonicalFrameInput::try_new(*key).expect("frame"),
                ))
                .expect("declaration");
        }
        builder.finish().expect("module")
    }

    fn module_with_junction(
        limits: &CompileLimits,
        approach_keys: &[&str],
    ) -> RoadEditingSourceModule {
        let header = RoadEditingModuleHeader::try_new(
            "city",
            "road-editing",
            Vec::new(),
            RoadEditingProvenance::direct("editor save").expect("provenance"),
        )
        .expect("header");
        let mut builder = RoadEditingSourceModuleBuilder::new(
            header,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            limits,
        )
        .expect("builder");
        for key in ["edge-a", "edge-b", "edge-internal"] {
            builder
                .add_declaration(RoadEditingDeclaration::LaneEdge(
                    LaneEdgeInput::try_new(key, 10.0, Vec::new(), None).expect("edge"),
                ))
                .expect("edge declaration");
        }
        let approaches = approach_keys
            .iter()
            .map(|key| LaneEdgeReference::local(*key).expect("approach"))
            .collect();
        builder
            .add_declaration(RoadEditingDeclaration::Junction(
                JunctionInput::try_new(
                    "junction-a",
                    approaches,
                    vec![LaneEdgeReference::local("edge-internal").expect("internal")],
                )
                .expect("junction"),
            ))
            .expect("junction declaration");
        builder.finish().expect("module")
    }

    fn minimal_curve() -> RoadEditingCurveProgram {
        RoadEditingCurveProgram::try_new(
            RoadEditingPoint3::try_new(0.0, 0.0, 0.0).expect("start"),
            vec![RoadEditingCurveSegment::line(
                RoadEditingPoint3::try_new(10.0, 0.0, 0.0).expect("end"),
            )],
        )
        .expect("curve")
    }

    fn minimal_curve_with_segment_canvas() -> RoadEditingCurveProgram {
        RoadEditingCurveProgram::try_new(
            RoadEditingPoint3::try_new(0.0, 0.0, 0.0).expect("start"),
            vec![
                RoadEditingCurveSegment::line(
                    RoadEditingPoint3::try_new(10.0, 0.0, 0.0).expect("end"),
                )
                .with_canvas_selection("canvas/alignment-segment")
                .expect("segment canvas"),
            ],
        )
        .expect("curve")
    }

    pub(crate) fn module_with_every_declaration(limits: &CompileLimits) -> RoadEditingSourceModule {
        let mut builder = RoadEditingSourceModuleBuilder::new(
            RoadEditingModuleHeader::try_new(
                "city",
                "road-editing",
                Vec::new(),
                RoadEditingProvenance::direct("editor save").expect("provenance"),
            )
            .expect("header"),
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            limits,
        )
        .expect("builder");

        let corridor = RoadCorridorReference::local("corridor").expect("corridor ref");
        let section = RoadSectionReference::owner_scoped(vec!["corridor".into()], "section")
            .expect("section ref");
        let lane =
            AuthoringLaneReference::owner_scoped(vec!["corridor".into(), "section".into()], "lane")
                .expect("lane ref");
        let facility = FacilityBandReference::owner_scoped(vec!["corridor".into()], "facility")
            .expect("facility ref");
        let lane_group = LaneGroupReference::owner_scoped(
            vec!["corridor".into(), "section".into()],
            "lane-group",
        )
        .expect("lane group ref");
        let edge_a = LaneEdgeReference::local("edge-a").expect("edge a");
        let edge_b = LaneEdgeReference::local("edge-b").expect("edge b");
        let edge_internal = LaneEdgeReference::local("edge-internal").expect("internal edge");
        let junction = JunctionReference::local("junction").expect("junction ref");
        let movement = MovementReference::owner_scoped(vec!["junction".into()], "movement")
            .expect("movement ref");
        let path =
            ManeuverPathReference::owner_scoped(vec!["junction".into(), "movement".into()], "path")
                .expect("path ref");
        let gate = ManeuverGateReference::owner_scoped(
            vec!["junction".into(), "movement".into(), "path".into()],
            "gate",
        )
        .expect("gate ref");
        let stop_line = StopLineReference::local("stop").expect("stop ref");
        let signal_group = SignalGroupReference::local("signal-group").expect("group ref");
        let controller = SignalControllerReference::local("controller").expect("controller ref");
        let phase = SignalPhaseReference::owner_scoped(vec!["controller".into()], "phase")
            .expect("phase ref");
        let participant = ParticipantClassReference::local("car").expect("participant ref");

        builder
            .add_alignment(
                RoadAlignmentInput::try_new(
                    "alignment",
                    CanonicalFrameReference::local("frame").expect("frame ref"),
                    minimal_curve_with_segment_canvas(),
                )
                .expect("alignment"),
            )
            .expect("add alignment");

        let declarations = vec![
            RoadEditingDeclaration::CanonicalFrame(
                CanonicalFrameInput::try_new("frame").expect("frame"),
            ),
            RoadEditingDeclaration::RoadCorridor(
                RoadCorridorInput::try_new(
                    "corridor",
                    RoadAlignmentReference::try_new("alignment").expect("alignment ref"),
                    0.0,
                    RoadEditingStationEnd::AlignmentEnd,
                    section.clone(),
                    lane.clone(),
                    vec![
                        RoadEditingCorridorElement::RoadSection(section.clone()),
                        RoadEditingCorridorElement::FacilityBand(facility),
                    ],
                )
                .expect("corridor"),
            ),
            RoadEditingDeclaration::RoadSection(
                RoadSectionInput::try_new("section", "motorLane", vec![lane], corridor.clone())
                    .expect("section"),
            ),
            RoadEditingDeclaration::AuthoringLane(
                AuthoringLaneInput::try_new(
                    "lane",
                    edge_a.clone(),
                    RoadEditingLaneDirection::Forward,
                    LinearWidthProfile::try_new(3.5, 3.5).expect("lane width"),
                    Some(lane_group.clone()),
                    section.clone(),
                )
                .expect("lane"),
            ),
            RoadEditingDeclaration::LaneEdge(
                LaneEdgeInput::try_new("edge-a", 10.0, vec![edge_b.clone()], None).expect("edge a"),
            ),
            RoadEditingDeclaration::LaneEdge(
                LaneEdgeInput::try_new("edge-b", 10.0, Vec::new(), None).expect("edge b"),
            ),
            RoadEditingDeclaration::LaneEdge(
                LaneEdgeInput::try_new("edge-internal", 8.0, Vec::new(), Some(minimal_curve()))
                    .expect("internal edge"),
            ),
            RoadEditingDeclaration::Junction(
                JunctionInput::try_new(
                    "junction",
                    vec![edge_a.clone(), edge_b.clone()],
                    vec![edge_internal.clone()],
                )
                .expect("junction"),
            ),
            RoadEditingDeclaration::Movement(
                MovementInput::try_new("movement", junction, "entry", "exit").expect("movement"),
            ),
            RoadEditingDeclaration::ManeuverPath(
                ManeuverPathInput::try_new(
                    "path",
                    movement,
                    edge_a.clone(),
                    vec![edge_internal],
                    edge_b.clone(),
                )
                .expect("path"),
            ),
            RoadEditingDeclaration::StopLine(
                StopLineInput::try_new("stop", edge_a.clone()).expect("stop line"),
            ),
            RoadEditingDeclaration::SignalGroup(
                SignalGroupInput::try_new("signal-group").expect("signal group"),
            ),
            RoadEditingDeclaration::SignalController(
                SignalControllerInput::try_new(
                    "controller",
                    0,
                    vec![signal_group.clone()],
                    vec![phase],
                )
                .expect("controller"),
            ),
            RoadEditingDeclaration::SignalPhase(
                SignalPhaseInput::try_new(
                    "phase",
                    1_000,
                    vec![
                        RoadEditingSignalPhaseState::try_new(
                            signal_group.clone(),
                            SignalAspect::Green,
                        )
                        .expect("phase state"),
                    ],
                    controller,
                )
                .expect("phase"),
            ),
            RoadEditingDeclaration::ManeuverGate(
                ManeuverGateInput::try_new(
                    "gate",
                    path.clone(),
                    0,
                    stop_line,
                    RoadEditingSignalControl::SignalGroup(signal_group),
                )
                .expect("gate"),
            ),
            RoadEditingDeclaration::WaitingZone(
                WaitingZoneInput::try_new("waiting", path, gate.clone(), gate, 1).expect("waiting"),
            ),
            RoadEditingDeclaration::ParkingArea(
                ParkingAreaInput::try_new("parking-area").expect("parking area"),
            ),
            RoadEditingDeclaration::ParkingSpace(
                ParkingSpaceInput::try_new(
                    "parking-space",
                    ParkingLaneAnchor::try_new(edge_a.clone(), 1.0).expect("entry anchor"),
                    ParkingLaneAnchor::try_new(edge_b.clone(), 1.0).expect("exit anchor"),
                    ParkingSpaceGeometry::try_new(2.0, 0.0, 5.0, 2.5).expect("parking geometry"),
                )
                .expect("parking space")
                .with_parking_area(
                    ParkingAreaReference::local("parking-area").expect("parking area ref"),
                ),
            ),
            RoadEditingDeclaration::LaneGroup(
                LaneGroupInput::try_new("lane-group", section).expect("lane group"),
            ),
            RoadEditingDeclaration::FacilityBand(
                FacilityBandInput::try_new(
                    "facility",
                    "median",
                    LinearWidthProfile::try_new(1.0, 1.0).expect("facility width"),
                    corridor,
                )
                .expect("facility"),
            ),
            RoadEditingDeclaration::ParticipantClass(
                ParticipantClassInput::try_new("car").expect("participant"),
            ),
            RoadEditingDeclaration::AccessRule(
                AccessRuleInput::try_new(
                    "access",
                    RoadEditingAccessTarget::LaneEdge(edge_a.clone()),
                    AccessEffect::Allow,
                    vec![participant.clone()],
                    0,
                )
                .expect("access rule")
                .with_regulation(AccessRegulationInput::try_new("test", "v1").expect("regulation")),
            ),
            RoadEditingDeclaration::VehicleProfile(
                VehicleProfileInput::try_new(
                    "vehicle",
                    participant,
                    IidmVehicleProfileInput::try_new(4.5, 13.0, 2.0, 1.5, 1.5, 2.0, 4.0)
                        .expect("iidm"),
                )
                .expect("vehicle"),
            ),
        ];
        for declaration in declarations {
            builder
                .add_declaration(declaration)
                .expect("add declaration");
        }
        builder.finish().expect("module")
    }

    #[test]
    fn writer_emits_checked_size_prefixed_lfre_buffer() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = RoadEditingSourceWriter::new(&limits)
            .write(module_with_frames(&limits, &["frame-b", "frame-a"]))
            .expect("buffer");
        let bytes = buffer.as_bytes();
        let prefix = u32::from_le_bytes(bytes[0..4].try_into().expect("size prefix"));

        assert_eq!(
            usize::try_from(prefix).expect("portable size"),
            bytes.len() - 4
        );
        assert!(wire::road_editing_source_size_prefixed_buffer_has_identifier(bytes));
        let root = wire::size_prefixed_root_as_road_editing_source(bytes).expect("verified root");
        assert_eq!(root.format_version(), FORMAT_VERSION);
        assert_eq!(
            root.geometry_accuracy_profile(),
            wire::GeometryAccuracyProfile::Balanced5Cm
        );
        assert_eq!(
            root.geometry_direction_profile(),
            wire::GeometryDirectionProfile::Balanced2Deg
        );
        let frames = root.canonical_frames();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames.get(0).canonical_frame_key(), "frame-a");
        assert_eq!(frames.get(1).canonical_frame_key(), "frame-b");
        assert!(buffer.retained_capacity_bytes() >= bytes.len());
        assert!(buffer.retained_capacity_bytes() < 10_000);
    }

    #[test]
    fn root_input_order_does_not_change_wire_bytes() {
        let limits = CompileLimits::p100_initial_v1();
        let first = RoadEditingSourceWriter::new(&limits)
            .write(module_with_frames(
                &limits,
                &["frame-c", "frame-a", "frame-b"],
            ))
            .expect("first buffer");
        let second = RoadEditingSourceWriter::new(&limits)
            .write(module_with_frames(
                &limits,
                &["frame-b", "frame-c", "frame-a"],
            ))
            .expect("second buffer");

        assert_eq!(first.as_bytes(), second.as_bytes());
    }

    #[test]
    fn unordered_reference_input_does_not_change_wire_bytes() {
        let limits = CompileLimits::p100_initial_v1();
        let first = RoadEditingSourceWriter::new(&limits)
            .write(module_with_junction(&limits, &["edge-b", "edge-a"]))
            .expect("first buffer");
        let second = RoadEditingSourceWriter::new(&limits)
            .write(module_with_junction(&limits, &["edge-a", "edge-b"]))
            .expect("second buffer");

        assert_eq!(first.as_bytes(), second.as_bytes());
    }

    #[test]
    fn every_stable_declaration_encoder_produces_a_verified_root_vector() {
        let limits = CompileLimits::p100_initial_v1();
        let buffer = RoadEditingSourceWriter::new(&limits)
            .write(module_with_every_declaration(&limits))
            .expect("complete buffer");
        let root = wire::size_prefixed_root_as_road_editing_source(buffer.as_bytes())
            .expect("verified root");

        assert_eq!(root.road_corridors().len(), 1);
        assert_eq!(root.road_sections().len(), 1);
        assert_eq!(root.authoring_lanes().len(), 1);
        assert_eq!(root.lane_edges().len(), 3);
        assert_eq!(root.junctions().len(), 1);
        assert_eq!(root.movements().len(), 1);
        assert_eq!(root.maneuver_paths().len(), 1);
        assert_eq!(root.maneuver_gates().len(), 1);
        assert_eq!(root.waiting_zones().len(), 1);
        assert_eq!(root.stop_lines().len(), 1);
        assert_eq!(root.signal_groups().len(), 1);
        assert_eq!(root.signal_controllers().len(), 1);
        assert_eq!(root.signal_phases().len(), 1);
        assert_eq!(root.parking_areas().len(), 1);
        assert_eq!(root.parking_spaces().len(), 1);
        assert_eq!(root.lane_groups().len(), 1);
        assert_eq!(root.facility_bands().len(), 1);
        assert_eq!(root.participant_classes().len(), 1);
        assert_eq!(root.access_rules().len(), 1);
        assert_eq!(root.vehicle_profiles().len(), 1);
        assert_eq!(root.canonical_frames().len(), 1);
    }

    #[test]
    fn unordered_references_sort_by_resolved_namespace() {
        let limits = CompileLimits::p100_initial_v1();
        let header = RoadEditingModuleHeader::try_new(
            "z-city",
            "road-editing",
            vec!["a-city".into()],
            RoadEditingProvenance::direct("editor save").expect("provenance"),
        )
        .expect("header");
        let mut builder = RoadEditingSourceModuleBuilder::new(
            header,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            &limits,
        )
        .expect("builder");
        builder
            .add_declaration(RoadEditingDeclaration::LaneEdge(
                LaneEdgeInput::try_new("edge-z", 10.0, Vec::new(), None).expect("local target"),
            ))
            .expect("local target declaration");
        builder
            .add_declaration(RoadEditingDeclaration::LaneEdge(
                LaneEdgeInput::try_new(
                    "source",
                    10.0,
                    vec![
                        LaneEdgeReference::local("edge-z").expect("local reference"),
                        LaneEdgeReference::imported("a-city", Vec::new(), "edge-a")
                            .expect("imported reference"),
                    ],
                    None,
                )
                .expect("source edge"),
            ))
            .expect("source declaration");
        let buffer = RoadEditingSourceWriter::new(&limits)
            .write(builder.finish().expect("module"))
            .expect("buffer");
        let root = wire::size_prefixed_root_as_road_editing_source(buffer.as_bytes())
            .expect("verified root");
        let source = root
            .lane_edges()
            .iter()
            .find(|edge| edge.lane_edge_key() == "source")
            .expect("source edge");
        let successors = source.successors();

        assert_eq!(successors.get(0), "a-city::edge-a");
        assert_eq!(successors.get(1), "edge-z");
    }
}
