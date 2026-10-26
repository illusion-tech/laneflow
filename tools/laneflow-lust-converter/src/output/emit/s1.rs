//! #1093：保留生产声明和控制锚点的真实完整路径研究。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use laneflow_compiler::{DiagnosticBundle, DiagnosticPayload, SpatialGeometryViolation};
use serde_json::{Value, json};

use super::*;
use crate::output::geom::s1::{self as sampler, Candidate, program_bits, program_points};
use crate::output::model::ManeuverPath;

const ASSIGNMENT_LIMIT: usize = 128;

mod measurement;

thread_local! {
    static ORIGINAL_PATH_KEYS: RefCell<Option<HashMap<String, String>>> = const { RefCell::new(None) };
}

struct PathKeysReset;
impl Drop for PathKeysReset {
    fn drop(&mut self) {
        ORIGINAL_PATH_KEYS.with(|keys| *keys.borrow_mut() = None);
    }
}

pub(super) fn preserve_path_keys(keys: &mut HashMap<String, PathKeys>) {
    ORIGINAL_PATH_KEYS.with(|original| {
        if let Some(original) = original.borrow().as_ref() {
            for (id, key) in keys {
                key.path_key = original[id].clone();
            }
        }
    });
}

fn path_edges(path: &ManeuverPath) -> Vec<&str> {
    std::iter::once(path.entry_edge_id.as_str())
        .chain(path.internal_edge_ids.iter().map(String::as_str))
        .chain(std::iter::once(path.exit_edge_id.as_str()))
        .collect()
}

fn clip(traffic: &TrafficPackage, path: &ManeuverPath) -> TrafficPackage {
    // 保留完整控制器、相位及其关联路径，不改写相位或移除控制锚点。
    let mut paths = HashSet::from([path.id.as_str()]);
    loop {
        let required = traffic
            .signals
            .maneuver_gates
            .iter()
            .filter(|g| paths.contains(g.maneuver_path_id.as_str()))
            .map(|g| g.signal_control.group_id.as_str())
            .collect::<HashSet<_>>();
        let groups = traffic
            .signals
            .controllers
            .iter()
            .filter(|c| c.group_ids.iter().any(|g| required.contains(g.as_str())))
            .flat_map(|c| c.group_ids.iter().map(String::as_str))
            .collect::<HashSet<_>>();
        let before = paths.len();
        paths.extend(
            traffic
                .signals
                .maneuver_gates
                .iter()
                .filter(|g| groups.contains(g.signal_control.group_id.as_str()))
                .map(|g| g.maneuver_path_id.as_str()),
        );
        if paths.len() == before {
            break;
        }
    }
    let mut part = traffic.clone();
    part.maneuver_paths
        .retain(|p| paths.contains(p.id.as_str()));
    let movements = part
        .maneuver_paths
        .iter()
        .map(|p| p.movement_id.as_str())
        .collect::<HashSet<_>>();
    part.movements.retain(|m| movements.contains(m.id.as_str()));
    let junctions = part
        .movements
        .iter()
        .map(|m| m.junction_id.as_str())
        .collect::<HashSet<_>>();
    part.junctions.retain(|j| junctions.contains(j.id.as_str()));
    part.routes.clear();
    part.signals
        .maneuver_gates
        .retain(|g| paths.contains(g.maneuver_path_id.as_str()));
    let required_groups = part
        .signals
        .maneuver_gates
        .iter()
        .map(|g| g.signal_control.group_id.as_str())
        .collect::<HashSet<_>>();
    part.signals.controllers.retain(|c| {
        c.group_ids
            .iter()
            .any(|g| required_groups.contains(g.as_str()))
    });
    let groups = part
        .signals
        .controllers
        .iter()
        .flat_map(|c| c.group_ids.iter().map(String::as_str))
        .collect::<HashSet<_>>();
    part.signals
        .groups
        .retain(|g| groups.contains(g.id.as_str()));
    let stops = part
        .signals
        .maneuver_gates
        .iter()
        .map(|g| g.stop_line_id.as_str())
        .collect::<HashSet<_>>();
    part.signals
        .stop_lines
        .retain(|s| stops.contains(s.id.as_str()));
    let mut edges = part
        .maneuver_paths
        .iter()
        .flat_map(path_edges)
        .collect::<HashSet<_>>();
    edges.extend(part.signals.stop_lines.iter().map(|s| s.edge_id.as_str()));
    part.lane_graph
        .edges
        .retain(|e| edges.contains(e.id.as_str()));
    part
}

