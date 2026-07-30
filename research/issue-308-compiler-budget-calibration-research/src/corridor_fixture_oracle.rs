//! 由当前 production loader 规范化结果独立重建走廊研究模板。

use crate::corridor::{
    CorridorTemplate, EntityRef, TemplateEntity, TemplateGeometry, TemplateGeometryRule,
    TemplateRelation,
};
use laneflow_core::{
    AccessEffect, AccessTargetId, CorridorElementId, InitialTrafficData, SignalAspect,
    SignalControlInput,
};
use laneflow_data::{NamedArtifact, from_json_slice, from_scenario_json_slice};
use std::collections::BTreeMap;

#[derive(Default)]
struct TypedDocumentRefs {
    by_kind: BTreeMap<u16, BTreeMap<String, EntityRef>>,
    lanes: Vec<Vec<EntityRef>>,
    phases: Vec<Vec<EntityRef>>,
}

impl TypedDocumentRefs {
    fn named(&self, kind: u16, id: &str) -> Result<EntityRef, String> {
        self.by_kind
            .get(&kind)
            .and_then(|values| values.get(id))
            .copied()
            .ok_or_else(|| format!("production projection cannot resolve kind={kind} id={id}"))
    }
}

pub(crate) fn build_production_loader_template() -> Result<CorridorTemplate, String> {
    let root = crate::repository_root();
    let manifest = std::fs::read(root.join("examples/data/v0.1-signalized-corridor.scenario.json"))
        .map_err(|error| error.to_string())?;
    let traffic = std::fs::read(root.join("examples/data/v0.10-signalized-corridor.laneflow.json"))
        .map_err(|error| error.to_string())?;
    let spatial = std::fs::read(root.join("examples/data/v0.1-signalized-corridor.spatial.json"))
        .map_err(|error| error.to_string())?;
    let signalized = from_scenario_json_slice(
        &manifest,
        &[
            NamedArtifact::new("v0.10-signalized-corridor.laneflow.json", &traffic),
            NamedArtifact::new("v0.1-signalized-corridor.spatial.json", &spatial),
        ],
    )
    .map_err(|error| error.to_string())?;
    let parking_bytes =
        std::fs::read(root.join("examples/data/v0.10-parking-signals-baseline.laneflow.json"))
            .map_err(|error| error.to_string())?;
    let parking = from_json_slice(&parking_bytes).map_err(|error| error.to_string())?;
    let traffic_documents = [
        signalized.traffic().initial_traffic_data(),
        parking.initial_traffic_data(),
    ];

    let mut references = [TypedDocumentRefs::default(), TypedDocumentRefs::default()];
    let mut entities = Vec::new();
    let mut next_local = [0_u32; 23];
    for kind in 1_u16..=21 {
        for (document_index, data) in traffic_documents.iter().enumerate() {
            register_entities(
                data,
                document_index,
                kind,
                &mut next_local,
                &mut references,
                &mut entities,
            )?;
        }
    }
    let frame = EntityRef { kind: 22, local: 0 };
    entities.push(TemplateEntity {
        reference: frame,
        identity_references: BTreeMap::new(),
    });
    let positions = entities
        .iter()
        .enumerate()
        .map(|(index, entity)| (entity.reference, index))
        .collect::<BTreeMap<_, _>>();
    if positions.len() != entities.len() {
        return Err("duplicate typed entity kind/local".to_owned());
    }

    let mut relations = Vec::new();
    let mut corridor_owners = BTreeMap::<EntityRef, EntityRef>::new();
    for (document_index, data) in traffic_documents.iter().enumerate() {
        let refs = &references[document_index];
        let cross = data.cross_section();
        for handle in cross.corridors() {
            let corridor = cross.corridor(handle).ok_or("missing corridor")?;
            let parent = refs.named(1, corridor.id())?;
            for element in corridor.elements() {
                let child = match element {
                    CorridorElementId::Section(id) => refs.named(2, id)?,
                    CorridorElementId::Band(id) => refs.named(17, id)?,
                };
                if corridor_owners.insert(child, parent).is_some() {
                    return Err("duplicate corridor element owner".to_owned());
                }
            }
        }
    }
    for (document_index, data) in traffic_documents.iter().enumerate() {
        let refs = &references[document_index];
        let cross = data.cross_section();
        for (section_index, handle) in cross.sections().enumerate() {
            let section = cross.section(handle).ok_or("missing section")?;
            let parent = refs.named(2, section.id())?;
            for lane in references[document_index]
                .lanes
                .get(section_index)
                .ok_or("missing typed lanes")?
            {
                set_identity(&mut entities, &positions, *lane, 32, parent)?;
                relations.push(TemplateRelation::Owner {
                    child: *lane,
                    parent,
                });
            }
        }
    }
    for (document_index, data) in traffic_documents.iter().enumerate() {
        let refs = &references[document_index];
        let junctions = data.junctions();
        for handle in junctions.movements() {
            let movement = junctions.movement(handle).ok_or("missing movement")?;
            let child = refs.named(6, movement.id())?;
            let parent = refs.named(5, movement.junction_id())?;
            set_identity(&mut entities, &positions, child, 34, parent)?;
            relations.push(TemplateRelation::Owner { child, parent });
        }
    }
    for (document_index, data) in traffic_documents.iter().enumerate() {
        let refs = &references[document_index];
        let junctions = data.junctions();
        for handle in junctions.maneuver_paths() {
            let path = junctions.maneuver_path(handle).ok_or("missing path")?;
            let child = refs.named(7, path.id())?;
            let movement = refs.named(6, path.movement_id())?;
            set_identity(&mut entities, &positions, child, 11, movement)?;
            set_identity(
                &mut entities,
                &positions,
                child,
                12,
                refs.named(4, path.entry_edge_id())?,
            )?;
            set_identity(
                &mut entities,
                &positions,
                child,
                13,
                refs.named(4, path.exit_edge_id())?,
            )?;
            relations.push(TemplateRelation::Owner {
                child,
                parent: movement,
            });
        }
    }
    for (document_index, data) in traffic_documents.iter().enumerate() {
        let refs = &references[document_index];
        let signals = data.signals();
        for handle in signals.maneuver_gates() {
            let gate = signals.maneuver_gate(handle).ok_or("missing gate")?;
            let child = refs.named(8, gate.id())?;
            let parent = refs.named(7, gate.maneuver_path_id())?;
            set_identity(&mut entities, &positions, child, 14, parent)?;
            relations.push(TemplateRelation::Owner { child, parent });
        }
    }
    for (document_index, data) in traffic_documents.iter().enumerate() {
        let refs = &references[document_index];
        for handle in data.waiting().waiting_zones() {
            let zone = data
                .waiting()
                .waiting_zone(handle)
                .ok_or("missing waiting zone")?;
            let child = refs.named(9, zone.id())?;
            let parent = refs.named(7, zone.maneuver_path_id())?;
            set_identity(&mut entities, &positions, child, 14, parent)?;
            relations.push(TemplateRelation::Owner { child, parent });
        }
    }
    for (document_index, data) in traffic_documents.iter().enumerate() {
        let refs = &references[document_index];
        let signals = data.signals();
        for (controller_index, controller) in signals.controllers().enumerate() {
            let parent = refs.named(12, controller.id())?;
            for child in references[document_index]
                .phases
                .get(controller_index)
                .ok_or("missing typed phases")?
            {
                set_identity(&mut entities, &positions, *child, 20, parent)?;
                relations.push(TemplateRelation::Owner {
                    child: *child,
                    parent,
                });
            }
        }
    }
    for (document_index, data) in traffic_documents.iter().enumerate() {
        let refs = &references[document_index];
        for space in data.parking().spaces() {
            if let Some(area_id) = space.area_id() {
                relations.push(TemplateRelation::Owner {
                    child: refs.named(15, space.id())?,
                    parent: refs.named(14, area_id)?,
                });
            }
        }
    }
    for (document_index, data) in traffic_documents.iter().enumerate() {
        let refs = &references[document_index];
        let cross = data.cross_section();
        for handle in cross.groups() {
            let group = cross.group(handle).ok_or("missing lane group")?;
            let child = refs.named(16, group.id())?;
            let parent = refs.named(2, group.road_section_id())?;
            set_identity(&mut entities, &positions, child, 32, parent)?;
            relations.push(TemplateRelation::Owner { child, parent });
        }
    }
    for (child, parent) in corridor_owners {
        set_identity(&mut entities, &positions, child, 33, parent)?;
        relations.push(TemplateRelation::Owner { child, parent });
    }

    let mut per_document_relations = [Vec::new(), Vec::new()];
    for (document_index, data) in traffic_documents.iter().enumerate() {
        append_typed_relations(
            data,
            &references[document_index],
            &mut per_document_relations[document_index],
        )?;
    }
    for category in 0_u8..8 {
        for document in &per_document_relations {
            relations.extend(
                document
                    .iter()
                    .filter(|relation| typed_relation_category(relation) == category)
                    .cloned(),
            );
        }
    }
    let geometry = typed_geometry(&traffic_documents, &references, signalized.spatial(), frame)?;
    Ok(CorridorTemplate {
        entities,
        relations,
        geometry,
    })
}

