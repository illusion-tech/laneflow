use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Component {
    pub(super) id: String,
    pub(super) target_paths: Vec<String>,
    pub(super) paths: Vec<String>,
    pub(super) edges: Vec<String>,
    pub(super) missing_edges: Vec<String>,
    pub(super) controllers: Vec<String>,
}

struct Union(Vec<usize>);

impl Union {
    fn root(&mut self, node: usize) -> usize {
        let mut root = node;
        while self.0[root] != root {
            root = self.0[root];
        }
        let mut current = node;
        while current != root {
            let next = self.0[current];
            self.0[current] = root;
            current = next;
        }
        root
    }

    fn join(&mut self, left: usize, right: usize) {
        let left = self.root(left);
        let right = self.root(right);
        self.0[left.max(right)] = left.min(right);
    }
}

pub(super) fn inventory(
    traffic: &TrafficPackage,
    originals: &BTreeMap<String, re::RoadEditingCurveProgram>,
    targets: &BTreeSet<String>,
) -> Vec<Component> {
    let mut paths = traffic.maneuver_paths.iter().collect::<Vec<_>>();
    paths.sort_by(|a, b| a.id.cmp(&b.id));
    let index = paths
        .iter()
        .enumerate()
        .map(|(i, path)| (path.id.as_str(), i))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(index.len(), paths.len(), "unique path identities");
    assert!(targets.iter().all(|id| index.contains_key(id.as_str())));
    let stops = traffic
        .signals
        .stop_lines
        .iter()
        .map(|stop| (stop.id.as_str(), stop.edge_id.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut dependencies = paths
        .iter()
        .map(|path| path_edges(path).into_iter().collect::<BTreeSet<_>>())
        .collect::<Vec<_>>();
    let mut group_paths = BTreeMap::<&str, BTreeSet<usize>>::new();
    for gate in &traffic.signals.maneuver_gates {
        let i = index[gate.maneuver_path_id.as_str()];
        dependencies[i].insert(stops[gate.stop_line_id.as_str()]);
        group_paths
            .entry(gate.signal_control.group_id.as_str())
            .or_default()
            .insert(i);
    }
    let mut union = Union((0..paths.len()).collect());
    let mut first_user = BTreeMap::<&str, usize>::new();
    for (i, edges) in dependencies.iter().enumerate() {
        for &edge in edges {
            if !originals.contains_key(edge)
                && let Some(&first) = first_user.get(edge)
            {
                union.join(first, i);
            } else if !originals.contains_key(edge) {
                first_user.insert(edge, i);
            }
        }
    }
    let mut controller_users = BTreeMap::new();
    for controller in &traffic.signals.controllers {
        let users = controller
            .group_ids
            .iter()
            .filter_map(|group| group_paths.get(group.as_str()))
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>();
        if let Some(&first) = users.first() {
            for &other in &users {
                union.join(first, other);
            }
        }
        controller_users.insert(controller.id.as_str(), users);
    }
    let roots = (0..paths.len()).map(|i| union.root(i)).collect::<Vec<_>>();
    let mut groups = BTreeMap::<usize, Vec<usize>>::new();
    for (i, &root) in roots.iter().enumerate() {
        groups.entry(root).or_default().push(i);
    }
    let mut components = Vec::new();
    for (root, members) in groups {
        let target_paths = members
            .iter()
            .filter(|&&i| targets.contains(&paths[i].id))
            .map(|&i| paths[i].id.clone())
            .collect::<Vec<_>>();
        if target_paths.is_empty() {
            continue;
        }
        let edges = members
            .iter()
            .flat_map(|&i| dependencies[i].iter().copied())
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let missing_edges = edges
            .iter()
            .filter(|id| !originals.contains_key(*id))
            .cloned()
            .collect();
        let controllers = controller_users
            .iter()
            .filter(|(_, users)| users.first().is_some_and(|&first| roots[first] == root))
            .map(|(id, users)| {
                assert!(users.iter().all(|&i| roots[i] == root));
                (*id).to_owned()
            })
            .collect();
        components.push(Component {
            id: paths[members[0]].id.clone(),
            target_paths,
            paths: members.iter().map(|&i| paths[i].id.clone()).collect(),
            edges: edges.into_iter().collect(),
            missing_edges,
            controllers,
        });
    }
    components.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(
        components
            .iter()
            .map(|c| c.target_paths.len())
            .sum::<usize>(),
        targets.len(),
        "complete target partition"
    );
    components
}

pub(super) fn selection(components: &[Component]) -> Value {
    let mut priority = components.iter().collect::<Vec<_>>();
    priority.sort_by(|a, b| {
        a.missing_edges
            .len()
            .cmp(&b.missing_edges.len())
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut selected = Vec::new();
    let mut edges = BTreeSet::new();
    let mut excluded = Vec::new();
    for component in priority {
        let reason = if component.missing_edges.is_empty() {
            Some("original_programs_complete")
        } else if component.missing_edges.len() > 2 {
            Some("more_than_two_missing_programs")
        } else if component.paths.len() > 64 || component.edges.len() > 128 {
            Some("component_size_limit")
        } else if selected.len() == 32 || edges.len() + component.missing_edges.len() > 64 {
            Some("cohort_size_limit")
        } else {
            None
        };
        if let Some(reason) = reason {
            excluded.push(json!({"component":component.id,"reason":reason}));
        } else {
            selected.push(component.id.clone());
            for edge in &component.missing_edges {
                assert!(edges.insert(edge.clone()), "unique mutable component owner");
            }
        }
    }
    json!({"order":"missing_program_count_then_component_path_id",
        "limits":{"missing_programs_per_component":2,"paths_per_component":64,
            "edges_per_component":128,"components":32,"search_edges":64},
        "component_ids":selected,"search_edges":edges,"excluded":excluded})
}

#[test]
fn shared_failed_edge_pulls_all_users_but_fixed_success_does_not() {
    let observation = small_observation();
    let mut traffic = observation.traffic.unwrap();
    traffic.signals.controllers.clear();
    traffic.signals.groups.clear();
    traffic.signals.maneuver_gates.clear();
    traffic.signals.stop_lines.clear();
    let mut originals = small_programs(&small_observation());
    let target = traffic.maneuver_paths[0].id.clone();
    let shared = traffic.maneuver_paths[0].entry_edge_id.clone();
    let targets = BTreeSet::from([target.clone()]);
    let fixed = inventory(&traffic, &originals, &targets);
    assert_eq!(fixed[0].paths, vec![target.clone()]);
    originals.remove(&shared).unwrap();
    let varied = inventory(&traffic, &originals, &targets);
    let expected = traffic
        .maneuver_paths
        .iter()
        .filter(|path| path_edges(path).contains(&shared.as_str()))
        .map(|path| path.id.clone())
        .collect::<BTreeSet<_>>();
    assert!(expected.len() > 1, "shared fixture edge");
    assert_eq!(
        varied[0].paths.iter().cloned().collect::<BTreeSet<_>>(),
        expected
    );
    assert_eq!(varied[0].missing_edges, vec![shared]);
    traffic.maneuver_paths.reverse();
    assert_eq!(inventory(&traffic, &originals, &targets), varied);
}

#[test]
fn complete_controller_members_remain_even_when_geometry_is_fixed() {
    let observation = small_observation();
    let traffic = observation.traffic.as_ref().unwrap();
    let originals = small_programs(&observation);
    let path = &traffic.maneuver_paths[0];
    let part = clip(traffic, path);
    assert!(!part.signals.controllers.is_empty());
    let components = inventory(traffic, &originals, &BTreeSet::from([path.id.clone()]));
    assert_eq!(components.len(), 1);
    assert_eq!(
        components[0].paths.iter().cloned().collect::<BTreeSet<_>>(),
        part.maneuver_paths.iter().map(|p| p.id.clone()).collect()
    );
    assert!(components[0].missing_edges.is_empty());
}

#[test]
fn selection_never_cuts_a_large_missing_dependency_component() {
    let mut component = Component {
        id: "a".to_owned(),
        target_paths: vec!["a".to_owned()],
        paths: vec!["a".to_owned(), "b".to_owned()],
        edges: vec!["e0".to_owned(), "e1".to_owned()],
        missing_edges: vec!["e0".to_owned(), "e1".to_owned()],
        controllers: vec!["signal".to_owned()],
    };
    assert_eq!(
        selection(&[component.clone()])["component_ids"],
        json!(["a"])
    );
    component.missing_edges.push("e2".to_owned());
    let result = selection(&[component]);
    assert_eq!(result["component_ids"], json!([]));
    assert_eq!(
        result["excluded"][0]["reason"],
        "more_than_two_missing_programs"
    );
}