fn add_programs(
    builder: &mut re::RoadEditingSourceModuleBuilder<'_>,
    traffic: &TrafficPackage,
    programs: &BTreeMap<String, re::RoadEditingCurveProgram>,
    approaches: &HashSet<&str>,
) -> Result<()> {
    let frame = re::CanonicalFrameReference::local(LUST_FRAME_ID)?;
    for edge in &traffic.lane_graph.edges {
        let key = edge_key(&edge.id);
        let geometry = programs[&edge.id].clone();
        if approaches.contains(edge.id.as_str()) {
            let corridor_key = format!("{key}.road");
            let section =
                re::RoadSectionReference::owner_scoped(vec![corridor_key.clone()], "section")?;
            let lane = re::AuthoringLaneReference::owner_scoped(
                vec![corridor_key.clone(), "section".to_owned()],
                "lane",
            )?;
            builder.add_alignment(re::RoadAlignmentInput::try_new(
                &key,
                frame.clone(),
                geometry,
            )?)?;
            builder.add_declaration(re::RoadEditingDeclaration::RoadCorridor(
                re::RoadCorridorInput::try_new(
                    &corridor_key,
                    re::RoadAlignmentReference::try_new(&key)?,
                    0.0,
                    re::RoadEditingStationEnd::AlignmentEnd,
                    section.clone(),
                    lane.clone(),
                    vec![re::RoadEditingCorridorElement::RoadSection(section.clone())],
                )?,
            ))?;
            builder.add_declaration(re::RoadEditingDeclaration::RoadSection(
                re::RoadSectionInput::try_new(
                    "section",
                    "motorLane",
                    vec![lane.clone()],
                    re::RoadCorridorReference::local(&corridor_key)?,
                )?,
            ))?;
            builder.add_declaration(re::RoadEditingDeclaration::AuthoringLane(
                re::AuthoringLaneInput::try_new(
                    "lane",
                    edge_ref(&key)?,
                    re::RoadEditingLaneDirection::Forward,
                    re::LinearWidthProfile::try_new(
                        SUMO_LANE_WIDTH_METERS,
                        SUMO_LANE_WIDTH_METERS,
                    )?,
                    None,
                    section,
                )?,
            ))?;
            builder.add_declaration(re::RoadEditingDeclaration::LaneEdge(
                re::LaneEdgeInput::try_new(&key, edge.speed_limit, Vec::new(), None)?,
            ))?;
        } else {
            builder.add_declaration(re::RoadEditingDeclaration::LaneEdge(
                re::LaneEdgeInput::try_new(&key, edge.speed_limit, Vec::new(), Some(geometry))?,
            ))?;
        }
    }
    Ok(())
}

fn rejected(stage: &str, bundle: &DiagnosticBundle, path: &ManeuverPath) -> Value {
    let pairs = path_edges(path)
        .windows(2)
        .map(|w| (edge_key(w[0]), edge_key(w[1])))
        .collect::<Vec<_>>();
    let mut status = "target_check_not_completed";
    if stage == "compile" {
        for d in bundle.diagnostics() {
            if let DiagnosticPayload::InvalidSpatialGeometry {
                lane_edge_key,
                related_lane_edge_key: Some(right),
                violation,
                ..
            } = d.payload()
                && pairs.iter().any(|(a, b)| {
                    a.as_str() == lane_edge_key.as_ref() && b.as_str() == right.as_ref()
                })
            {
                match violation {
                    SpatialGeometryViolation::DiscontinuousJoin { .. } => {
                        status = "join_position_rejected"
                    }
                    SpatialGeometryViolation::DirectionDiscontinuity { .. } => {
                        status = "cross_edge_direction_rejected"
                    }
                    _ => {}
                }
            }
        }
    }
    json!({"status":status,"stage":stage,"diagnostics":bundle.diagnostics().iter().map(|d|
        format!("{} {:?}: {:?}",d.code().as_str(),d.stable_key(),d.payload())).collect::<Vec<_>>()})
}

