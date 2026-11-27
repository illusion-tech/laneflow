//! L1 在完整横断面的两条外侧车道上发起交接，共用中道和下游平面交叉路口。
//! 背景车道与停车位独立编制，W 族继续使用自己的发射器。

use laneflow_compiler::*;
use laneflow_format::{FormatLimits, check_post_emission_bundle};

use crate::encoding::{config_bytes, digest};
use crate::{BUILD_ID, Error, Identity, NAMESPACE, compile_error, key};

/// 两个换道者从不同源边进入同一目标，随后各按普通机动通过路口。
pub(crate) const LC_ROUTES: [[&str; 4]; 2] = [
    ["lc-left", "lc-center", "lc-int", "lc-exit"],
    ["lc-right", "lc-center", "lc-int", "lc-exit"],
];
pub(crate) const LC_EDIT_NAMESPACE: &str = "lf-synth-road";
const LC_FRAME: &str = "lc-frame";
const CLUSTER_SPACING_M: f64 = 200.;

pub(crate) fn compile(identity: &Identity) -> Result<(Vec<u8>, Vec<String>, u32), Error> {
    let limits = CompileProfile::single_network_1m_stable_entities();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: NAMESPACE,
            source_document_key: "lf-synth-lane-change.document",
            generator_build_id: BUILD_ID,
            parameters_and_inputs_digest: digest(&config_bytes(identity)?),
            frontend_options_digest: digest(b"lf-synth-frontend-1"),
            random_seed: Some(0),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .map_err(|e| compile_error("source header", e))?;
    let mut builder = SyntheticModuleBuilder::new(header, &limits)
        .map_err(|e| compile_error("synthetic module", e))?;
    builder
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "car",
            extends: None,
        })
        .map_err(|e| compile_error("participant class", e))?;
    builder
        .add_vehicle_profile(VehicleProfileInput {
            geometry: crate::PASSENGER_CAR_GEOMETRY,
            vehicle_profile_key: "car",
            participant_class: ParticipantClassReference::local("car"),
            iidm: IidmVehicleProfileInput {
                length_meters: 4.5,
                desired_speed_meters_per_second: 15.0,
                min_gap_meters: 2.5,
                time_headway_seconds: 1.5,
                max_acceleration_meters_per_second_squared: 1.5,
                comfortable_deceleration_meters_per_second_squared: 4.5,
                emergency_deceleration_meters_per_second_squared: 9.0,
            },
        })
        .map_err(|e| compile_error("vehicle profile", e))?;
    let mut geometry = Vec::new();
    for cell in 0..identity.cells() {
        let facility = key(cell, 0, "facility");
        builder
            .add_parking_facility(ParkingFacilityInput {
                parking_facility_key: &facility,
                virtual_capacity: 0,
                virtual_entries: &[],
                virtual_exits: &[],
            })
            .map_err(|e| compile_error("parking facility", e))?;
        // 背景使用普通车道和停车位，下游路口的门和流统一由编辑侧策略覆盖。
        add_background_route(&mut builder, cell, &facility)?;
        for route in 0..4 {
            let [entry, exit] = crate::network::centerlines(cell, route);
            geometry.push((key(cell, route, "entry"), entry));
            geometry.push((key(cell, route, "exit"), exit));
        }
    }
    let clusters = identity
        .lane_change()
        .ok_or_else(|| Error::Invalid("L1 缺少换道簇".into()))?
        .clusters;
    let geometry_refs: Vec<_> = geometry
        .iter()
        .map(|(name, points)| LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local(name),
            centerline_points: points,
        })
        .collect();
    builder
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "world",
            lane_edge_geometries: &geometry_refs,
        })
        .map_err(|e| compile_error("canonical frame", e))?;
    let mut unit = CompilationUnitBuilder::new(limits.clone());
    unit.add_synthetic_module(
        builder
            .finish()
            .map_err(|e| compile_error("finish module", e))?,
    )
    .map_err(|e| compile_error("compilation unit", e))?;
    let editing_bytes = lane_change_editing(&limits, clusters)?;
    unit.add_road_editing_module(
        laneflow_compiler::road_editing::RoadEditingModuleInput::try_new(
            "lf-synth-road.document",
            &editing_bytes,
            None,
        )
        .map_err(|e| compile_error("editing module input", e))?,
    )
    .map_err(|e| compile_error("editing unit", e))?;
    let output = Compiler::new()
        .compile(unit.build().map_err(|e| compile_error("build unit", e))?)
        .map_err(|e| compile_error("compile", e))?;
    let candidate = emit_portable_candidate(
        &output,
        &PortableEmissionProvenance::try_new(BUILD_ID)
            .map_err(|e| compile_error("provenance", e))?,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .map_err(|e| compile_error("emit", e))?;
    check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .map_err(|e| compile_error("post-emission", e))?;
    let sections = (0..clusters)
        .map(|cluster| {
            let key = cluster_key("lc-road-section", cluster);
            output
                .lir()
                .road_sections()
                .find(|s| {
                    s.identity_fields().any(|f| {
                        f.tag() == laneflow_static_contract::FieldTag::SectionKey
                            && f.value_bytes() == key.as_bytes()
                    })
                })
                .map(|s| s.stable_id().to_string())
                .ok_or_else(|| Error::Invalid(format!("缺少换道区段 {key}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let points = output
        .lir()
        .canonical_network_usage()
        .get(laneflow_static_contract::CanonicalNetworkDimension::GeometryPointCount);
    let points = u32::try_from(points).map_err(|e| compile_error("geometry point count", e))?;
    Ok((
        candidate.canonical_artifact().bytes().to_vec(),
        sections,
        points,
    ))
}

/// L1 背景路线（#1218 D1）：每 cell 四条路线，仅车道边 + 25 停车位，无机动/门/信号。
fn add_background_route(
    b: &mut SyntheticModuleBuilder,
    c: u32,
    facility: &str,
) -> Result<(), Error> {
    for route in 0..4 {
        let entry = key(c, route, "entry");
        let exit = key(c, route, "exit");
        b.add_lane_edge(LaneEdgeInput {
            lane_edge_key: &entry,
            length_meters: 5_130.0,
            speed_limit_meters_per_second: 13.9,
            successors: &[LaneEdgeReference::local(&exit)],
        })
        .map_err(|e| compile_error("background entry edge", e))?;
        b.add_lane_edge(LaneEdgeInput {
            lane_edge_key: &exit,
            length_meters: 5_130.0,
            speed_limit_meters_per_second: 13.9,
            successors: &[],
        })
        .map_err(|e| compile_error("background exit edge", e))?;
        for s in 0..25 {
            let anchor = || ParkingLaneAnchorInput {
                lane_edge: LaneEdgeReference::local(&exit),
                progress_meters: 5.0 + 7.5 * f64::from(s),
            };
            b.add_parking_space(ParkingSpaceInput {
                parking_space_key: &key(c, route, &format!("space{s:02}")),
                parking_facility: Some(ParkingFacilityReference::local(facility)),
                entry: anchor(),
                exit: anchor(),
                geometry: ParkingSpaceGeometryInput {
                    lateral_offset_meters: 3.0,
                    heading_offset_radians: 0.0,
                    length_meters: 5.0,
                    width_meters: 2.4,
                },
            })
            .map_err(|e| compile_error("background parking space", e))?;
        }
    }
    Ok(())
}

/// 换道簇边键（参数化，#1233）：簇 `cluster` 的边键带确定性后缀，避免跨簇冲突。
pub(crate) fn cluster_key(base: &str, cluster: u32) -> String {
    format!("{base}-{cluster}")
}

/// 各走廊沿参考车道中心线编制。相邻的两个车道成员以各自的共同边界授予跨越许可。
fn add_road(
    builder: &mut road_editing::RoadEditingSourceModuleBuilder<'_>,
    cluster: u32,
    name: &str,
    geometry: road_editing::RoadEditingCurveProgram,
    lanes: &[(&str, f64, Option<&str>)],
) -> Result<(), DiagnosticBundle> {
    use e::RoadEditingDeclaration as D;
    use road_editing as e;
    let key = cluster_key(name, cluster);
    let section_key = cluster_key(&format!("{name}-section"), cluster);
    let section = e::RoadSectionReference::owner_scoped(vec![key.clone()], &section_key)?;
    let corridor = e::RoadCorridorReference::local(&key)?;
    let references: Vec<_> = lanes
        .iter()
        .map(|(base, _, _)| {
            e::AuthoringLaneReference::owner_scoped(vec![key.clone(), section_key.clone()], *base)
        })
        .collect::<Result<_, _>>()?;
    builder.add_alignment(e::RoadAlignmentInput::try_new(
        &key,
        e::CanonicalFrameReference::local(LC_FRAME)?,
        geometry,
    )?)?;
    builder.add_declaration(D::RoadCorridor(
        e::RoadCorridorInput::try_new(
            &key,
            e::RoadAlignmentReference::try_new(&key)?,
            0.,
            e::RoadEditingStationEnd::AlignmentEnd,
            section.clone(),
            references[lanes.len() / 2].clone(),
            vec![e::RoadEditingCorridorElement::RoadSection(section.clone())],
        )?
        .with_cross_section(e::CrossSectionCoverage::Complete, vec![]),
    ))?;
    let boundaries = references
        .windows(2)
        .map(|pair| e::BoundaryExtentInput {
            left: e::BoundaryMemberInput::AuthoringLane(pair[0].clone()),
            right: e::BoundaryMemberInput::AuthoringLane(pair[1].clone()),
            kind: e::BoundaryExtentKind::Rule,
            local_key: "mark".into(),
            low: e::BoundaryStation::CorridorEndpoint,
            high: e::BoundaryStation::CorridorEndpoint,
            payload: Some(e::BoundaryPayloadInput::CrossingRule {
                directions: 3,
                effect: 1,
            }),
            canvas_selection: None,
        })
        .collect();
    builder.add_declaration(D::RoadSection(
        e::RoadSectionInput::try_new(section_key, "motorLane", references, corridor)?
            .with_boundary_extents(boundaries),
    ))?;
    for &(base, speed, successor) in lanes {
        let edge_key = cluster_key(base, cluster);
        let successors = successor
            .map(|s| e::LaneEdgeReference::local(cluster_key(s, cluster)))
            .transpose()?
            .into_iter()
            .collect();
        builder.add_declaration(D::LaneEdge(e::LaneEdgeInput::try_new(
            &edge_key, speed, successors, None,
        )?))?;
        builder.add_declaration(D::AuthoringLane(
            e::AuthoringLaneInput::try_new(
                base,
                e::LaneEdgeReference::local(edge_key)?,
                e::RoadEditingLaneDirection::Forward,
                e::LinearWidthProfile::try_new(3.5, 3.5)?,
                None,
                section.clone(),
            )?
            .with_wheel_support(Some(true)),
        ))?;
    }
    Ok(())
}

/// 三条主道同时在共同截面开始。普通后继只用于两股直行交通通过下游路口。
fn add_cluster_roads(
    b: &mut road_editing::RoadEditingSourceModuleBuilder<'_>,
    cluster: u32,
) -> Result<(), DiagnosticBundle> {
    use road_editing::{
        RoadEditingCurveProgram as Curve, RoadEditingCurveSegment as Segment, RoadEditingPoint3,
    };
    let x = CLUSTER_SPACING_M * f64::from(cluster);
    let [left_speed, right_speed] = crate::HANDOFF_SPEED_MM_S.map(|mm_s| f64::from(mm_s) / 1000.);
    let point = |lateral, z| RoadEditingPoint3::try_new(x + lateral, 0., z);
    let straight = |[x0, z0]: [f64; 2], [x1, z1]: [f64; 2]| {
        Curve::try_new(point(x0, z0)?, vec![Segment::line(point(x1, z1)?)])
    };
    add_road(
        b,
        cluster,
        "lc-road",
        straight([0., 0.], [0., 120.])?,
        &[
            ("lc-left", left_speed, None),
            ("lc-center", 8., Some("lc-int")),
            ("lc-right", right_speed, None),
        ],
    )?;
    add_road(
        b,
        cluster,
        "lc-aux-road",
        straight([-30., 126.], [-4., 126.])?,
        &[("lc-aux", 8., Some("lc-aux-int"))],
    )?;
    for (edge, start, end, exit) in [
        ("lc-int", [0., 120.], [0., 132.], "lc-exit"),
        ("lc-aux-int", [-4., 126.], [4., 126.], "lc-aux-exit"),
    ] {
        b.add_declaration(road_editing::RoadEditingDeclaration::LaneEdge(
            road_editing::LaneEdgeInput::try_new(
                cluster_key(edge, cluster),
                8.,
                vec![road_editing::LaneEdgeReference::local(cluster_key(
                    exit, cluster,
                ))?],
                Some(straight(start, end)?),
            )?,
        ))?;
    }
    add_road(
        b,
        cluster,
        "lc-aux-exit-road",
        straight([4., 126.], [30., 126.])?,
        &[("lc-aux-exit", 8., None)],
    )?;
    add_road(
        b,
        cluster,
        "lc-exit-road",
        straight([0., 132.], [0., 632.])?,
        &[("lc-exit", 8., None)],
    )?;
    Ok(())
}

fn lane_change_editing(limits: &CompileProfile, clusters: u32) -> Result<Vec<u8>, Error> {
    use laneflow_compiler::road_editing as editing;

    let header = editing::RoadEditingModuleHeader::try_new(
        LC_EDIT_NAMESPACE,
        "lf-synth-road.document",
        vec![],
        editing::RoadEditingProvenance::direct("lf-synth dynamic handoff")
            .map_err(|e| compile_error("editing provenance", e))?,
    )
    .map_err(|e| compile_error("editing header", e))?;
    let mut builder = editing::RoadEditingSourceModuleBuilder::new(
        header,
        GeometryAccuracyProfile::Balanced5Cm,
        GeometryDirectionProfile::Balanced2Deg,
        limits,
    )
    .map_err(|e| compile_error("editing module", e))?;
    builder
        .add_declaration(editing::RoadEditingDeclaration::CanonicalFrame(
            editing::CanonicalFrameInput::try_new(LC_FRAME)
                .map_err(|e| compile_error("cluster frame", e))?,
        ))
        .map_err(|e| compile_error("frame declaration", e))?;
    let edge = |k: &str| {
        editing::LaneEdgeReference::local(k).map_err(|e| compile_error("cluster edge", e))
    };
    let mut gate_rules = Vec::new();
    let mut stream_rules = Vec::new();
    for cluster in 0..clusters {
        add_cluster_roads(&mut builder, cluster)
            .map_err(|e| compile_error("lane change roads", e))?;
        let junction_key = cluster_key("lc-junction", cluster);
        let zone_key = cluster_key("lc-zone", cluster);
        let junction = editing::JunctionReference::local(junction_key.clone())
            .map_err(|e| compile_error("junction ref", e))?;
        let zone = editing::ConflictZoneReference::owner_scoped(
            vec![junction_key.clone()],
            zone_key.clone(),
        )
        .map_err(|e| compile_error("zone ref", e))?;
        builder
            .add_declaration(editing::RoadEditingDeclaration::Junction(
                editing::JunctionInput::try_new(
                    junction_key.clone(),
                    vec![
                        edge(cluster_key("lc-center", cluster).as_str())?,
                        edge(cluster_key("lc-aux", cluster).as_str())?,
                        edge(cluster_key("lc-exit", cluster).as_str())?,
                        edge(cluster_key("lc-aux-exit", cluster).as_str())?,
                    ],
                    vec![
                        edge(cluster_key("lc-int", cluster).as_str())?,
                        edge(cluster_key("lc-aux-int", cluster).as_str())?,
                    ],
                )
                .map_err(|e| compile_error("junction", e))?,
            ))
            .map_err(|e| compile_error("junction declaration", e))?;
        builder
            .add_declaration(editing::RoadEditingDeclaration::ConflictZone(
                editing::ConflictZoneInput::try_new(zone_key.clone(), junction.clone())
                    .map_err(|e| compile_error("zone", e))?,
            ))
            .map_err(|e| compile_error("zone declaration", e))?;
        for (
            entry_base,
            int_base,
            exit_base,
            movement_key,
            path_key,
            stop_key,
            gate_key,
            stream_key,
        ) in [
            (
                "lc-center",
                "lc-int",
                "lc-exit",
                "lc-movement",
                "lc-path",
                "lc-stop",
                "lc-admission",
                "lc-stream",
            ),
            (
                "lc-aux",
                "lc-aux-int",
                "lc-aux-exit",
                "lc-movement-2",
                "lc-path-2",
                "lc-stop-2",
                "lc-admission-2",
                "lc-stream-2",
            ),
        ] {
            let entry_edge = cluster_key(entry_base, cluster);
            let exit_edge = cluster_key(exit_base, cluster);
            let int_edge = cluster_key(int_base, cluster);
            let movement_key = cluster_key(movement_key, cluster);
            let path_key = cluster_key(path_key, cluster);
            let stop_key = cluster_key(stop_key, cluster);
            let gate_key = cluster_key(gate_key, cluster);
            let stream_key = cluster_key(stream_key, cluster);
            let movement = editing::MovementReference::owner_scoped(
                vec![junction_key.clone()],
                movement_key.clone(),
            )
            .map_err(|e| compile_error("movement ref", e))?;
            let path = editing::ManeuverPathReference::owner_scoped(
                vec![junction_key.clone(), movement_key.clone()],
                path_key.clone(),
            )
            .map_err(|e| compile_error("path ref", e))?;
            let admission_gate = editing::ManeuverGateReference::owner_scoped(
                vec![junction_key.clone(), movement_key.clone(), path_key.clone()],
                gate_key.clone(),
            )
            .map_err(|e| compile_error("gate ref", e))?;
            builder
                .add_declaration(editing::RoadEditingDeclaration::Movement(
                    editing::MovementInput::try_new(
                        movement_key.clone(),
                        junction.clone(),
                        entry_edge.clone(),
                        exit_edge.clone(),
                    )
                    .map_err(|e| compile_error("movement", e))?,
                ))
                .map_err(|e| compile_error("movement declaration", e))?;
            builder
                .add_declaration(editing::RoadEditingDeclaration::ManeuverPath(
                    editing::ManeuverPathInput::try_new(
                        path_key.clone(),
                        movement.clone(),
                        edge(entry_edge.as_str())?,
                        vec![edge(int_edge.as_str())?],
                        edge(exit_edge.as_str())?,
                    )
                    .map_err(|e| compile_error("path", e))?,
                ))
                .map_err(|e| compile_error("path declaration", e))?;
            builder
                .add_declaration(editing::RoadEditingDeclaration::StopLine(
                    editing::StopLineInput::try_new(stop_key.clone(), edge(entry_edge.as_str())?)
                        .map_err(|e| compile_error("stop line", e))?,
                ))
                .map_err(|e| compile_error("stop line declaration", e))?;
            builder
                .add_declaration(editing::RoadEditingDeclaration::ManeuverGate(
                    editing::ManeuverGateInput::try_new(
                        gate_key.clone(),
                        path.clone(),
                        0,
                        editing::StopLineReference::local(stop_key.clone())
                            .map_err(|e| compile_error("stop ref", e))?,
                        editing::RoadEditingSignalControl::None,
                    )
                    .map_err(|e| compile_error("gate", e))?,
                ))
                .map_err(|e| compile_error("gate declaration", e))?;
            builder
                .add_declaration(editing::RoadEditingDeclaration::ParticipantStream(
                    editing::ParticipantStreamInput::try_new(
                        stream_key.clone(),
                        junction.clone(),
                        path.clone(),
                        vec![editing::ConflictPassageInput::new(
                            zone.clone(),
                            editing::PathAnchorInput::gate(admission_gate.clone()),
                            editing::PathAnchorInput::edge_boundary(2),
                        )],
                    )
                    .map_err(|e| compile_error("stream", e))?,
                ))
                .map_err(|e| compile_error("stream declaration", e))?;
            gate_rules.push(
                editing::PolicyGateRuleInput::try_new(
                    gate_key.clone(),
                    admission_gate,
                    None,
                    GateInterpretation::Uncontrolled,
                    GateProhibition::None,
                    vec![],
                )
                .map_err(|e| compile_error("gate rule", e))?,
            );
            stream_rules.push(
                editing::PolicyStreamRuleInput::try_new(
                    stream_key.clone(),
                    editing::ParticipantStreamReference::owner_scoped(
                        vec![junction_key.clone()],
                        stream_key.clone(),
                    )
                    .map_err(|e| compile_error("stream ref", e))?,
                    None,
                    0,
                    vec![],
                    None,
                    vec![],
                )
                .map_err(|e| compile_error("stream rule", e))?,
            );
        }
        // 冲突区覆盖横纵两条路口内部边的实际交叉位置。
        let offset = CLUSTER_SPACING_M * f64::from(cluster);
        let region_points = vec![
            editing::RoadEditingPoint2::try_new(offset - 2.0, 124.0)
                .map_err(|e| compile_error("region point", e))?,
            editing::RoadEditingPoint2::try_new(offset + 2.0, 124.0)
                .map_err(|e| compile_error("region point", e))?,
            editing::RoadEditingPoint2::try_new(offset + 2.0, 128.0)
                .map_err(|e| compile_error("region point", e))?,
            editing::RoadEditingPoint2::try_new(offset - 2.0, 128.0)
                .map_err(|e| compile_error("region point", e))?,
        ];
        builder
            .add_conflict_zone_region(
                editing::ConflictZoneRegionInput::try_new(
                    zone.clone(),
                    editing::CanonicalFrameReference::local(LC_FRAME)
                        .map_err(|e| compile_error("frame ref", e))?,
                    -1.0,
                    1.0,
                    region_points,
                )
                .map_err(|e| compile_error("conflict region", e))?,
            )
            .map_err(|e| compile_error("conflict region declaration", e))?;
    }
    builder
        .add_declaration(editing::RoadEditingDeclaration::RightOfWayPolicySet(
            editing::RightOfWayPolicySetInput::try_new(
                "policy",
                RegulationIdentity::try_new("engineering", "lf-synth-v1")
                    .map_err(|e| compile_error("regulation", e))?
                    .with_source("repository:laneflow")
                    .map_err(|e| compile_error("regulation source", e))?,
                vec![],
                vec![
                    editing::PolicyGapProfileInput::try_new(
                        "calibration-gap",
                        "lf-synth-v1",
                        500,
                        500,
                        100,
                    )
                    .map_err(|e| compile_error("gap profile", e))?,
                ],
                stream_rules,
                gate_rules,
            )
            .map_err(|e| compile_error("policy set", e))?,
        ))
        .map_err(|e| compile_error("policy declaration", e))?;
    let written = editing::RoadEditingSourceWriter::new(limits)
        .write(
            builder
                .finish()
                .map_err(|e| compile_error("editing finish", e))?,
        )
        .map_err(|e| compile_error("editing write", e))?;
    Ok(written.as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Case, LaneChangeParams};
    use laneflow_format::check_canonical_network_input;
    use laneflow_static_contract::{EntityKind, LaneEdgeId, RoadSectionId};
    use laneflow_static_network::{
        BoundaryOwner, SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision, vehicle::HandoffWindow,
    };

    #[test]
    fn emitted_clusters_bind_both_outer_lanes_to_the_same_target_without_an_ordinary_hop() {
        let clusters = 64;
        let mut identity = Identity::new(
            10_000,
            Case::LaneChange(LaneChangeParams {
                clusters,
                changers_per_cluster: 2,
            }),
            16,
        )
        .unwrap();
        identity.scale = 100;
        let (lfca, sections, _) = compile(&identity).unwrap();
        let network = build_shared_network_revision(
            check_canonical_network_input(lfca.as_slice(), FormatLimits::HARD).unwrap(),
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::Omit,
                SharedNetworkBuildLimits::new(64 * 1024 * 1024, 64 * 1024 * 1024),
            ),
        )
        .unwrap();
        let traffic = network.traffic();
        assert_eq!(sections.len(), clusters as usize);
        assert_eq!(
            sections
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            sections.len()
        );
        for (cluster, section) in sections.iter().enumerate() {
            let owner = network
                .identity()
                .ordinal(section.parse::<RoadSectionId>().unwrap())
                .unwrap();
            let edge = |base: &str| {
                let id = derive_canonical_stable_id_v1(
                    EntityKind::LaneEdge,
                    LC_EDIT_NAMESPACE,
                    &cluster_key(base, cluster as u32),
                )
                .unwrap();
                network
                    .identity()
                    .ordinal(LaneEdgeId::from_untyped(id))
                    .unwrap()
            };
            let center = edge("lc-center");
            for (left, (base, speed)) in [("lc-left", 3000), ("lc-right", 1500)]
                .into_iter()
                .enumerate()
            {
                let source = edge(base);
                assert!(traffic.successors(source).unwrap().is_empty());
                assert_eq!(
                    traffic.lane_speed_limits_millimetres_per_second()[source.index()],
                    speed
                );
                let window = HandoffWindow {
                    edges: [source, center],
                    owner: BoundaryOwner::Section(owner),
                    left_index: left as u32,
                    stations_mm: [20_000, 110_000],
                };
                let binding = network
                    .bind_handoff_window(network.network_revision(), window)
                    .unwrap();
                assert_eq!(
                    network
                        .station_mapping()
                        .corridor_domain(binding.corridor())
                        .unwrap()
                        .length_mm(),
                    120_000
                );
            }
            assert_eq!(
                traffic.successors(center),
                Some([edge("lc-int")].as_slice())
            );
            assert_eq!(
                traffic.successors(edge("lc-aux")),
                Some([edge("lc-aux-int")].as_slice())
            );
            assert_eq!(
                traffic.successors(edge("lc-int")),
                Some([edge("lc-exit")].as_slice())
            );
        }
    }
}
