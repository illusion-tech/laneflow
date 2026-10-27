//! 直连声明与含内边路径共存时的生产发射回归。

use super::*;
use crate::output::model::{Centerline, SpatialEdge};

fn mixed_paths(join_direct: bool) -> (TrafficPackage, SpatialPackage, HashSet<String>) {
    let observation = small_observation();
    let mut traffic = observation.traffic.as_ref().unwrap().clone();
    let path = traffic
        .maneuver_paths
        .iter_mut()
        .find(|p| p.exit_edge_id == "sumo:east_0")
        .unwrap();
    let removed = path.internal_edge_ids.pop().unwrap();
    assert!(path.internal_edge_ids.is_empty());
    traffic.lane_graph.edges.retain(|e| e.id != removed);
    let spatial = SpatialPackage {
        format_version: "0.1",
        frame_id: LUST_FRAME_ID.to_owned(),
        edges: observation
            .edges
            .iter()
            .zip(&observation.inputs)
            .filter(|((id, _), _)| *id != removed)
            .map(|((id, _), input)| SpatialEdge {
                traffic_edge_id: id.clone(),
                centerline: Centerline {
                    points: input
                        .points
                        .iter()
                        .map(|p| {
                            if join_direct && id == "sumo:east_0" {
                                fixture_translate(*p, [-10.0, 0.0, 0.0])
                            } else {
                                *p
                            }
                        })
                        .collect(),
                },
            })
            .collect(),
    };
    let internal = traffic
        .maneuver_paths
        .iter()
        .flat_map(|p| p.internal_edge_ids.clone())
        .collect();
    (traffic, spatial, internal)
}

#[test]
fn direct_successors_are_deduplicated_sorted_and_order_independent() {
    let (mut traffic, _, _) = mixed_paths(true);
    let direct = traffic
        .maneuver_paths
        .iter()
        .find(|p| p.internal_edge_ids.is_empty())
        .unwrap()
        .clone();
    let mut other = direct.clone();
    other.exit_edge_id = "sumo:south_0".to_owned();
    traffic.maneuver_paths.extend([other, direct]);
    let expected = direct_successors(&traffic).unwrap();
    assert_eq!(expected.len(), 1);
    assert_eq!(
        expected["sumo:west_0"]
            .iter()
            .map(|e| e.local_key())
            .collect::<Vec<_>>(),
        ["east_0", "south_0"]
    );
    let mut reordered = traffic.clone();
    reordered.maneuver_paths.reverse();
    assert_eq!(direct_successors(&reordered).unwrap(), expected);
}

#[test]
fn internal_paths_do_not_add_successor_authority() {
    let observation = small_observation();
    assert!(
        direct_successors(observation.traffic.as_ref().unwrap())
            .unwrap()
            .is_empty()
    );
    let (traffic, _, internal) = mixed_paths(true);
    let successors = direct_successors(&traffic).unwrap();
    assert_eq!(successors.len(), 1);
    assert_eq!(
        successors["sumo:west_0"]
            .iter()
            .map(|e| e.local_key())
            .collect::<Vec<_>>(),
        ["east_0"]
    );
    for edge in internal {
        assert!(!successors.contains_key(edge.as_str()));
        assert!(
            !successors
                .values()
                .flatten()
                .any(|target| target.local_key() == edge_key(&edge))
        );
    }
}

#[test]
fn production_direct_and_internal_paths_keep_complete_signal_closure() {
    let (traffic, spatial, internal) = mixed_paths(true);
    let original = json!(traffic);
    let first = compile_network_lfca(&traffic, &spatial, &internal).unwrap();
    let second = compile_network_lfca(&traffic, &spatial, &internal).unwrap();
    assert_eq!(first, second, "deterministic complete artifact");
    assert!(!first.network_lfca.is_empty());
    assert_eq!(json!(traffic), original, "source controls and topology");
    assert_eq!(first.counts.lane_edges, 4);
    assert_eq!(first.counts.maneuver_paths, 2);
    assert_eq!(first.counts.signal_controllers, 1);
    assert_eq!(first.counts.signal_groups, 2);
    assert_eq!(first.counts.signal_phases, 4);
    assert_eq!(first.counts.stop_lines, 1);
    assert_eq!(first.counts.maneuver_gates, 2);
}

#[test]
fn direct_successor_does_not_bypass_spatial_join_validation() {
    let (traffic, spatial, internal) = mixed_paths(false);
    let error = compile_network_lfca(&traffic, &spatial, &internal)
        .unwrap_err()
        .to_string();
    assert!(error.contains("DiscontinuousJoin"), "{error}");
    assert!(
        !error.contains("LF-COMP-DISCONNECTED-MANEUVER-PATH"),
        "{error}"
    );
}