fn compile_path(
    traffic: &TrafficPackage,
    path: &ManeuverPath,
    programs: &BTreeMap<String, re::RoadEditingCurveProgram>,
) -> Value {
    let part = clip(traffic, path);
    let dependency_paths = part
        .maneuver_paths
        .iter()
        .map(|p| p.id.as_str())
        .collect::<Vec<_>>();
    let missing = part
        .lane_graph
        .edges
        .iter()
        .map(|e| e.id.as_str())
        .filter(|id| !programs.contains_key(*id))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return json!({"status":"target_check_not_completed","stage":"emission_prerequisite","missing_real_programs":missing,"dependency_paths":dependency_paths});
    }
    let original_keys = path_keys(traffic).unwrap();
    let original_key = original_keys[&path.id].path_key.clone();
    ORIGINAL_PATH_KEYS.with(|slot| {
        assert!(slot.borrow().is_none(), "nested path key override");
        *slot.borrow_mut() = Some(
            original_keys
                .into_iter()
                .map(|(id, key)| (id, key.path_key))
                .collect(),
        );
    });
    let _reset = PathKeysReset;
    assert_eq!(path_keys(&part).unwrap()[&path.id].path_key, original_key);
    let approaches = traffic
        .maneuver_paths
        .iter()
        .flat_map(|p| [p.entry_edge_id.as_str(), p.exit_edge_id.as_str()])
        .collect();
    let limits = CompileLimits::single_network_1m_v2();
    let header = re::RoadEditingModuleHeader::try_new(
        LUST_NAMESPACE,
        DOCUMENT_KEY,
        Vec::new(),
        re::RoadEditingProvenance::generated(
            GENERATOR_BUILD_ID,
            [0; 32],
            [0; 32],
            None,
            PROVENANCE,
        )
        .unwrap(),
    )
    .unwrap();
    let mut builder = re::RoadEditingSourceModuleBuilder::new(
        header,
        GeometryAccuracyProfile::Balanced5Cm,
        GeometryDirectionProfile::Balanced2Deg,
        &limits,
    )
    .unwrap();
    let declared = (|| -> Result<()> {
        add_profiles(&mut builder, &part)?;
        builder.add_declaration(re::RoadEditingDeclaration::CanonicalFrame(
            re::CanonicalFrameInput::try_new(LUST_FRAME_ID)?,
        ))?;
        add_programs(&mut builder, &part, programs, &approaches)?;
        add_junctions(&mut builder, &part)?;
        add_signals(&mut builder, &part)?;
        Ok(())
    })();
    if let Err(error) = declared {
        return json!({"status":"target_check_not_completed","stage":"declaration","error":error.to_string()});
    }
    let model = match builder.finish() {
        Ok(v) => v,
        Err(e) => return rejected("finish", &e, path),
    };
    let buffer = match re::RoadEditingSourceWriter::new(&limits).write(model) {
        Ok(v) => v,
        Err(e) => return rejected("write", &e, path),
    };
    let mut unit = CompilationUnitBuilder::new(limits);
    if let Err(e) = unit.add_road_editing_module(
        re::RoadEditingModuleInput::try_new(DOCUMENT_KEY, buffer.as_bytes(), None).unwrap(),
    ) {
        return rejected("module_admission", &e, path);
    }
    let unit = match unit.build() {
        Ok(v) => v,
        Err(e) => return rejected("unit_build", &e, path),
    };
    match Compiler::new().compile(unit) {
        Err(e) => rejected("compile", &e, path),
        Ok(output) => {
            let edges=part.lane_graph.edges.iter().map(|source| {
                let key=edge_key(&source.id);
                let matched=output.lir().lane_edges().filter(|e|e.identity_fields().any(|f|f.value_bytes()==key.as_bytes())).collect::<Vec<_>>();
                assert_eq!(matched.len(),1,"compiled identity");let edge=matched[0];
                let geometry=edge.spatial_geometry().unwrap();
                let actual=geometry.points().map(|p|[p.x.to_bits(),p.y.to_bits(),p.z.to_bits()]).collect::<Vec<_>>();
                let expected=program_points(&programs[&source.id]).into_iter().map(|p|p.map(|x|(x as f32).to_bits())).collect::<Vec<_>>();
                assert_eq!(actual,expected,"complete canonical program");
                json!({"edge":source.id,"length_mm":edge.length_mm(),"canonical_arc_m":geometry.arc_length_meters(),
                    "source_length_m":source.length,"source_length_delta_m":f64::from(edge.length_mm())/1_000.0-source.length,"all_points_match":true})
            }).collect::<Vec<_>>();
            json!({"status":"accepted","stage":"compile","edges":edges,"original_path_key":original_key,"dependency_paths":dependency_paths,
                "stop_lines":part.signals.stop_lines,"maneuver_gates":part.signals.maneuver_gates})
        }
    }
}