fn typed_relation_category(relation: &TemplateRelation) -> u8 {
    match relation {
        TemplateRelation::EdgeConnection { .. } => 0,
        TemplateRelation::RouteOccurrence { .. } => 1,
        TemplateRelation::Access { .. } => 2,
        TemplateRelation::SignalGroup { .. } | TemplateRelation::PhaseState { .. } => 3,
        TemplateRelation::Gate { .. } | TemplateRelation::WaitingZone { .. } => 4,
        TemplateRelation::Parking { .. } => 5,
        TemplateRelation::LaneCoverage { .. } => 6,
        TemplateRelation::JunctionInternalEdge { .. } => 7,
        TemplateRelation::Owner { .. } => u8::MAX,
    }
}

fn register_entities(
    data: &InitialTrafficData,
    document_index: usize,
    kind: u16,
    next_local: &mut [u32; 23],
    references: &mut [TypedDocumentRefs; 2],
    entities: &mut Vec<TemplateEntity>,
) -> Result<(), String> {
    if kind == 3 {
        let mut all_lanes = Vec::new();
        for handle in data.cross_section().sections() {
            let section = data
                .cross_section()
                .section(handle)
                .ok_or("missing section")?;
            let mut lanes = Vec::new();
            for _ in section.lanes() {
                lanes.push(register_plain(kind, next_local, entities)?);
            }
            all_lanes.push(lanes);
        }
        references[document_index].lanes = all_lanes;
        return Ok(());
    }
    if kind == 13 {
        let mut all_phases = Vec::new();
        for controller in data.signals().controllers() {
            let mut phases = Vec::new();
            for _ in controller.phases() {
                phases.push(register_plain(kind, next_local, entities)?);
            }
            all_phases.push(phases);
        }
        references[document_index].phases = all_phases;
        return Ok(());
    }
    let ids = typed_ids(data, kind)?;
    for id in ids {
        let reference = register_plain(kind, next_local, entities)?;
        if references[document_index]
            .by_kind
            .entry(kind)
            .or_default()
            .insert(id.clone(), reference)
            .is_some()
        {
            return Err(format!("duplicate production ID kind={kind} id={id}"));
        }
    }
    Ok(())
}

