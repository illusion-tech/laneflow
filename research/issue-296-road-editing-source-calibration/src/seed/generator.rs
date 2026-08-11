use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;
use std::time::{Duration, Instant};

use laneflow_compiler::road_editing::{
    AccessRegulationInput, AccessRuleInput, AuthoringLaneInput, CanonicalFrameInput,
    FacilityBandInput, IidmVehicleProfileInput, JunctionInput, LaneEdgeInput, LaneEdgeReference,
    LaneGroupInput, LaneGroupReference, LinearWidthProfile, ManeuverGateInput,
    ManeuverGateReference, ManeuverPathInput, ManeuverPathReference, MovementInput,
    ParkingAreaInput, ParkingAreaReference, ParkingLaneAnchor, ParkingSpaceGeometry,
    ParkingSpaceInput, ParticipantClassInput, ParticipantClassReference, RoadAlignmentInput,
    RoadAlignmentReference, RoadCorridorInput, RoadCorridorReference, RoadEditingAccessTarget,
    RoadEditingCorridorElement, RoadEditingCurveProgram, RoadEditingCurveSegment,
    RoadEditingDeclaration, RoadEditingLaneDirection, RoadEditingModuleHeader,
    RoadEditingModuleInput, RoadEditingPoint3, RoadEditingProvenance, RoadEditingSignalControl,
    RoadEditingSignalPhaseState, RoadEditingSourceModule, RoadEditingSourceModuleBuilder,
    RoadEditingSourceWriter, RoadEditingStationEnd, RoadSectionInput, RoadSectionReference,
    SignalControllerInput, SignalControllerReference, SignalGroupInput, SignalGroupReference,
    SignalPhaseInput, SignalPhaseReference, StaticRouteInput, StopLineInput, StopLineReference,
    VehicleProfileInput, WaitingZoneInput,
};
use laneflow_compiler::{
    AccessEffect, CompilationOutput, CompilationUnitBuilder, CompileLimits, Compiler,
    DiagnosticBundle, GeometryAccuracyProfile, GeometryDirectionProfile, SignalAspect,
};
use sha2::{Digest, Sha256};

use super::*;

const GENERATOR_BUILD_ID: &str = "laneflow-road-editing-p100-generator-v1";
const PARAMETERS_PREFIX: &[u8] = b"laneflow.road-editing.p100.parameters-and-inputs.v1\0";
const FRONTEND_OPTIONS_PREFIX: &[u8] = b"laneflow.road-editing.p100.frontend-options.v1\0";
const REGULARITY_MODULE_INDEX: usize = 0;
const REGULARITY_ALIGNMENT_KEY: &str = "corridor-main-road-0/road";

/// 冻结 workload 中一组位置档与方向档组合。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct P100ProfileCombination {
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
}

impl P100ProfileCombination {
    #[must_use]
    pub const fn accuracy(self) -> GeometryAccuracyProfile {
        self.accuracy
    }

    #[must_use]
    pub const fn direction(self) -> GeometryDirectionProfile {
        self.direction
    }
}

/// `LF-ROAD-EDITING-P100-v1` 冻结的九种组合和枚举顺序。
pub const P100_PROFILE_COMBINATIONS: [P100ProfileCombination; 9] = [
    P100ProfileCombination {
        accuracy: GeometryAccuracyProfile::Fine2Cm,
        direction: GeometryDirectionProfile::Smooth1Deg,
    },
    P100ProfileCombination {
        accuracy: GeometryAccuracyProfile::Fine2Cm,
        direction: GeometryDirectionProfile::Balanced2Deg,
    },
    P100ProfileCombination {
        accuracy: GeometryAccuracyProfile::Fine2Cm,
        direction: GeometryDirectionProfile::Compact5Deg,
    },
    P100ProfileCombination {
        accuracy: GeometryAccuracyProfile::Balanced5Cm,
        direction: GeometryDirectionProfile::Smooth1Deg,
    },
    P100ProfileCombination {
        accuracy: GeometryAccuracyProfile::Balanced5Cm,
        direction: GeometryDirectionProfile::Balanced2Deg,
    },
    P100ProfileCombination {
        accuracy: GeometryAccuracyProfile::Balanced5Cm,
        direction: GeometryDirectionProfile::Compact5Deg,
    },
    P100ProfileCombination {
        accuracy: GeometryAccuracyProfile::Compact10Cm,
        direction: GeometryDirectionProfile::Smooth1Deg,
    },
    P100ProfileCombination {
        accuracy: GeometryAccuracyProfile::Compact10Cm,
        direction: GeometryDirectionProfile::Balanced2Deg,
    },
    P100ProfileCombination {
        accuracy: GeometryAccuracyProfile::Compact10Cm,
        direction: GeometryDirectionProfile::Compact5Deg,
    },
];

#[derive(Clone, Copy)]
enum P100Variant {
    Base,
    RegularityProbe,
}

pub struct TypedP100Module {
    module_index: u8,
    module: RoadEditingSourceModule,
}

impl TypedP100Module {
    #[must_use]
    pub const fn module_index(&self) -> u8 {
        self.module_index
    }

    #[must_use]
    pub fn namespace(&self) -> &str {
        self.module.header().authoring_namespace_id()
    }

    #[must_use]
    pub fn source_document_key(&self) -> &str {
        self.module.header().source_document_key()
    }

    #[must_use]
    pub const fn module(&self) -> &RoadEditingSourceModule {
        &self.module
    }

    #[must_use]
    pub fn into_module(self) -> RoadEditingSourceModule {
        self.module
    }
}

pub struct EncodedP100Module {
    module_index: u8,
    namespace: Box<str>,
    source_document_key: Box<str>,
    buffer: laneflow_compiler::road_editing::OwnedRoadEditingSourceBuffer,
}

impl EncodedP100Module {
    #[must_use]
    pub const fn module_index(&self) -> u8 {
        self.module_index
    }

    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    #[must_use]
    pub fn source_document_key(&self) -> &str {
        &self.source_document_key
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.buffer.as_bytes()
    }

    #[must_use]
    pub fn retained_capacity_bytes(&self) -> usize {
        self.buffer.retained_capacity_bytes()
    }

    #[must_use]
    pub fn sha256(&self) -> [u8; 32] {
        Sha256::digest(self.as_bytes()).into()
    }
}

#[derive(Debug)]
pub enum GeneratorError {
    Seed(SeedError),
    Model(DiagnosticBundle),
    Contract(String),
}

impl fmt::Display for GeneratorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Seed(error) => error.fmt(formatter),
            Self::Model(error) => error.fmt(formatter),
            Self::Contract(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for GeneratorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Seed(error) => Some(error),
            Self::Model(error) => Some(error),
            Self::Contract(_) => None,
        }
    }
}

impl From<SeedError> for GeneratorError {
    fn from(value: SeedError) -> Self {
        Self::Seed(value)
    }
}

impl From<DiagnosticBundle> for GeneratorError {
    fn from(value: DiagnosticBundle) -> Self {
        Self::Model(value)
    }
}

pub fn build_base_modules(
    repository_root: &Path,
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    limits: &CompileLimits,
) -> Result<Vec<TypedP100Module>, GeneratorError> {
    build_base_modules_from_seed(
        load_p100_seed(repository_root)?,
        accuracy,
        direction,
        limits,
    )
}

pub fn build_base_modules_from_seed(
    seed: LoadedP100Seed,
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    limits: &CompileLimits,
) -> Result<Vec<TypedP100Module>, GeneratorError> {
    build_modules(seed, accuracy, direction, limits, P100Variant::Base)
}

/// 构造冻结的五模块 curved-offset regularity companion workload。
///
/// 该 workload 固定使用 `Fine2Cm` / `Smooth1Deg`，只替换 m00 的一条 reference line；
/// semantic seed 文件和其余有类型字段保持不变。
pub fn build_regularity_probe_modules(
    repository_root: &Path,
    limits: &CompileLimits,
) -> Result<Vec<TypedP100Module>, GeneratorError> {
    build_regularity_probe_modules_from_seed(load_p100_seed(repository_root)?, limits)
}

pub fn build_regularity_probe_modules_from_seed(
    seed: LoadedP100Seed,
    limits: &CompileLimits,
) -> Result<Vec<TypedP100Module>, GeneratorError> {
    build_modules(
        seed,
        GeometryAccuracyProfile::Fine2Cm,
        GeometryDirectionProfile::Smooth1Deg,
        limits,
        P100Variant::RegularityProbe,
    )
}