fn run(observation: &observe::EmissionObservation, samples: &Value) -> (Value, Value) {
    let traffic = observation.traffic.as_ref().unwrap();
    let selected = samples["samples"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["edge"].as_str().unwrap())
        .collect::<HashSet<_>>();
    assert_eq!(selected.len(), 17);
    let mut programs = BTreeMap::new();
    let mut pools = BTreeMap::<String, Vec<Candidate>>::new();
    let mut rows = BTreeMap::new();
    let mut times = BTreeMap::new();
    let mut preserved = 0;
    let mut success_replay_seconds = 0.0;
    for ((id, original), input) in observation.edges.iter().zip(&observation.inputs) {
        if let observe::EdgeEmission::Emitted(program) = original {
            let replay_started = Instant::now();
            // 逐个重跑全部成功边，比较完整 f64 程序，而非只比较段数或端点。
            let replay = geom::repair_curve(
                &input.points,
                input.clamps[0],
                input.clamps[1],
                input.clamp_sources[0].as_ref(),
                input.clamp_sources[1].as_ref(),
            )
            .unwrap();
            assert_eq!(program_bits(program), program_bits(&replay.program));
            success_replay_seconds += replay_started.elapsed().as_secs_f64();
            preserved += 1;
            programs.insert(id.clone(), program.clone());
        }
        if !selected.contains(id.as_str()) {
            continue;
        }
        let expected = samples["samples"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["edge"] == id.as_str())
            .unwrap();
        assert_eq!(
            json!(
                input
                    .points
                    .iter()
                    .map(|p| p.map(f64::to_bits))
                    .collect::<Vec<_>>()
            ),
            expected["input_points_f64_bits"]
        );
        let start = Instant::now();
        assert_eq!(
            json!(input.clamps.map(|c| c.map(|p| p.map(f64::to_bits)))),
            expected["clamps_f64_bits"]
        );
        if let observe::EdgeEmission::Failed(message) = original {
            assert_eq!(expected["outcome"]["error"], message.as_str());
        }
        let row = match original {
            observe::EdgeEmission::Emitted(program) => {
                json!({"original":"emitted","status":"preserved_success","program":program_bits(program)})
            }
            observe::EdgeEmission::Failed(message) => {
                let search = sampler::search(input);
                let values=search.candidates.iter().map(|c|json!({"program":program_bits(&c.repaired.program),"arc_m":c.repaired.quantized_arc_meters,"spans":c.spans})).collect::<Vec<_>>();
                let row = json!({"original":"emission_failed","original_error":message,"search":search.report,"candidates":values});
                if !search.candidates.is_empty() {
                    pools.insert(id.clone(), search.candidates);
                }
                row
            }
        };
        times.insert(id.clone(), start.elapsed().as_secs_f64());
        eprintln!(
            "S1 sample {id}: {}",
            row.get("search").unwrap_or(&row["status"])
        );
        rows.insert(id.clone(), row);
    }
    assert_eq!(rows.len(), 17);
    assert_eq!(preserved, 16_265);
    let mut paths = traffic
        .maneuver_paths
        .iter()
        .filter(|p| path_edges(p).iter().any(|e| selected.contains(e)))
        .collect::<Vec<_>>();
    paths.sort_by(|a, b| a.id.cmp(&b.id));
    let baseline=paths.iter().map(|p|json!({"path":p.id,"edges":path_edges(p),"result":compile_path(traffic,p,&programs)})).collect::<Vec<_>>();
    let keys = pools.keys().cloned().collect::<Vec<_>>();
    let mut choices = vec![0; keys.len()];
    let mut trials = Vec::new();
    let mut best_paths = Vec::new();
    let mut best_accepted = 0;
    let mut best_assignment = Value::Null;
    let mut assignment_exhausted = false;
    let compile_started = Instant::now();
    for trial in 0..ASSIGNMENT_LIMIT {
        for (i, key) in keys.iter().enumerate() {
            programs.insert(key.clone(), pools[key][choices[i]].repaired.program.clone());
        }
        let assignment = keys
            .iter()
            .zip(&choices)
            .map(|(k, c)| (k.clone(), *c))
            .collect::<BTreeMap<_, _>>();
        let mut results = Vec::new();
        let mut counts = BTreeMap::<String, usize>::new();
        for path in &paths {
            let result = compile_path(traffic, path, &programs);
            *counts
                .entry(result["status"].as_str().unwrap().to_owned())
                .or_default() += 1;
            results.push(json!({"path":path.id,"edges":path_edges(path),"result":result}));
        }
        let accepted = counts.get("accepted").copied().unwrap_or(0);
        trials.push(json!({"trial":trial,"assignment":assignment,"counts":counts}));
        if trial == 0 || accepted > best_accepted {
            best_accepted = accepted;
            best_paths = results;
            best_assignment = json!(assignment);
        }
        // 每轮的所有路径消费同一赋值；按边名和候选序号遍历笛卡尔积。
        let mut advanced = false;
        for i in (0..keys.len()).rev() {
            choices[i] += 1;
            if choices[i] < pools[&keys[i]].len() {
                advanced = true;
                break;
            }
            choices[i] = 0;
        }
        if !advanced {
            assignment_exhausted = true;
            break;
        }
    }
    let mut policy = sampler::policy();
    policy["assignments"] = json!(ASSIGNMENT_LIMIT);
    (
        json!({"schema":"laneflow-1093-s1-research","base_commit":"6d403d00bb11364ab21d2c1f61bbd9f17ce7b8c8",
        "policy":policy,
        "successful_programs_replayed_equal":preserved,"samples":rows,"affected_paths":paths.len(),"baseline_paths":baseline,
        "assignment_trials":trials,"retained_pool_assignments_exhausted":assignment_exhausted,"best_assignment":best_assignment,
        "best_assignment_paths":best_paths,"g1_accepted":false}),
        json!({"edge_search_seconds":times,"success_replay_and_compare_seconds":success_replay_seconds,"assignment_compile_seconds":compile_started.elapsed().as_secs_f64()}),
    )
}