fn typed_ids(data: &InitialTrafficData, kind: u16) -> Result<Vec<String>, String> {
    let ids = match kind {
        1 => data
            .cross_section()
            .corridors()
            .map(|handle| {
                data.cross_section()
                    .corridor(handle)
                    .expect("normalized corridor")
                    .id()
                    .to_owned()
            })
            .collect(),
        2 => data
            .cross_section()
            .sections()
            .map(|handle| {
                data.cross_section()
                    .section(handle)
                    .expect("normalized section")
                    .id()
                    .to_owned()
            })
            .collect(),
        4 => data
            .lane_graph()
            .edges()
            .map(|value| value.id().to_owned())
            .collect(),
        5 => data
            .junctions()
            .junctions()
            .map(|handle| {
                data.junctions()
                    .junction(handle)
                    .expect("normalized junction")
                    .id()
                    .to_owned()
            })
            .collect(),
        6 => data
            .junctions()
            .movements()
            .map(|handle| {
                data.junctions()
                    .movement(handle)
                    .expect("normalized movement")
                    .id()
                    .to_owned()
            })
            .collect(),
        7 => data
            .junctions()
            .maneuver_paths()
            .map(|handle| {
                data.junctions()
                    .maneuver_path(handle)
                    .expect("normalized path")
                    .id()
                    .to_owned()
            })
            .collect(),
        8 => data
            .signals()
            .maneuver_gates()
            .map(|handle| {
                data.signals()
                    .maneuver_gate(handle)
                    .expect("normalized gate")
                    .id()
                    .to_owned()
            })
            .collect(),
        9 => data
            .waiting()
            .waiting_zones()
            .map(|handle| {
                data.waiting()
                    .waiting_zone(handle)
                    .expect("normalized waiting zone")
                    .id()
                    .to_owned()
            })
            .collect(),
        10 => data
            .signals()
            .stop_lines()
            .map(|value| value.id().to_owned())
            .collect(),
        11 => data
            .signals()
            .groups()
            .map(|value| value.id().to_owned())
            .collect(),
        12 => data
            .signals()
            .controllers()
            .map(|value| value.id().to_owned())
            .collect(),
        14 => data
            .parking()
            .areas()
            .map(|value| value.id().to_owned())
            .collect(),
        15 => data
            .parking()
            .spaces()
            .map(|value| value.id().to_owned())
            .collect(),
        16 => data
            .cross_section()
            .groups()
            .map(|handle| {
                data.cross_section()
                    .group(handle)
                    .expect("normalized lane group")
                    .id()
                    .to_owned()
            })
            .collect(),
        17 => data
            .cross_section()
            .bands()
            .map(|handle| {
                data.cross_section()
                    .band(handle)
                    .expect("normalized facility band")
                    .id()
                    .to_owned()
            })
            .collect(),
        18 => data
            .participant_classes()
            .classes()
            .map(|handle| {
                data.participant_classes()
                    .class(handle)
                    .expect("normalized participant class")
                    .id()
                    .to_owned()
            })
            .collect(),
        19 => data
            .access()
            .rules()
            .map(|handle| {
                data.access()
                    .rule(handle)
                    .expect("normalized access rule")
                    .id()
                    .to_owned()
            })
            .collect(),
        20 => data
            .vehicle_profiles()
            .profiles()
            .map(|value| value.external_id().to_owned())
            .collect(),
        21 => data.routes().map(|value| value.id().to_owned()).collect(),
        _ => return Err(format!("unsupported typed kind {kind}")),
    };
    Ok(ids)
}