fn build_modules(
    seed: LoadedP100Seed,
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    limits: &CompileLimits,
    variant: P100Variant,
) -> Result<Vec<TypedP100Module>, GeneratorError> {
    let mut data = seed.data;
    if matches!(variant, P100Variant::RegularityProbe) {
        apply_regularity_probe(&mut data.documents)?;
    }
    let width_profiles = WidthProfiles::new(&data.documents)?;
    let regulated_access_rules = regulated_access_rules(&data.documents);
    let mut modules = Vec::with_capacity(data.documents.len());

    for (module_index, document) in data.documents.iter().enumerate() {
        let index = u8::try_from(module_index)
            .map_err(|_| contract("P100 module index does not fit u8"))?;
        let module_key = data
            .workload
            .generator_contract
            .module_keys
            .get(module_index)
            .ok_or_else(|| contract("P100 module key is missing"))?;
        let source_document_key = data
            .workload
            .generator_contract
            .source_document_keys
            .get(module_index)
            .ok_or_else(|| contract("P100 source document key is missing"))?;
        let imports = if module_index == 0 {
            Vec::new()
        } else {
            vec![data.workload.generator_contract.module_keys[module_index - 1].clone()]
        };
        let provenance = RoadEditingProvenance::generated(
            GENERATOR_BUILD_ID,
            parameters_digest(data.seed_digest, index),
            frontend_options_digest(accuracy, direction),
            Some(u64::from(index)),
            format!("LF-ROAD-EDITING-P100-v1 module {index:02}"),
        )?;
        let header = RoadEditingModuleHeader::try_new(
            module_key.clone(),
            source_document_key.clone(),
            imports,
            provenance,
        )?;
        let module = build_module(
            module_index,
            document,
            header,
            accuracy,
            direction,
            limits,
            &width_profiles,
            &regulated_access_rules,
        )?;
        modules.push(TypedP100Module {
            module_index: index,
            module,
        });
    }
    Ok(modules)
}

fn apply_regularity_probe(documents: &mut [GeometryDocument]) -> Result<(), GeneratorError> {
    let document = documents
        .get_mut(REGULARITY_MODULE_INDEX)
        .ok_or_else(|| contract("regularity probe module p100.m00 is missing"))?;
    let mut matches = document
        .roads
        .iter_mut()
        .filter(|road| road.road_key == REGULARITY_ALIGNMENT_KEY);
    let road = matches
        .next()
        .ok_or_else(|| contract("regularity probe alignment is missing"))?;
    if matches.next().is_some() {
        return Err(contract("regularity probe alignment is not unique"));
    }
    let expected_start = [0.0, 0.0, 0.0];
    let expected_end = [189.5, 0.0, 0.0];
    if road.reference_line.start != expected_start
        || !matches!(
            road.reference_line.segments.as_slice(),
            [CurveSegment::Line { end }] if *end == expected_end
        )
    {
        return Err(contract(
            "regularity probe replacement precondition does not match the frozen base line",
        ));
    }
    road.reference_line = Curve {
        start: expected_start,
        segments: vec![CurveSegment::CubicBezier {
            control1: [20.0, 0.0, 20.0],
            control2: [20.0, 0.0, 0.0],
            end: expected_end,
        }],
    };
    Ok(())
}

pub fn encode_modules(
    modules: Vec<TypedP100Module>,
    limits: &CompileLimits,
) -> Result<Vec<EncodedP100Module>, GeneratorError> {
    let mut encoded = Vec::with_capacity(modules.len());
    for module in modules {
        let module_index = module.module_index;
        let namespace: Box<str> = module.namespace().into();
        let source_document_key: Box<str> = module.source_document_key().into();
        let buffer = RoadEditingSourceWriter::new(limits).write(module.into_module())?;
        encoded.push(EncodedP100Module {
            module_index,
            namespace,
            source_document_key,
            buffer,
        });
    }
    Ok(encoded)
}