fn small_observation() -> observe::EmissionObservation {
    use crate::convert::profiles::{convert_vehicle_profiles, select_passenger_vtypes};
    use crate::convert::topology::{
        TopologyConvertOptions, convert_network_topology_with_tll_and_profiles,
    };
    use crate::sumo::{parse_sumo_network_xml, parse_tll_static_xml, parse_vtypes_xml};
    let net = parse_sumo_network_xml(include_str!(
        "../../../tests/fixtures/minimal/t-junction.net.xml"
    ))
    .unwrap();
    let tll = parse_tll_static_xml(include_str!(
        "../../../tests/fixtures/minimal/t-junction.tll.xml"
    ))
    .unwrap();
    let vtypes = parse_vtypes_xml(include_str!(
        "../../../tests/fixtures/minimal/vtypes.add.xml"
    ))
    .unwrap();
    let profiles = convert_vehicle_profiles(&select_passenger_vtypes(&vtypes).unwrap()).unwrap();
    let (result, observation) = observe::capture(|| {
        convert_network_topology_with_tll_and_profiles(
            &net,
            &tll,
            &profiles,
            &TopologyConvertOptions::default(),
            geom::ReportSource::unverified_unknown(),
        )
    });
    result.unwrap();
    observation
}

fn small_programs(
    observation: &observe::EmissionObservation,
) -> BTreeMap<String, re::RoadEditingCurveProgram> {
    observation
        .edges
        .iter()
        .map(|(id, e)| {
            let observe::EdgeEmission::Emitted(p) = e else {
                panic!("fixture emitted");
            };
            (id.clone(), p.clone())
        })
        .collect()
}