fn register_plain(
    kind: u16,
    next_local: &mut [u32; 23],
    entities: &mut Vec<TemplateEntity>,
) -> Result<EntityRef, String> {
    let local = next_local[usize::from(kind)];
    next_local[usize::from(kind)] = local
        .checked_add(1)
        .ok_or_else(|| "typed local ordinal overflow".to_owned())?;
    let reference = EntityRef { kind, local };
    entities.push(TemplateEntity {
        reference,
        identity_references: BTreeMap::new(),
    });
    Ok(reference)
}

fn set_identity(
    entities: &mut [TemplateEntity],
    positions: &BTreeMap<EntityRef, usize>,
    child: EntityRef,
    tag: u16,
    parent: EntityRef,
) -> Result<(), String> {
    let entity = entities
        .get_mut(*positions.get(&child).ok_or("unknown typed child")?)
        .ok_or("unknown typed entity position")?;
    if entity.identity_references.insert(tag, parent).is_some() {
        return Err("duplicate typed identity reference".to_owned());
    }
    Ok(())
}

fn append_typed_relations(
    data: &InitialTrafficData,
    refs: &TypedDocumentRefs,
    relations: &mut Vec<TemplateRelation>,
) -> Result<(), String> {
    for edge in data.lane_graph().edges() {
        let source = refs.named(4, edge.id())?;
        for target in edge.next_edge_ids() {
            relations.push(TemplateRelation::EdgeConnection {
                source,
                target: refs.named(4, target)?,
            });
        }
    }
    for route in data.routes() {
        for (index, edge) in route.edge_ids().iter().enumerate() {
            relations.push(TemplateRelation::RouteOccurrence {
                route: refs.named(21, route.id())?,
                index: u32::try_from(index).map_err(|_| "route index overflow")?,
                edge: refs.named(4, edge)?,
            });
        }
    }
    for handle in data.access().rules() {
        let rule = data.access().rule(handle).ok_or("missing access rule")?;
        let (target_kind, target_id) = match rule.target() {
            AccessTargetId::RoadSection(id) => (2, id.as_str()),
            AccessTargetId::LaneEdge(id) => (4, id.as_str()),
            AccessTargetId::ManeuverPath(id) => (7, id.as_str()),
            AccessTargetId::LaneGroup(id) => (16, id.as_str()),
            AccessTargetId::FacilityBand(id) => (17, id.as_str()),
        };
        for participant in rule.participant_class_ids() {
            relations.push(TemplateRelation::Access {
                rule: refs.named(19, rule.id())?,
                participant: refs.named(18, participant)?,
                target: refs.named(target_kind, target_id)?,
                decision: match rule.effect() {
                    AccessEffect::Deny => 0,
                    AccessEffect::Allow => 1,
                },
            });
        }
    }
    for handle in data.signals().maneuver_gates() {
        let gate = data.signals().maneuver_gate(handle).ok_or("missing gate")?;
        if let SignalControlInput::Group(group) = gate.signal_control() {
            relations.push(TemplateRelation::SignalGroup {
                group: refs.named(11, group)?,
                gate: refs.named(8, gate.id())?,
            });
        }
    }
    for (controller_index, controller) in data.signals().controllers().enumerate() {
        for (phase_index, phase) in controller.phases().iter().enumerate() {
            let phase_ref = *refs
                .phases
                .get(controller_index)
                .and_then(|phases| phases.get(phase_index))
                .ok_or("missing phase ref")?;
            for state in phase.states() {
                relations.push(TemplateRelation::PhaseState {
                    phase: phase_ref,
                    group: refs.named(11, state.group_id())?,
                    state: match state.aspect() {
                        SignalAspect::Red => 0,
                        SignalAspect::Yellow => 1,
                        SignalAspect::Green => 2,
                    },
                });
            }
        }
    }
    for handle in data.signals().maneuver_gates() {
        let gate = data.signals().maneuver_gate(handle).ok_or("missing gate")?;
        let path_handle = data
            .junctions()
            .maneuver_path_handle(gate.maneuver_path_id())
            .ok_or("missing gate path")?;
        let path = data
            .junctions()
            .maneuver_path(path_handle)
            .ok_or("missing gate path definition")?;
        let edge_id = typed_path_edge(path, gate.transition_index())?;
        relations.push(TemplateRelation::Gate {
            path: refs.named(7, gate.maneuver_path_id())?,
            transition_index: gate.transition_index(),
            gate: refs.named(8, gate.id())?,
            stop_line: refs.named(10, gate.stop_line_id())?,
            edge: refs.named(4, edge_id)?,
            edge_position_bits: 1.0_f32.to_bits(),
        });
    }
    for handle in data.waiting().waiting_zones() {
        let zone = data
            .waiting()
            .waiting_zone(handle)
            .ok_or("missing waiting zone")?;
        let entry = data
            .signals()
            .maneuver_gate(
                data.signals()
                    .maneuver_gate_handle(zone.entry_gate_id())
                    .ok_or("missing entry gate")?,
            )
            .ok_or("missing entry gate definition")?;
        let release = data
            .signals()
            .maneuver_gate(
                data.signals()
                    .maneuver_gate_handle(zone.release_gate_id())
                    .ok_or("missing release gate")?,
            )
            .ok_or("missing release gate definition")?;
        relations.push(TemplateRelation::WaitingZone {
            path: refs.named(7, zone.maneuver_path_id())?,
            entry_transition_index: entry.transition_index(),
            release_transition_index: release.transition_index(),
            zone: refs.named(9, zone.id())?,
            before_gate: refs.named(8, zone.entry_gate_id())?,
            after_gate: refs.named(8, zone.release_gate_id())?,
            capacity: zone.max_occupancy(),
        });
    }
    for space in data.parking().spaces() {
        let (entry_high_bits, entry_residual_bits) = progress_bits(space.entry_progress())?;
        let (exit_high_bits, exit_residual_bits) = progress_bits(space.exit_progress())?;
        relations.push(TemplateRelation::Parking {
            space: refs.named(15, space.id())?,
            entry_edge: refs.named(4, space.entry_edge_id())?,
            entry_high_bits,
            entry_residual_bits,
            exit_edge: refs.named(4, space.exit_edge_id())?,
            exit_high_bits,
            exit_residual_bits,
        });
    }
    for (section_index, handle) in data.cross_section().sections().enumerate() {
        let section = data
            .cross_section()
            .section(handle)
            .ok_or("missing section")?;
        for (lane_index, lane) in section.lanes().iter().enumerate() {
            let lane_ref = *refs
                .lanes
                .get(section_index)
                .and_then(|lanes| lanes.get(lane_index))
                .ok_or("missing lane ref")?;
            for (index, edge) in lane.edge_ids().iter().enumerate() {
                relations.push(TemplateRelation::LaneCoverage {
                    lane: lane_ref,
                    index: u32::try_from(index).map_err(|_| "lane occurrence overflow")?,
                    edge: refs.named(4, edge)?,
                });
            }
        }
    }
    for handle in data.junctions().maneuver_paths() {
        let path = data
            .junctions()
            .maneuver_path(handle)
            .ok_or("missing path")?;
        let movement = data
            .junctions()
            .movement(
                data.junctions()
                    .movement_handle(path.movement_id())
                    .ok_or("missing movement")?,
            )
            .ok_or("missing movement definition")?;
        for edge in path.internal_edge_ids() {
            relations.push(TemplateRelation::JunctionInternalEdge {
                junction: refs.named(5, movement.junction_id())?,
                edge: refs.named(4, edge)?,
            });
        }
    }
    Ok(())
}