/// 让已编码 P100 模块通过 production reader、preflight、lowering、geometry compile 和
/// common admission，并返回完整 Canonical LIR 输出。
///
/// # Errors
///
/// 任一输入身份、模块准入或编译阶段失败时返回对应诊断；不会接受部分编译结果。
pub fn compile_encoded_modules(
    modules: &[EncodedP100Module],
    limits: CompileLimits,
) -> Result<CompilationOutput, GeneratorError> {
    let mut builder = CompilationUnitBuilder::new(limits);
    for module in modules {
        let input =
            RoadEditingModuleInput::try_new(module.source_document_key(), module.as_bytes(), None)
                .map_err(|error| {
                    contract(format!("generated source identity is invalid: {error}"))
                })?;
        builder.add_road_editing_module(input)?;
    }
    let unit = builder.build()?;
    Ok(Compiler::new().compile(unit)?)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct P100CompileStageDurations {
    size_prefix_and_identifier_preflight: Duration,
    flatbuffers_verifier: Duration,
    semantic_preflight_and_typed_ast_lowering: Duration,
    complete_compile: Duration,
}

impl P100CompileStageDurations {
    #[must_use]
    pub const fn size_prefix_and_identifier_preflight(self) -> Duration {
        self.size_prefix_and_identifier_preflight
    }

    #[must_use]
    pub const fn flatbuffers_verifier(self) -> Duration {
        self.flatbuffers_verifier
    }

    #[must_use]
    pub const fn semantic_preflight_and_typed_ast_lowering(self) -> Duration {
        self.semantic_preflight_and_typed_ast_lowering
    }

    #[must_use]
    pub const fn complete_compile(self) -> Duration {
        self.complete_compile
    }
}

/// 在输入身份构造完成后，按冻结边界计时 production admission 与完整编译。
pub fn compile_encoded_modules_with_stage_timing(
    modules: &[EncodedP100Module],
    limits: CompileLimits,
) -> Result<(CompilationOutput, P100CompileStageDurations), GeneratorError> {
    let inputs = modules
        .iter()
        .map(|module| {
            RoadEditingModuleInput::try_new(module.source_document_key(), module.as_bytes(), None)
                .map_err(|error| contract(format!("generated source identity is invalid: {error}")))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut builder = CompilationUnitBuilder::new(limits);
    let mut compiler = Compiler::new();
    let complete_started = Instant::now();
    let mut stage_durations = P100CompileStageDurations::default();
    for input in inputs {
        let durations = builder.add_road_editing_module_with_stage_timing(input)?;
        stage_durations.size_prefix_and_identifier_preflight = stage_durations
            .size_prefix_and_identifier_preflight
            .saturating_add(durations.size_prefix_and_identifier_preflight());
        stage_durations.flatbuffers_verifier = stage_durations
            .flatbuffers_verifier
            .saturating_add(durations.flatbuffers_verifier());
        stage_durations.semantic_preflight_and_typed_ast_lowering = stage_durations
            .semantic_preflight_and_typed_ast_lowering
            .saturating_add(durations.semantic_preflight_and_typed_ast_lowering());
    }
    let unit = builder.build()?;
    let output = compiler.compile(unit)?;
    stage_durations.complete_compile = complete_started.elapsed();
    Ok((output, stage_durations))
}

fn parameters_digest(seed_digest: [u8; 32], module_index: u8) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(PARAMETERS_PREFIX);
    hasher.update(seed_digest);
    hasher.update([module_index]);
    hasher.finalize().into()
}

fn frontend_options_digest(
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(FRONTEND_OPTIONS_PREFIX);
    hasher.update([1, accuracy as u8, direction as u8]);
    hasher.finalize().into()
}

#[allow(clippy::too_many_arguments)]
fn build_module(
    module_index: usize,
    document: &GeometryDocument,
    header: RoadEditingModuleHeader,
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    limits: &CompileLimits,
    width_profiles: &WidthProfiles,
    regulated_access_rules: &BTreeSet<(usize, String)>,
) -> Result<RoadEditingSourceModule, GeneratorError> {
    let indexes = ModuleIndexes::new(document)?;
    let canvases = CanvasOrdinals::new(document, &indexes)?;
    let curve_ordinals = CurveOrdinals::new(document);
    let mut builder = RoadEditingSourceModuleBuilder::new(header, accuracy, direction, limits)?;

    for road in &document.roads {
        let ordinal = curve_ordinals
            .alignments
            .get(&road.road_key)
            .copied()
            .ok_or_else(|| contract("alignment curve ordinal is missing"))?;
        let reference_line = curve_program(module_index, ordinal, &road.reference_line)?;
        let alignment = RoadAlignmentInput::try_new(
            road.road_key.clone(),
            canonical_frame_ref(&road.frame)?,
            reference_line,
        )?
        .with_canvas_selection(alignment_canvas(module_index, ordinal))?;
        builder.add_alignment(alignment)?;
    }

    add_road_declarations(
        module_index,
        document,
        &indexes,
        &canvases,
        &curve_ordinals,
        width_profiles,
        &mut builder,
    )?;
    add_junction_declarations(
        module_index,
        document,
        &indexes,
        &canvases,
        &curve_ordinals,
        &mut builder,
    )?;
    add_overlay_declarations(
        module_index,
        document,
        &indexes,
        &canvases,
        regulated_access_rules,
        &mut builder,
    )?;
    for frame in &document.frames {
        add_decl(
            &mut builder,
            RoadEditingDeclaration::CanonicalFrame(
                CanonicalFrameInput::try_new(frame.frame_key.clone())?.with_canvas_selection(
                    canvases.stable(module_index, 22, &[], &frame.frame_key)?,
                )?,
            ),
        )?;
    }
    Ok(builder.finish()?)
}

#[allow(clippy::too_many_arguments)]
fn add_road_declarations(
    module_index: usize,
    document: &GeometryDocument,
    indexes: &ModuleIndexes,
    canvases: &CanvasOrdinals,
    _curve_ordinals: &CurveOrdinals,
    width_profiles: &WidthProfiles,
    builder: &mut RoadEditingSourceModuleBuilder<'_>,
) -> Result<(), GeneratorError> {
    for road in &document.roads {
        let span = only_span(road)?;
        let corridor_key = span.corridor_key.as_str();
        let elements = span
            .elements
            .iter()
            .map(|element| match element {
                CorridorElement::RoadSection { section_key } => {
                    Ok(RoadEditingCorridorElement::RoadSection(road_section_ref(
                        corridor_key,
                        section_key,
                    )?))
                }
                CorridorElement::FacilityBand { facility_band_key } => {
                    Ok(RoadEditingCorridorElement::FacilityBand(facility_band_ref(
                        corridor_key,
                        facility_band_key,
                    )?))
                }
            })
            .collect::<Result<Vec<_>, GeneratorError>>()?;
        let end_station = match &span.end_station_meters {
            EndStation::Finite(value) => RoadEditingStationEnd::Finite(*value),
            EndStation::AlignmentEnd(value) if value == "end" => {
                RoadEditingStationEnd::AlignmentEnd
            }
            EndStation::AlignmentEnd(_) => {
                return Err(contract("corridor string end must be exactly 'end'"));
            }
        };
        let corridor = RoadCorridorInput::try_new(
            corridor_key.to_owned(),
            RoadAlignmentReference::try_new(road.road_key.clone())?,
            span.start_station_meters,
            end_station,
            road_section_ref(corridor_key, &span.reference_section_key)?,
            authoring_lane_ref(
                corridor_key,
                &span.reference_section_key,
                &span.reference_lane_key,
            )?,
            elements,
        )?
        .with_canvas_selection(canvases.stable(module_index, 1, &[], corridor_key)?)?;
        add_decl(builder, RoadEditingDeclaration::RoadCorridor(corridor))?;

        for section in &span.road_sections {
            let section_owners = [corridor_key, section.section_key.as_str()];
            let lane_refs = section
                .lanes
                .iter()
                .map(|lane| authoring_lane_ref(corridor_key, &section.section_key, &lane.lane_key))
                .collect::<Result<Vec<_>, GeneratorError>>()?;
            let section_input = RoadSectionInput::try_new(
                section.section_key.clone(),
                section.kind_id.clone(),
                lane_refs,
                road_corridor_ref(corridor_key)?,
            )?
            .with_canvas_selection(canvases.stable(
                module_index,
                2,
                &[corridor_key],
                &section.section_key,
            )?)?;
            add_decl(builder, RoadEditingDeclaration::RoadSection(section_input))?;

            for lane in &section.lanes {
                let profile = width_profiles.get(
                    module_index,
                    MemberKind::AuthoringLane,
                    corridor_key,
                    &lane.lane_key,
                )?;
                let lane_group = lane
                    .lane_group_key
                    .as_deref()
                    .map(|key| lane_group_ref(corridor_key, &section.section_key, key))
                    .transpose()?;
                let lane_input = AuthoringLaneInput::try_new(
                    lane.lane_key.clone(),
                    lane_edge_ref(&lane.lane_edge_key)?,
                    match lane.direction {
                        LaneDirection::Forward => RoadEditingLaneDirection::Forward,
                        LaneDirection::Backward => RoadEditingLaneDirection::Backward,
                    },
                    profile,
                    lane_group,
                    road_section_ref(corridor_key, &section.section_key)?,
                )?
                .with_canvas_selection(canvases.stable(
                    module_index,
                    3,
                    &section_owners,
                    &lane.lane_key,
                )?)?;
                add_decl(builder, RoadEditingDeclaration::AuthoringLane(lane_input))?;

                let successors = lane
                    .successors
                    .iter()
                    .map(|key| lane_edge_ref(key))
                    .collect::<Result<Vec<_>, GeneratorError>>()?;
                let edge = LaneEdgeInput::try_new(
                    lane.lane_edge_key.clone(),
                    lane.speed_limit_meters_per_second,
                    successors,
                    None,
                )?
                .with_canvas_selection(canvases.stable(
                    module_index,
                    4,
                    &[],
                    &lane.lane_edge_key,
                )?)?;
                add_decl(builder, RoadEditingDeclaration::LaneEdge(edge))?;
            }
            for group in &section.lane_groups {
                let input = LaneGroupInput::try_new(
                    group.lane_group_key.clone(),
                    road_section_ref(corridor_key, &section.section_key)?,
                )?
                .with_canvas_selection(canvases.stable(
                    module_index,
                    16,
                    &section_owners,
                    &group.lane_group_key,
                )?)?;
                add_decl(builder, RoadEditingDeclaration::LaneGroup(input))?;
            }
        }
        for band in &span.facility_bands {
            let input = FacilityBandInput::try_new(
                band.facility_band_key.clone(),
                band.kind_id.clone(),
                width_profiles.get(
                    module_index,
                    MemberKind::FacilityBand,
                    corridor_key,
                    &band.facility_band_key,
                )?,
                road_corridor_ref(corridor_key)?,
            )?
            .with_canvas_selection(canvases.stable(
                module_index,
                17,
                &[corridor_key],
                &band.facility_band_key,
            )?)?;
            add_decl(builder, RoadEditingDeclaration::FacilityBand(input))?;
        }
    }
    let _ = indexes;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn add_junction_declarations(
    module_index: usize,
    document: &GeometryDocument,
    indexes: &ModuleIndexes,
    canvases: &CanvasOrdinals,
    curve_ordinals: &CurveOrdinals,
    builder: &mut RoadEditingSourceModuleBuilder<'_>,
) -> Result<(), GeneratorError> {
    for junction in &document.junctions {
        let approach_edges = junction
            .approach_edges
            .iter()
            .map(|key| lane_edge_ref(key))
            .collect::<Result<Vec<_>, GeneratorError>>()?;
        let internal_edges = junction
            .internal_edges
            .iter()
            .map(|edge| lane_edge_ref(&edge.lane_edge_key))
            .collect::<Result<Vec<_>, GeneratorError>>()?;
        let input = JunctionInput::try_new(
            junction.junction_key.clone(),
            approach_edges,
            internal_edges,
        )?
        .with_canvas_selection(canvases.stable(
            module_index,
            5,
            &[],
            &junction.junction_key,
        )?)?;
        add_decl(builder, RoadEditingDeclaration::Junction(input))?;

        for edge in &junction.internal_edges {
            let ordinal = curve_ordinals
                .internal_edges
                .get(&edge.lane_edge_key)
                .copied()
                .ok_or_else(|| contract("internal edge curve ordinal is missing"))?;
            let geometry = curve_program(module_index, ordinal, &edge.geometry)?;
            let input = LaneEdgeInput::try_new(
                edge.lane_edge_key.clone(),
                edge.speed_limit_meters_per_second,
                Vec::new(),
                Some(geometry),
            )?
            .with_canvas_selection(canvases.stable(
                module_index,
                4,
                &[],
                &edge.lane_edge_key,
            )?)?;
            add_decl(builder, RoadEditingDeclaration::LaneEdge(input))?;
        }

        for connection in &junction.connections {
            let movement = MovementInput::try_new(
                connection.movement_key.clone(),
                junction_ref(&junction.junction_key)?,
                connection.directed_entry_approach_key.clone(),
                connection.directed_exit_approach_key.clone(),
            )?
            .with_canvas_selection(canvases.stable(
                module_index,
                6,
                &[&junction.junction_key],
                &connection.movement_key,
            )?)?;
            add_decl(builder, RoadEditingDeclaration::Movement(movement))?;

            let internal = connection
                .internal_edge_sequence
                .iter()
                .map(|key| lane_edge_ref(key))
                .collect::<Result<Vec<_>, GeneratorError>>()?;
            let path = ManeuverPathInput::try_new(
                connection.maneuver_path_key.clone(),
                movement_ref(&junction.junction_key, &connection.movement_key)?,
                lane_edge_ref(&connection.entry_edge)?,
                internal,
                lane_edge_ref(&connection.exit_edge)?,
            )?
            .with_canvas_selection(canvases.stable(
                module_index,
                7,
                &[&junction.junction_key, &connection.movement_key],
                &connection.maneuver_path_key,
            )?)?;
            add_decl(builder, RoadEditingDeclaration::ManeuverPath(path))?;
        }
    }
    let _ = indexes;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn add_overlay_declarations(
    module_index: usize,
    document: &GeometryDocument,
    indexes: &ModuleIndexes,
    canvases: &CanvasOrdinals,
    regulated_access_rules: &BTreeSet<(usize, String)>,
    builder: &mut RoadEditingSourceModuleBuilder<'_>,
) -> Result<(), GeneratorError> {
    let overlays = &document.overlays;
    for group in &overlays.signal_groups {
        let value =
            SignalGroupInput::try_new(group.signal_group_key.clone())?.with_canvas_selection(
                canvases.stable(module_index, 11, &[], &group.signal_group_key)?,
            )?;
        add_decl(builder, RoadEditingDeclaration::SignalGroup(value))?;
    }
    for controller in &overlays.signal_controllers {
        let groups = controller
            .signal_groups
            .iter()
            .map(|key| signal_group_ref(key))
            .collect::<Result<Vec<_>, GeneratorError>>()?;
        let phases = controller
            .phases
            .iter()
            .map(|phase| {
                signal_phase_ref(&controller.signal_controller_key, &phase.signal_phase_key)
            })
            .collect::<Result<Vec<_>, GeneratorError>>()?;
        let value = SignalControllerInput::try_new(
            controller.signal_controller_key.clone(),
            milliseconds(controller.offset_seconds, "signalController.offsetSeconds")?,
            groups,
            phases,
        )?
        .with_canvas_selection(canvases.stable(
            module_index,
            12,
            &[],
            &controller.signal_controller_key,
        )?)?;
        add_decl(builder, RoadEditingDeclaration::SignalController(value))?;

        for phase in &controller.phases {
            let states = phase
                .states
                .iter()
                .map(|state| {
                    RoadEditingSignalPhaseState::try_new(
                        signal_group_ref(&state.signal_group)?,
                        match state.aspect {
                            super::SignalAspect::Red => SignalAspect::Red,
                            super::SignalAspect::Yellow => SignalAspect::Yellow,
                            super::SignalAspect::Green => SignalAspect::Green,
                        },
                    )
                    .map_err(GeneratorError::from)
                })
                .collect::<Result<Vec<_>, GeneratorError>>()?;
            let value = SignalPhaseInput::try_new(
                phase.signal_phase_key.clone(),
                milliseconds(phase.duration_seconds, "signalPhase.durationSeconds")?,
                states,
                signal_controller_ref(&controller.signal_controller_key)?,
            )?
            .with_canvas_selection(canvases.stable(
                module_index,
                13,
                &[&controller.signal_controller_key],
                &phase.signal_phase_key,
            )?)?;
            add_decl(builder, RoadEditingDeclaration::SignalPhase(value))?;
        }
    }
    for area in &overlays.parking_areas {
        let value =
            ParkingAreaInput::try_new(area.parking_area_key.clone())?.with_canvas_selection(
                canvases.stable(module_index, 14, &[], &area.parking_area_key)?,
            )?;
        add_decl(builder, RoadEditingDeclaration::ParkingArea(value))?;
    }
    for space in &overlays.parking_spaces {
        let entry = ParkingLaneAnchor::try_new(
            lane_edge_ref(&space.entry.lane_edge)?,
            space.entry.progress_meters,
        )?;
        let exit = ParkingLaneAnchor::try_new(
            lane_edge_ref(&space.exit.lane_edge)?,
            space.exit.progress_meters,
        )?;
        let geometry = ParkingSpaceGeometry::try_new(
            space.geometry.lateral_offset_meters,
            space.geometry.heading_offset_radians,
            space.geometry.length_meters,
            space.geometry.width_meters,
        )?;
        let mut value =
            ParkingSpaceInput::try_new(space.parking_space_key.clone(), entry, exit, geometry)?;
        if let Some(area) = &space.parking_area {
            value = value.with_parking_area(parking_area_ref(area)?);
        }
        value = value.with_canvas_selection(canvases.stable(
            module_index,
            15,
            &[],
            &space.parking_space_key,
        )?)?;
        add_decl(builder, RoadEditingDeclaration::ParkingSpace(value))?;
    }
    for class in &overlays.participant_classes {
        let mut value = ParticipantClassInput::try_new(class.participant_class_key.clone())?;
        if let Some(parent) = &class.extends {
            value = value.with_extends(participant_class_ref(parent)?);
        }
        value = value.with_canvas_selection(canvases.stable(
            module_index,
            18,
            &[],
            &class.participant_class_key,
        )?)?;
        add_decl(builder, RoadEditingDeclaration::ParticipantClass(value))?;
    }
    for rule in &overlays.access_rules {
        let target = access_target(indexes, &rule.target)?;
        let participants = rule
            .participant_classes
            .iter()
            .map(|key| participant_class_ref(key))
            .collect::<Result<Vec<_>, GeneratorError>>()?;
        let mut value = AccessRuleInput::try_new(
            rule.access_rule_key.clone(),
            target,
            match rule.effect {
                super::AccessEffect::Allow => AccessEffect::Allow,
                super::AccessEffect::Deny => AccessEffect::Deny,
            },
            participants,
            rule.priority,
        )?;
        if regulated_access_rules.contains(&(module_index, rule.access_rule_key.clone())) {
            value = value.with_regulation(
                AccessRegulationInput::try_new("CN", "p100-v1")?
                    .with_source("LF-ROAD-EDITING-P100-v1")?,
            );
        }
        value = value.with_canvas_selection(canvases.stable(
            module_index,
            19,
            &[],
            &rule.access_rule_key,
        )?)?;
        add_decl(builder, RoadEditingDeclaration::AccessRule(value))?;
    }
    for profile in &overlays.vehicle_profiles {
        let iidm = IidmVehicleProfileInput::try_new(
            profile.iidm.length_meters,
            profile.iidm.desired_speed_meters_per_second,
            profile.iidm.min_gap_meters,
            profile.iidm.time_headway_seconds,
            profile.iidm.max_acceleration_meters_per_second_squared,
            profile
                .iidm
                .comfortable_deceleration_meters_per_second_squared,
            profile
                .iidm
                .emergency_deceleration_meters_per_second_squared,
        )?;
        let value = VehicleProfileInput::try_new(
            profile.vehicle_profile_key.clone(),
            participant_class_ref(&profile.participant_class)?,
            iidm,
        )?
        .with_canvas_selection(canvases.stable(
            module_index,
            20,
            &[],
            &profile.vehicle_profile_key,
        )?)?;
        add_decl(builder, RoadEditingDeclaration::VehicleProfile(value))?;
    }
    for route in &overlays.static_routes {
        let edges = route
            .edge_sequence
            .iter()
            .map(|key| lane_edge_ref(key))
            .collect::<Result<Vec<_>, GeneratorError>>()?;
        let value = StaticRouteInput::try_new(route.static_route_key.clone(), edges)?
            .with_canvas_selection(canvases.stable(
                module_index,
                21,
                &[],
                &route.static_route_key,
            )?)?;
        add_decl(builder, RoadEditingDeclaration::StaticRoute(value))?;
    }
    for stop_line in &overlays.stop_lines {
        let value = StopLineInput::try_new(
            stop_line.stop_line_key.clone(),
            lane_edge_ref(&stop_line.lane_edge)?,
        )?
        .with_canvas_selection(canvases.stable(
            module_index,
            10,
            &[],
            &stop_line.stop_line_key,
        )?)?;
        add_decl(builder, RoadEditingDeclaration::StopLine(value))?;
    }
    for gate in &overlays.maneuver_gates {
        let owners = indexes
            .path_owners
            .get(&gate.maneuver_path)
            .ok_or_else(|| {
                contract(format!(
                    "maneuver gate path owner missing: {}",
                    gate.maneuver_path
                ))
            })?;
        let path = maneuver_path_ref(&owners.0, &owners.1, &gate.maneuver_path)?;
        let signal_control = match &gate.signal_control {
            Some(group) => RoadEditingSignalControl::SignalGroup(signal_group_ref(group)?),
            None => RoadEditingSignalControl::None,
        };
        let value = ManeuverGateInput::try_new(
            gate.maneuver_gate_key.clone(),
            path,
            gate.transition_index,
            stop_line_ref(&gate.stop_line)?,
            signal_control,
        )?
        .with_canvas_selection(canvases.stable(
            module_index,
            8,
            &[&owners.0, &owners.1, &gate.maneuver_path],
            &gate.maneuver_gate_key,
        )?)?;
        add_decl(builder, RoadEditingDeclaration::ManeuverGate(value))?;
    }
    for zone in &overlays.waiting_zones {
        let owners = indexes
            .path_owners
            .get(&zone.maneuver_path)
            .ok_or_else(|| {
                contract(format!(
                    "waiting-zone path owner missing: {}",
                    zone.maneuver_path
                ))
            })?;
        let owner_keys = vec![
            owners.0.clone(),
            owners.1.clone(),
            zone.maneuver_path.clone(),
        ];
        let value = WaitingZoneInput::try_new(
            zone.waiting_zone_key.clone(),
            maneuver_path_ref(&owners.0, &owners.1, &zone.maneuver_path)?,
            ManeuverGateReference::owner_scoped(owner_keys.clone(), zone.entry_gate.clone())?,
            ManeuverGateReference::owner_scoped(owner_keys, zone.release_gate.clone())?,
            zone.max_occupancy,
        )?
        .with_canvas_selection(canvases.stable(
            module_index,
            9,
            &[&owners.0, &owners.1, &zone.maneuver_path],
            &zone.waiting_zone_key,
        )?)?;
        add_decl(builder, RoadEditingDeclaration::WaitingZone(value))?;
    }
    Ok(())
}

fn add_decl(
    builder: &mut RoadEditingSourceModuleBuilder<'_>,
    declaration: RoadEditingDeclaration,
) -> Result<(), GeneratorError> {
    builder.add_declaration(declaration)?;
    Ok(())
}

fn curve_program(
    module_index: usize,
    curve_ordinal: u32,
    curve: &Curve,
) -> Result<RoadEditingCurveProgram, GeneratorError> {
    let start = point(curve.start)?;
    let mut segments = Vec::with_capacity(curve.segments.len());
    for (segment_index, segment) in curve.segments.iter().enumerate() {
        let segment = match segment {
            CurveSegment::Line { end } => RoadEditingCurveSegment::line(point(*end)?),
            CurveSegment::CubicBezier {
                control1,
                control2,
                end,
            } => RoadEditingCurveSegment::cubic_bezier(
                point(*control1)?,
                point(*control2)?,
                point(*end)?,
            ),
        }
        .with_canvas_selection(curve_canvas(
            module_index,
            curve_ordinal,
            u32::try_from(segment_index)
                .map_err(|_| contract("curve segment ordinal does not fit u32"))?,
        ))?;
        segments.push(segment);
    }
    Ok(RoadEditingCurveProgram::try_new(start, segments)?)
}

fn point(value: [f64; 3]) -> Result<RoadEditingPoint3, GeneratorError> {
    Ok(RoadEditingPoint3::try_new(value[0], value[1], value[2])?)
}

fn milliseconds(value: f64, field: &str) -> Result<u64, GeneratorError> {
    let scaled = value * 1_000.0;
    if !scaled.is_finite()
        || scaled < 0.0
        || scaled.fract() != 0.0
        || scaled >= 18_446_744_073_709_551_616.0
    {
        return Err(contract(format!(
            "{field} does not checked-convert to integral milliseconds"
        )));
    }
    Ok(scaled as u64)
}

fn access_target(
    indexes: &ModuleIndexes,
    target: &AccessTarget,
) -> Result<RoadEditingAccessTarget, GeneratorError> {
    match target {
        AccessTarget::LaneEdge { lane_edge } => {
            Ok(RoadEditingAccessTarget::LaneEdge(lane_edge_ref(lane_edge)?))
        }
        AccessTarget::LaneGroup { lane_group } => {
            let (corridor, section) = indexes
                .lane_group_owners
                .get(lane_group)
                .ok_or_else(|| contract(format!("lane group owner missing: {lane_group}")))?;
            Ok(RoadEditingAccessTarget::LaneGroup(lane_group_ref(
                corridor, section, lane_group,
            )?))
        }
        AccessTarget::RoadSection { road_section } => {
            let corridor = indexes
                .section_owners
                .get(road_section)
                .ok_or_else(|| contract(format!("road section owner missing: {road_section}")))?;
            Ok(RoadEditingAccessTarget::RoadSection(road_section_ref(
                corridor,
                road_section,
            )?))
        }
        AccessTarget::ManeuverPath { maneuver_path } => {
            let (junction, movement) = indexes
                .path_owners
                .get(maneuver_path)
                .ok_or_else(|| contract(format!("maneuver path owner missing: {maneuver_path}")))?;
            Ok(RoadEditingAccessTarget::ManeuverPath(maneuver_path_ref(
                junction,
                movement,
                maneuver_path,
            )?))
        }
        AccessTarget::FacilityBand { facility_band } => Err(contract(format!(
            "seed unexpectedly targets unsupported FacilityBand {facility_band}"
        ))),
    }
}

fn only_span(road: &Road) -> Result<&CrossSectionSpan, GeneratorError> {
    match road.cross_section_spans.as_slice() {
        [span] => Ok(span),
        _ => Err(contract(format!(
            "road {} must contain exactly one cross-section span",
            road.road_key
        ))),
    }
}

fn alignment_canvas(module: usize, ordinal: u32) -> String {
    format!("cv/m{module:02}/a/{ordinal:03}")
}

fn curve_canvas(module: usize, program: u32, segment: u32) -> String {
    format!("cv/m{module:02}/c/{program:03}/{segment:03}")
}

fn stable_canvas(module: usize, kind: u8, ordinal: u32) -> String {
    format!("cv/m{module:02}/d{kind:02}/{ordinal:03}")
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct AddressKey {
    kind: u8,
    owners: Vec<String>,
    local_key: String,
}

struct CanvasOrdinals {
    ordinals: BTreeMap<AddressKey, u32>,
}

impl CanvasOrdinals {
    fn new(document: &GeometryDocument, indexes: &ModuleIndexes) -> Result<Self, GeneratorError> {
        let mut by_kind = BTreeMap::<u8, Vec<AddressKey>>::new();
        let mut push = |kind: u8, owners: Vec<String>, local_key: String| {
            by_kind.entry(kind).or_default().push(AddressKey {
                kind,
                owners,
                local_key,
            });
        };
        for road in &document.roads {
            let span = only_span(road)?;
            push(1, Vec::new(), span.corridor_key.clone());
            for section in &span.road_sections {
                push(
                    2,
                    vec![span.corridor_key.clone()],
                    section.section_key.clone(),
                );
                for lane in &section.lanes {
                    push(
                        3,
                        vec![span.corridor_key.clone(), section.section_key.clone()],
                        lane.lane_key.clone(),
                    );
                    push(4, Vec::new(), lane.lane_edge_key.clone());
                }
                for group in &section.lane_groups {
                    push(
                        16,
                        vec![span.corridor_key.clone(), section.section_key.clone()],
                        group.lane_group_key.clone(),
                    );
                }
            }
            for band in &span.facility_bands {
                push(
                    17,
                    vec![span.corridor_key.clone()],
                    band.facility_band_key.clone(),
                );
            }
        }
        for junction in &document.junctions {
            push(5, Vec::new(), junction.junction_key.clone());
            for edge in &junction.internal_edges {
                push(4, Vec::new(), edge.lane_edge_key.clone());
            }
            for connection in &junction.connections {
                push(
                    6,
                    vec![junction.junction_key.clone()],
                    connection.movement_key.clone(),
                );
                push(
                    7,
                    vec![
                        junction.junction_key.clone(),
                        connection.movement_key.clone(),
                    ],
                    connection.maneuver_path_key.clone(),
                );
            }
        }
        let overlays = &document.overlays;
        for value in &overlays.maneuver_gates {
            let (junction, movement) = indexes
                .path_owners
                .get(&value.maneuver_path)
                .ok_or_else(|| contract("maneuver gate owner path is missing"))?;
            push(
                8,
                vec![
                    junction.clone(),
                    movement.clone(),
                    value.maneuver_path.clone(),
                ],
                value.maneuver_gate_key.clone(),
            );
        }
        for value in &overlays.waiting_zones {
            let (junction, movement) = indexes
                .path_owners
                .get(&value.maneuver_path)
                .ok_or_else(|| contract("waiting zone owner path is missing"))?;
            push(
                9,
                vec![
                    junction.clone(),
                    movement.clone(),
                    value.maneuver_path.clone(),
                ],
                value.waiting_zone_key.clone(),
            );
        }
        push_module_scoped(
            &mut push,
            10,
            overlays.stop_lines.iter().map(|v| v.stop_line_key.clone()),
        );
        push_module_scoped(
            &mut push,
            11,
            overlays
                .signal_groups
                .iter()
                .map(|v| v.signal_group_key.clone()),
        );
        for controller in &overlays.signal_controllers {
            push(12, Vec::new(), controller.signal_controller_key.clone());
            for phase in &controller.phases {
                push(
                    13,
                    vec![controller.signal_controller_key.clone()],
                    phase.signal_phase_key.clone(),
                );
            }
        }
        push_module_scoped(
            &mut push,
            14,
            overlays
                .parking_areas
                .iter()
                .map(|v| v.parking_area_key.clone()),
        );
        push_module_scoped(
            &mut push,
            15,
            overlays
                .parking_spaces
                .iter()
                .map(|v| v.parking_space_key.clone()),
        );
        push_module_scoped(
            &mut push,
            18,
            overlays
                .participant_classes
                .iter()
                .map(|v| v.participant_class_key.clone()),
        );
        push_module_scoped(
            &mut push,
            19,
            overlays
                .access_rules
                .iter()
                .map(|v| v.access_rule_key.clone()),
        );
        push_module_scoped(
            &mut push,
            20,
            overlays
                .vehicle_profiles
                .iter()
                .map(|v| v.vehicle_profile_key.clone()),
        );
        push_module_scoped(
            &mut push,
            21,
            overlays
                .static_routes
                .iter()
                .map(|v| v.static_route_key.clone()),
        );
        push_module_scoped(
            &mut push,
            22,
            document.frames.iter().map(|v| v.frame_key.clone()),
        );

        let mut ordinals = BTreeMap::new();
        for values in by_kind.values_mut() {
            values.sort_unstable_by(|left, right| {
                left.owners
                    .cmp(&right.owners)
                    .then_with(|| left.local_key.cmp(&right.local_key))
            });
            for (ordinal, value) in values.iter().enumerate() {
                let ordinal = u32::try_from(ordinal)
                    .map_err(|_| contract("stable canvas ordinal does not fit u32"))?;
                if ordinals.insert(value.clone(), ordinal).is_some() {
                    return Err(contract("duplicate stable source address in seed"));
                }
            }
        }
        Ok(Self { ordinals })
    }

    fn stable(
        &self,
        module: usize,
        kind: u8,
        owners: &[&str],
        local_key: &str,
    ) -> Result<String, GeneratorError> {
        let key = AddressKey {
            kind,
            owners: owners.iter().map(|value| (*value).to_owned()).collect(),
            local_key: local_key.to_owned(),
        };
        let ordinal = self.ordinals.get(&key).copied().ok_or_else(|| {
            contract(format!(
                "stable canvas ordinal missing for kind {kind} key {local_key}"
            ))
        })?;
        Ok(stable_canvas(module, kind, ordinal))
    }
}

fn push_module_scoped<I>(push: &mut impl FnMut(u8, Vec<String>, String), kind: u8, values: I)
where
    I: IntoIterator<Item = String>,
{
    for value in values {
        push(kind, Vec::new(), value);
    }
}

struct CurveOrdinals {
    alignments: BTreeMap<String, u32>,
    internal_edges: BTreeMap<String, u32>,
}

impl CurveOrdinals {
    fn new(document: &GeometryDocument) -> Self {
        let mut alignment_keys = document
            .roads
            .iter()
            .map(|road| road.road_key.clone())
            .collect::<Vec<_>>();
        alignment_keys.sort_unstable();
        let alignments = alignment_keys
            .into_iter()
            .enumerate()
            .map(|(ordinal, key)| (key, ordinal as u32))
            .collect::<BTreeMap<_, _>>();
        let mut internal_keys = document
            .junctions
            .iter()
            .flat_map(|junction| junction.internal_edges.iter())
            .map(|edge| edge.lane_edge_key.clone())
            .collect::<Vec<_>>();
        internal_keys.sort_unstable();
        let first = u32::try_from(alignments.len()).expect("P100 alignment count fits u32");
        let internal_edges = internal_keys
            .into_iter()
            .enumerate()
            .map(|(ordinal, key)| (key, first + ordinal as u32))
            .collect();
        Self {
            alignments,
            internal_edges,
        }
    }
}

struct ModuleIndexes {
    section_owners: BTreeMap<String, String>,
    lane_group_owners: BTreeMap<String, (String, String)>,
    path_owners: BTreeMap<String, (String, String)>,
}

impl ModuleIndexes {
    fn new(document: &GeometryDocument) -> Result<Self, GeneratorError> {
        let mut section_owners = BTreeMap::new();
        let mut lane_group_owners = BTreeMap::new();
        let mut path_owners = BTreeMap::new();
        for road in &document.roads {
            let span = only_span(road)?;
            for section in &span.road_sections {
                insert_unique(
                    &mut section_owners,
                    section.section_key.clone(),
                    span.corridor_key.clone(),
                    "road section",
                )?;
                for group in &section.lane_groups {
                    insert_unique(
                        &mut lane_group_owners,
                        group.lane_group_key.clone(),
                        (span.corridor_key.clone(), section.section_key.clone()),
                        "lane group",
                    )?;
                }
            }
        }
        for junction in &document.junctions {
            for connection in &junction.connections {
                insert_unique(
                    &mut path_owners,
                    connection.maneuver_path_key.clone(),
                    (
                        junction.junction_key.clone(),
                        connection.movement_key.clone(),
                    ),
                    "maneuver path",
                )?;
            }
        }
        Ok(Self {
            section_owners,
            lane_group_owners,
            path_owners,
        })
    }
}

fn insert_unique<K: Ord, V>(
    map: &mut BTreeMap<K, V>,
    key: K,
    value: V,
    kind: &str,
) -> Result<(), GeneratorError> {
    if map.insert(key, value).is_some() {
        return Err(contract(format!("duplicate {kind} key in seed")));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum MemberKind {
    AuthoringLane,
    FacilityBand,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct MemberKey {
    module_index: usize,
    kind: MemberKind,
    corridor_key: String,
    local_key: String,
}

#[derive(Clone, Copy)]
enum ConnectedEndpoint {
    Start,
    End,
}

struct WidthCandidate {
    key: MemberKey,
    base_width: f64,
    connected: ConnectedEndpoint,
}

struct WidthProfiles {
    profiles: BTreeMap<MemberKey, (f64, f64)>,
}

impl WidthProfiles {
    fn new(documents: &[GeometryDocument]) -> Result<Self, GeneratorError> {
        const CONNECTED_END: [&str; 3] = [
            "corridor-main-road-0",
            "corridor-side-1-road-0",
            "corridor-side-2-road-0",
        ];
        const CONNECTED_START: [&str; 3] = [
            "corridor-main-road-4",
            "corridor-side-1-road-2",
            "corridor-side-2-road-2",
        ];
        let mut profiles = BTreeMap::new();
        let mut eligible = Vec::new();
        for (module_index, document) in documents.iter().enumerate() {
            for road in &document.roads {
                let span = only_span(road)?;
                let connected = if CONNECTED_END.contains(&span.corridor_key.as_str()) {
                    Some(ConnectedEndpoint::End)
                } else if CONNECTED_START.contains(&span.corridor_key.as_str()) {
                    Some(ConnectedEndpoint::Start)
                } else {
                    None
                };
                for section in &span.road_sections {
                    for lane in &section.lanes {
                        let key = MemberKey {
                            module_index,
                            kind: MemberKind::AuthoringLane,
                            corridor_key: span.corridor_key.clone(),
                            local_key: lane.lane_key.clone(),
                        };
                        if profiles
                            .insert(key.clone(), (lane.width_meters, lane.width_meters))
                            .is_some()
                        {
                            return Err(contract("duplicate lane width member"));
                        }
                        let is_rewrite_target =
                            module_index == 2 && lane.lane_key == "section-main-w2e-road-0/lane/2";
                        if let Some(connected) = connected.filter(|_| !is_rewrite_target) {
                            eligible.push(WidthCandidate {
                                key,
                                base_width: lane.width_meters,
                                connected,
                            });
                        }
                    }
                }
                for band in &span.facility_bands {
                    let key = MemberKey {
                        module_index,
                        kind: MemberKind::FacilityBand,
                        corridor_key: span.corridor_key.clone(),
                        local_key: band.facility_band_key.clone(),
                    };
                    if profiles
                        .insert(key.clone(), (band.width_meters, band.width_meters))
                        .is_some()
                    {
                        return Err(contract("duplicate facility width member"));
                    }
                    if let Some(connected) = connected {
                        eligible.push(WidthCandidate {
                            key,
                            base_width: band.width_meters,
                            connected,
                        });
                    }
                }
            }
        }
        eligible.sort_unstable_by(|left, right| {
            left.key
                .module_index
                .cmp(&right.key.module_index)
                .then_with(|| left.key.kind.cmp(&right.key.kind))
                .then_with(|| left.key.local_key.cmp(&right.key.local_key))
        });
        if eligible.len() != 169 {
            return Err(contract(format!(
                "width eligible member count should be 169, got {}",
                eligible.len()
            )));
        }
        let moderate_candidates = eligible
            .iter()
            .filter(|candidate| candidate.key.kind == MemberKind::AuthoringLane)
            .filter(|candidate| {
                matches!(
                    candidate.key.corridor_key.as_str(),
                    "corridor-main-road-0" | "corridor-main-road-4"
                )
            })
            .collect::<Vec<_>>();
        if moderate_candidates.len() != 59 {
            return Err(contract(format!(
                "width moderate candidate count should be 59, got {}",
                moderate_candidates.len()
            )));
        }
        let mut selected = BTreeSet::new();
        for (ordinal, candidate) in moderate_candidates.into_iter().take(20).enumerate() {
            let widening = ordinal % 2 == 0;
            let base = candidate.base_width;
            let endpoints = match (widening, candidate.connected) {
                (true, ConnectedEndpoint::End) => (0.75 * base, base),
                (true, ConnectedEndpoint::Start) => (base, 1.25 * base),
                (false, ConnectedEndpoint::End) => (1.25 * base, base),
                (false, ConnectedEndpoint::Start) => (base, 0.75 * base),
            };
            profiles.insert(candidate.key.clone(), endpoints);
            selected.insert(candidate.key.clone());
        }
        let zero_to_positive = eligible
            .iter()
            .filter(|candidate| !selected.contains(&candidate.key))
            .filter(|candidate| candidate.key.kind == MemberKind::FacilityBand)
            .filter(|candidate| matches!(candidate.connected, ConnectedEndpoint::End))
            .take(5)
            .collect::<Vec<_>>();
        let positive_to_zero = eligible
            .iter()
            .filter(|candidate| !selected.contains(&candidate.key))
            .filter(|candidate| candidate.key.kind == MemberKind::FacilityBand)
            .filter(|candidate| matches!(candidate.connected, ConnectedEndpoint::Start))
            .take(5)
            .collect::<Vec<_>>();
        if zero_to_positive.len() != 5 || positive_to_zero.len() != 5 {
            return Err(contract("width zero-profile candidate count is incomplete"));
        }
        for candidate in zero_to_positive {
            profiles.insert(candidate.key.clone(), (0.0, candidate.base_width));
            selected.insert(candidate.key.clone());
        }
        for candidate in positive_to_zero {
            profiles.insert(candidate.key.clone(), (candidate.base_width, 0.0));
            selected.insert(candidate.key.clone());
        }
        if selected.len() != 30 || profiles.len() != 205 {
            return Err(contract("width profile assignment counts are not 30/205"));
        }
        Ok(Self { profiles })
    }

    fn get(
        &self,
        module_index: usize,
        kind: MemberKind,
        corridor_key: &str,
        local_key: &str,
    ) -> Result<LinearWidthProfile, GeneratorError> {
        let key = MemberKey {
            module_index,
            kind,
            corridor_key: corridor_key.to_owned(),
            local_key: local_key.to_owned(),
        };
        let (start, end) = self
            .profiles
            .get(&key)
            .copied()
            .ok_or_else(|| contract(format!("width profile missing for member {local_key}")))?;
        Ok(LinearWidthProfile::try_new(start, end)?)
    }
}

fn regulated_access_rules(documents: &[GeometryDocument]) -> BTreeSet<(usize, String)> {
    let mut values = documents
        .iter()
        .enumerate()
        .flat_map(|(module, document)| {
            document
                .overlays
                .access_rules
                .iter()
                .map(move |rule| (module, rule.access_rule_key.clone()))
        })
        .collect::<Vec<_>>();
    values.sort_unstable();
    values
        .into_iter()
        .enumerate()
        .filter_map(|(ordinal, value)| (ordinal % 2 == 0).then_some(value))
        .collect()
}

fn road_corridor_ref(key: &str) -> Result<RoadCorridorReference, GeneratorError> {
    Ok(RoadCorridorReference::local(key.to_owned())?)
}

fn road_section_ref(corridor: &str, section: &str) -> Result<RoadSectionReference, GeneratorError> {
    Ok(RoadSectionReference::owner_scoped(
        vec![corridor.to_owned()],
        section.to_owned(),
    )?)
}

fn authoring_lane_ref(
    corridor: &str,
    section: &str,
    lane: &str,
) -> Result<laneflow_compiler::road_editing::AuthoringLaneReference, GeneratorError> {
    Ok(
        laneflow_compiler::road_editing::AuthoringLaneReference::owner_scoped(
            vec![corridor.to_owned(), section.to_owned()],
            lane.to_owned(),
        )?,
    )
}

fn facility_band_ref(
    corridor: &str,
    band: &str,
) -> Result<laneflow_compiler::road_editing::FacilityBandReference, GeneratorError> {
    Ok(
        laneflow_compiler::road_editing::FacilityBandReference::owner_scoped(
            vec![corridor.to_owned()],
            band.to_owned(),
        )?,
    )
}

fn lane_group_ref(
    corridor: &str,
    section: &str,
    group: &str,
) -> Result<LaneGroupReference, GeneratorError> {
    Ok(LaneGroupReference::owner_scoped(
        vec![corridor.to_owned(), section.to_owned()],
        group.to_owned(),
    )?)
}

fn lane_edge_ref(key: &str) -> Result<LaneEdgeReference, GeneratorError> {
    Ok(LaneEdgeReference::local(key.to_owned())?)
}

fn canonical_frame_ref(
    key: &str,
) -> Result<laneflow_compiler::road_editing::CanonicalFrameReference, GeneratorError> {
    Ok(laneflow_compiler::road_editing::CanonicalFrameReference::local(key.to_owned())?)
}

fn junction_ref(
    key: &str,
) -> Result<laneflow_compiler::road_editing::JunctionReference, GeneratorError> {
    Ok(laneflow_compiler::road_editing::JunctionReference::local(
        key.to_owned(),
    )?)
}

fn movement_ref(
    junction: &str,
    movement: &str,
) -> Result<laneflow_compiler::road_editing::MovementReference, GeneratorError> {
    Ok(
        laneflow_compiler::road_editing::MovementReference::owner_scoped(
            vec![junction.to_owned()],
            movement.to_owned(),
        )?,
    )
}

fn maneuver_path_ref(
    junction: &str,
    movement: &str,
    path: &str,
) -> Result<ManeuverPathReference, GeneratorError> {
    Ok(ManeuverPathReference::owner_scoped(
        vec![junction.to_owned(), movement.to_owned()],
        path.to_owned(),
    )?)
}

fn signal_group_ref(key: &str) -> Result<SignalGroupReference, GeneratorError> {
    Ok(SignalGroupReference::local(key.to_owned())?)
}

fn signal_controller_ref(key: &str) -> Result<SignalControllerReference, GeneratorError> {
    Ok(SignalControllerReference::local(key.to_owned())?)
}

fn signal_phase_ref(controller: &str, phase: &str) -> Result<SignalPhaseReference, GeneratorError> {
    Ok(SignalPhaseReference::owner_scoped(
        vec![controller.to_owned()],
        phase.to_owned(),
    )?)
}

fn parking_area_ref(key: &str) -> Result<ParkingAreaReference, GeneratorError> {
    Ok(ParkingAreaReference::local(key.to_owned())?)
}

fn participant_class_ref(key: &str) -> Result<ParticipantClassReference, GeneratorError> {
    Ok(ParticipantClassReference::local(key.to_owned())?)
}

fn stop_line_ref(key: &str) -> Result<StopLineReference, GeneratorError> {
    Ok(StopLineReference::local(key.to_owned())?)
}

fn contract(message: impl Into<String>) -> GeneratorError {
    GeneratorError::Contract(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use laneflow_compiler::road_editing::RoadEditingCurveSegmentGeometry;

    fn repository_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("research crate is two levels below repository root")
            .to_path_buf()
    }

    #[test]
    fn base_modules_build_and_encode_deterministically() {
        let limits = CompileLimits::p100_initial_v2();
        let typed = build_base_modules(
            &repository_root(),
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            &limits,
        )
        .unwrap();
        assert_eq!(typed.len(), 5);
        assert!(typed.iter().all(|module| {
            module.module().declarations().len() == 343
                && module.module().road_alignments().len() == 7
        }));
        let first = encode_modules(typed, &limits).unwrap();
        let second = encode_modules(
            build_base_modules(
                &repository_root(),
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Balanced2Deg,
                &limits,
            )
            .unwrap(),
            &limits,
        )
        .unwrap();

        assert_eq!(first.len(), 5);
        assert_eq!(second.len(), 5);
        for (left, right) in first.iter().zip(&second) {
            assert_eq!(left.module_index(), right.module_index());
            assert_eq!(left.namespace(), right.namespace());
            assert_eq!(left.source_document_key(), right.source_document_key());
            assert_eq!(left.as_bytes(), right.as_bytes());
            assert_eq!(
                u32::from_le_bytes(left.as_bytes()[0..4].try_into().unwrap()) as usize + 4,
                left.as_bytes().len()
            );
            assert_eq!(&left.as_bytes()[8..12], b"LFRE");
        }

        let output = compile_encoded_modules(&first, limits)
            .unwrap_or_else(|error| panic!("P100 production compile failed: {error}"));
        let lir = output.lir();
        assert_eq!(lir.road_corridors().count(), 35);
        assert_eq!(lir.road_sections().count(), 70);
        assert_eq!(lir.authoring_lanes().count(), 170);
        assert_eq!(output.lir().lane_edges().count(), 330);
        assert_eq!(lir.junctions().count(), 10);
        assert_eq!(lir.movements().count(), 160);
        assert_eq!(lir.maneuver_paths().count(), 160);
        assert_eq!(lir.maneuver_gates().count(), 165);
        assert_eq!(lir.waiting_zones().count(), 5);
        assert_eq!(lir.stop_lines().count(), 105);
        assert_eq!(lir.signal_groups().count(), 40);
        assert_eq!(lir.signal_controllers().count(), 10);
        assert_eq!(lir.signal_phases().count(), 120);
        assert_eq!(lir.parking_areas().count(), 5);
        assert_eq!(lir.parking_spaces().count(), 5);
        assert_eq!(lir.lane_groups().count(), 30);
        assert_eq!(lir.facility_bands().count(), 35);
        assert_eq!(lir.participant_classes().count(), 15);
        assert_eq!(lir.access_rules().count(), 90);
        assert_eq!(lir.vehicle_profiles().count(), 10);
        assert_eq!(lir.static_routes().count(), 140);
        assert_eq!(lir.canonical_frames().count(), 5);
        assert_eq!(output.diagnostics(), []);
    }

    #[test]
    fn all_nine_profile_combinations_encode_uniquely_and_compile() {
        let limits = CompileLimits::p100_initial_v2();
        let expected_codes = [
            (1, 1),
            (1, 2),
            (1, 3),
            (2, 1),
            (2, 2),
            (2, 3),
            (3, 1),
            (3, 2),
            (3, 3),
        ];
        let actual_codes = P100_PROFILE_COMBINATIONS
            .map(|combination| (combination.accuracy() as u8, combination.direction() as u8));
        assert_eq!(actual_codes, expected_codes);

        let mut fixture_digests = BTreeSet::new();
        for combination in P100_PROFILE_COMBINATIONS {
            let encoded = encode_modules(
                build_base_modules(
                    &repository_root(),
                    combination.accuracy(),
                    combination.direction(),
                    &limits,
                )
                .unwrap(),
                &limits,
            )
            .unwrap();
            assert_eq!(encoded.len(), 5);
            for module in &encoded {
                assert!(fixture_digests.insert(module.sha256()));
            }
            let output =
                compile_encoded_modules(&encoded, limits.clone()).unwrap_or_else(|error| {
                    panic!(
                        "P100 profile {}/{} failed production compile: {error}",
                        combination.accuracy() as u8,
                        combination.direction() as u8
                    )
                });
            assert_eq!(output.diagnostics(), []);
            let metrics = output.metrics();
            let expected_source_bytes = encoded.iter().fold(0_u64, |total, module| {
                total.saturating_add(u64::try_from(module.as_bytes().len()).unwrap())
            });
            assert_eq!(metrics.source_bytes_total(), expected_source_bytes);
            assert_eq!(metrics.verified_table_occurrence_count(), 3_165);
            assert!(metrics.geometry_point_count() > 0);
            assert_eq!(metrics.total_horizontal_regularity_node_visits(), 0);
            assert_eq!(
                metrics
                    .maximum_horizontal_regularity_node_visits_per_offset_bearing_source_segment(),
                0
            );
            assert!(
                metrics.compiler_controlled_peak_bytes()
                    >= metrics.frontend_controlled_peak_bytes()
            );
        }
        assert_eq!(fixture_digests.len(), 45);
    }

    #[test]
    fn width_profiles_keep_side_lanes_constant_and_exact_counts() {
        let data = load_bound_seed_data(&repository_root()).unwrap();
        let profiles = WidthProfiles::new(&data.documents).unwrap();
        let mut constant = 0;
        let mut widening = 0;
        let mut narrowing = 0;
        let mut zero_to_positive = 0;
        let mut positive_to_zero = 0;
        for (key, (start, end)) in &profiles.profiles {
            if key.kind == MemberKind::AuthoringLane
                && key.corridor_key.starts_with("corridor-side-")
            {
                assert_eq!(start.to_bits(), end.to_bits(), "{}", key.local_key);
            }
            match (*start, *end) {
                (0.0, value) if value > 0.0 => zero_to_positive += 1,
                (value, 0.0) if value > 0.0 => positive_to_zero += 1,
                (left, right) if left < right => widening += 1,
                (left, right) if left > right => narrowing += 1,
                _ => constant += 1,
            }
        }
        assert_eq!(
            (
                constant,
                widening,
                narrowing,
                zero_to_positive,
                positive_to_zero,
            ),
            (175, 10, 10, 5, 5)
        );
    }

    #[test]
    fn regularity_probe_changes_only_the_frozen_curve_and_compiles() {
        let limits = CompileLimits::p100_initial_v2();
        let base = build_base_modules(
            &repository_root(),
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            &limits,
        )
        .unwrap();
        let probe = build_regularity_probe_modules(&repository_root(), &limits).unwrap();
        assert_eq!(base.len(), 5);
        assert_eq!(probe.len(), 5);

        for (base_module, probe_module) in base.iter().zip(&probe) {
            assert_eq!(base_module.module_index(), probe_module.module_index());
            assert_eq!(
                base_module.module().header(),
                probe_module.module().header()
            );
            assert_eq!(
                base_module.module().declarations(),
                probe_module.module().declarations()
            );
            assert_eq!(
                base_module.module().geometry_accuracy_profile(),
                GeometryAccuracyProfile::Fine2Cm
            );
            assert_eq!(
                probe_module.module().geometry_direction_profile(),
                GeometryDirectionProfile::Smooth1Deg
            );
            assert_alignment_delta(base_module, probe_module);
        }

        assert_eq!(curve_counts(&base), (175, 100, 200));
        assert_eq!(curve_counts(&probe), (174, 101, 202));

        let base_encoded = encode_modules(base, &limits).unwrap();
        let probe_encoded = encode_modules(probe, &limits).unwrap();
        assert_ne!(base_encoded[0].as_bytes(), probe_encoded[0].as_bytes());
        for (base_module, probe_module) in base_encoded[1..].iter().zip(&probe_encoded[1..]) {
            assert_eq!(base_module.as_bytes(), probe_module.as_bytes());
        }
        let (output, durations) = compile_encoded_modules_with_stage_timing(&probe_encoded, limits)
            .unwrap_or_else(|error| panic!("regularity probe failed production compile: {error}"));
        assert_eq!(output.diagnostics(), []);
        let metrics = output.metrics();
        let expected_source_bytes = probe_encoded.iter().fold(0_u64, |total, module| {
            total.saturating_add(u64::try_from(module.as_bytes().len()).unwrap())
        });
        assert_eq!(metrics.source_bytes_total(), expected_source_bytes);
        assert_eq!(metrics.verified_table_occurrence_count(), 3_165);
        assert!(metrics.geometry_point_count() > 0);
        assert_eq!(metrics.total_horizontal_regularity_node_visits(), 3);
        assert_eq!(
            metrics.maximum_horizontal_regularity_node_visits_per_offset_bearing_source_segment(),
            3
        );
        assert!(
            metrics.compiler_controlled_peak_bytes() >= metrics.frontend_controlled_peak_bytes()
        );
        assert!(durations.size_prefix_and_identifier_preflight() > Duration::ZERO);
        assert!(durations.flatbuffers_verifier() > Duration::ZERO);
        assert!(durations.semantic_preflight_and_typed_ast_lowering() > Duration::ZERO);
        let named_stages = durations
            .size_prefix_and_identifier_preflight()
            .saturating_add(durations.flatbuffers_verifier())
            .saturating_add(durations.semantic_preflight_and_typed_ast_lowering());
        assert!(durations.complete_compile() >= named_stages);
    }

    fn assert_alignment_delta(base: &TypedP100Module, probe: &TypedP100Module) {
        let base_alignments = base.module().road_alignments();
        let probe_alignments = probe.module().road_alignments();
        assert_eq!(base_alignments.len(), probe_alignments.len());
        for (base_alignment, probe_alignment) in base_alignments.iter().zip(probe_alignments) {
            assert_eq!(
                base_alignment.road_alignment_key(),
                probe_alignment.road_alignment_key()
            );
            if base.module_index() == REGULARITY_MODULE_INDEX as u8
                && base_alignment.road_alignment_key() == REGULARITY_ALIGNMENT_KEY
            {
                assert_eq!(
                    base_alignment.reference_line().start(),
                    probe_alignment.reference_line().start()
                );
                assert_eq!(
                    base_alignment.reference_line().segments()[0].geometry(),
                    RoadEditingCurveSegmentGeometry::Line {
                        end: point([189.5, 0.0, 0.0]).unwrap()
                    }
                );
                assert_eq!(
                    probe_alignment.reference_line().segments()[0].geometry(),
                    RoadEditingCurveSegmentGeometry::CubicBezier {
                        control_1: point([20.0, 0.0, 20.0]).unwrap(),
                        control_2: point([20.0, 0.0, 0.0]).unwrap(),
                        end: point([189.5, 0.0, 0.0]).unwrap(),
                    }
                );
                assert_eq!(
                    base_alignment.canonical_frame(),
                    probe_alignment.canonical_frame()
                );
                assert_eq!(
                    base_alignment.canvas_selection(),
                    probe_alignment.canvas_selection()
                );
            } else {
                assert_eq!(base_alignment, probe_alignment);
            }
        }
    }

    fn curve_counts(modules: &[TypedP100Module]) -> (usize, usize, usize) {
        let mut counts = (0, 0, 0);
        for module in modules {
            for alignment in module.module().road_alignments() {
                add_curve_counts(alignment.reference_line(), &mut counts);
            }
            for declaration in module.module().declarations() {
                if let RoadEditingDeclaration::LaneEdge(edge) = declaration
                    && let Some(curve) = edge.explicit_geometry()
                {
                    add_curve_counts(curve, &mut counts);
                }
            }
        }
        counts
    }

    fn add_curve_counts(program: &RoadEditingCurveProgram, counts: &mut (usize, usize, usize)) {
        for segment in program.segments() {
            match segment.geometry() {
                RoadEditingCurveSegmentGeometry::Line { .. } => counts.0 += 1,
                RoadEditingCurveSegmentGeometry::CubicBezier { .. } => {
                    counts.1 += 1;
                    counts.2 += 2;
                }
            }
        }
    }
}