fn fixture_point(value: [f64; 3]) -> re::RoadEditingPoint3 {
    re::RoadEditingPoint3::try_new(value[0], value[1], value[2]).unwrap()
}

fn fixture_translate(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] + b[i])
}

#[test]
fn full_path_keeps_source_ordinal_and_control_anchors() {
    let observation = small_observation();
    let mut traffic = observation.traffic.as_ref().unwrap().clone();
    let selected = traffic.maneuver_paths[0].clone();
    let mut preceding = selected.clone();
    preceding.id.push_str("-preceding");
    traffic.maneuver_paths.insert(0, preceding);
    let programs = small_programs(&observation);
    let result = compile_path(&traffic, &selected, &programs);
    assert_eq!(result["status"], "accepted", "{result}");
    assert_eq!(result["original_path_key"], "path-1");
    assert_eq!(
        result["edges"].as_array().unwrap().len(),
        observation.edges.len()
    );
    assert_eq!(result["maneuver_gates"].as_array().unwrap().len(), 2);
    assert_eq!(result["stop_lines"].as_array().unwrap().len(), 1);
    assert_eq!(
        serde_json::to_value(&traffic.signals.maneuver_gates).unwrap(),
        result["maneuver_gates"]
    );
    assert!(ORIGINAL_PATH_KEYS.with(|keys| keys.borrow().is_none()));
}

#[test]
fn missing_real_program_is_not_a_compiler_rejection() {
    let observation = small_observation();
    let traffic = observation.traffic.as_ref().unwrap();
    let path = &traffic.maneuver_paths[0];
    let mut programs = small_programs(&observation);
    programs.remove(&path.entry_edge_id);
    let result = compile_path(traffic, path, &programs);
    assert_eq!(result["status"], "target_check_not_completed");
    assert_eq!(result["stage"], "emission_prerequisite");
    assert_eq!(result["missing_real_programs"], json!([path.entry_edge_id]));
}

#[test]
fn complete_path_keeps_multiple_internal_edges() {
    let observation = small_observation();
    let mut traffic = observation.traffic.as_ref().unwrap().clone();
    let path = traffic
        .maneuver_paths
        .iter_mut()
        .find(|p| p.exit_edge_id == "sumo:east_0")
        .unwrap();
    let first = path.internal_edge_ids[0].clone();
    let second = format!("{first}-tail");
    path.internal_edge_ids.push(second.clone());
    let path = path.clone();
    let mut programs = small_programs(&observation);
    let points = program_points(&programs[&first]);
    let a = points[0];
    let b = *points.last().unwrap();
    let m = std::array::from_fn(|i| (a[i] + b[i]) * 0.5);
    let line = |a, b| {
        re::RoadEditingCurveProgram::try_new(
            fixture_point(a),
            vec![re::RoadEditingCurveSegment::line(fixture_point(b))],
        )
        .unwrap()
    };
    programs.insert(first.clone(), line(a, m));
    programs.insert(second.clone(), line(m, b));
    let mut edge = traffic
        .lane_graph
        .edges
        .iter()
        .find(|e| e.id == first)
        .unwrap()
        .clone();
    edge.id = second;
    traffic.lane_graph.edges.push(edge);
    let result = compile_path(&traffic, &path, &programs);
    assert_eq!(result["status"], "accepted", "{result}");
    assert_eq!(
        result["edges"].as_array().unwrap().len(),
        observation.edges.len() + 1
    );
}

#[test]
fn real_compiler_separates_seam_failure_from_geometry_prerequisite() {
    let observation = small_observation();
    let traffic = observation.traffic.as_ref().unwrap();
    let path = traffic
        .maneuver_paths
        .iter()
        .find(|p| p.exit_edge_id == "sumo:east_0")
        .unwrap();
    let mut programs = small_programs(&observation);
    let key = &path.internal_edge_ids[0];
    let points = program_points(&programs[key]);
    let program = |points: &[[f64; 3]]| {
        re::RoadEditingCurveProgram::try_new(
            fixture_point(points[0]),
            points[1..]
                .iter()
                .map(|p| re::RoadEditingCurveSegment::line(fixture_point(*p)))
                .collect(),
        )
        .unwrap()
    };
    let moved = points
        .iter()
        .map(|p| fixture_translate(*p, [0.0, 0.2, 0.0]))
        .collect::<Vec<_>>();
    programs.insert(key.clone(), program(&moved));
    let seam = compile_path(traffic, path, &programs);
    assert_eq!(seam["status"], "join_position_rejected", "{seam}");
    programs.insert(
        key.clone(),
        program(&[
            points[0],
            fixture_translate(points[0], [0.01, 0.0, 0.0]),
            *points.last().unwrap(),
        ]),
    );
    let admission = compile_path(traffic, path, &programs);
    assert_eq!(admission["status"], "target_check_not_completed");
    assert_eq!(admission["stage"], "compile");
    assert!(
        admission["diagnostics"]
            .to_string()
            .contains("DegenerateSegment"),
        "{admission}"
    );
}