fn typed_path_edge(path: &laneflow_core::ManeuverPath, transition: u32) -> Result<&str, String> {
    if transition == 0 {
        return Ok(path.entry_edge_id());
    }
    let index = usize::try_from(transition - 1).map_err(|_| "transition index overflow")?;
    if let Some(edge) = path.internal_edge_ids().get(index) {
        return Ok(edge);
    }
    if index == path.internal_edge_ids().len() {
        return Ok(path.exit_edge_id());
    }
    Err("gate transition index outside path".to_owned())
}

fn progress_bits(value: f64) -> Result<(u32, u32), String> {
    let high = value as f32;
    if !value.is_finite() || !high.is_finite() {
        return Err("parking progress is not finite f32".to_owned());
    }
    let residual = (value - f64::from(high)) as f32;
    Ok((high.to_bits(), residual.to_bits()))
}

fn typed_geometry(
    documents: &[&InitialTrafficData; 2],
    refs: &[TypedDocumentRefs; 2],
    spatial: &laneflow_data::LoadedSpatialPackage,
    frame: EntityRef,
) -> Result<Vec<TemplateGeometry>, String> {
    let mut geometry = Vec::new();
    let signal_graph = documents[0].lane_graph();
    for edge in spatial.edges() {
        let edge_id = signal_graph
            .edge_external_id(edge.edge())
            .ok_or("spatial edge handle not in traffic graph")?;
        let edge_ref = refs[0].named(4, edge_id)?;
        for (point_index, point) in edge.points().iter().enumerate() {
            geometry.push(TemplateGeometry {
                edge: edge_ref,
                frame,
                point_index: u32::try_from(point_index)
                    .map_err(|_| "geometry point index overflow")?,
                x_bits: point.x().to_bits(),
                y_bits: point.y().to_bits(),
                z_bits: point.z().to_bits(),
                coordinate_rule: TemplateGeometryRule::Fixed,
            });
        }
    }
    for edge in documents[1].lane_graph().edges() {
        let edge_ref = refs[1].named(4, edge.id())?;
        let x = edge_ref.local as f32;
        for (point_index, x) in [x, x + 1.0].into_iter().enumerate() {
            geometry.push(TemplateGeometry {
                edge: edge_ref,
                frame,
                point_index: u32::try_from(point_index)
                    .map_err(|_| "synthetic point index overflow")?,
                x_bits: x.to_bits(),
                y_bits: 0.0_f32.to_bits(),
                z_bits: 0.0_f32.to_bits(),
                coordinate_rule: TemplateGeometryRule::Fixed,
            });
        }
    }
    Ok(geometry)
}