#[test]
#[ignore = "requires verified LUST_SOURCE_DIR and fresh S1_OUTPUT_DIR"]
fn pinned_lust_s1_sample_validation() {
    use crate::convert::profiles::{convert_vehicle_profiles, select_passenger_vtypes};
    use crate::convert::topology::{
        TopologyConvertOptions, convert_network_topology_with_tll_and_profiles,
    };
    use crate::source::verify::prepare_verified_lust_inputs;
    use crate::sumo::{parse_sumo_network_xml, parse_tll_static_xml, parse_vtypes_xml};
    let started = Instant::now();
    let root = PathBuf::from(std::env::var("LUST_SOURCE_DIR").expect("LUST_SOURCE_DIR"));
    let out = PathBuf::from(std::env::var("S1_OUTPUT_DIR").expect("S1_OUTPUT_DIR"));
    if out.exists() {
        assert!(
            fs::read_dir(&out).unwrap().next().is_none(),
            "fresh output directory"
        );
    }
    fs::create_dir_all(&out).unwrap();
    let prepared = prepare_verified_lust_inputs(&root).expect("verified pinned source");
    let network = parse_sumo_network_xml(prepared.net_xml()).unwrap();
    let tll = parse_tll_static_xml(prepared.tll_xml()).unwrap();
    let vtypes = parse_vtypes_xml(prepared.vtypes_xml()).unwrap();
    let profiles = convert_vehicle_profiles(&select_passenger_vtypes(&vtypes).unwrap()).unwrap();
    let options = TopologyConvertOptions {
        require_lust_location_anchors: true,
        emit_infeasibility_report: true,
        ..TopologyConvertOptions::default()
    };
    let prepare_seconds = started.elapsed().as_secs_f64();
    let convert_started = Instant::now();
    let (original, observation) = observe::capture(|| {
        convert_network_topology_with_tll_and_profiles(
            &network,
            &tll,
            &profiles,
            &options,
            prepared.report_source().clone(),
        )
    });
    let original = original.unwrap();
    let baseline_conversion_seconds = convert_started.elapsed().as_secs_f64();
    let survey = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("evidence/lust-infeasible-survey.md"),
    )
    .unwrap();
    assert_eq!(
        original.infeasibility_report.unwrap().rendered,
        survey.replace("\r\n", "\n")
    );
    let samples: Value = serde_json::from_str(include_str!(
        "../../../../../research/issue-1084-lust-sampling/samples.json"
    ))
    .unwrap();
    measurement::write_inputs(&out, &observation, &samples);
    let (mut report, mut timing) = run(&observation, &samples);
    report["source"] = json!({"lust_commit":crate::source::LUST_COMMIT,"net_digest":prepared.report_source().net_digest()});
    timing["total_seconds"] = json!(started.elapsed().as_secs_f64());
    timing["source_verify_and_parse_seconds"] = json!(prepare_seconds);
    timing["baseline_conversion_seconds"] = json!(baseline_conversion_seconds);
    fs::write(
        out.join("results.json"),
        format!("{}\n", serde_json::to_string_pretty(&report).unwrap()),
    )
    .unwrap();
    fs::write(
        out.join("timing.json"),
        format!("{}\n", serde_json::to_string_pretty(&timing).unwrap()),
    )
    .unwrap();
    println!(
        "S1-RESULT paths={} trials={} saved={}",
        report["affected_paths"],
        report["assignment_trials"].as_array().unwrap().len(),
        out.display()
    );
}
