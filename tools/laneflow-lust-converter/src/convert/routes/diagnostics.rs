//! 按需路线阶段诊断；正式路线入口不经过此模块。

use std::{collections::BTreeSet, time::Instant};

use crate::{
    Error,
    convert::{junction::NormalizedTopology, population::PopulationRecord},
    output::model::ManeuverPath,
    sumo::SumoNetwork,
};

use super::{
    BoundPopulationRecord, HopOption, RouteObserver, RoutePopulationBundle,
    build_external_lanes_by_edge, build_hop_index, build_routes_and_bind_population,
    expand_road_route_with_observer, validate_known_edges,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RouteDiagnosticKind {
    Success,
    SourceReference,
    RoadHopMissing,
    LaneContinuityGap,
    MaterializationFailure,
    Unclassified,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RouteDiagnosticStage {
    SourceValidation,
    Normalization,
    RouteExpansion,
    Materialization,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DiagnosticError {
    pub variant: &'static str,
    pub message: String,
}

impl DiagnosticError {
    fn from_error(error: Error) -> Self {
        let variant = match &error {
            Error::Io { .. } => "Io",
            Error::Toml(_) => "Toml",
            Error::Config(_) => "Config",
            Error::MissingSourceFile { .. } => "MissingSourceFile",
            Error::SourceSizeMismatch { .. } => "SourceSizeMismatch",
            Error::SourceDigestMismatch { .. } => "SourceDigestMismatch",
            Error::SourceChangedAfterVerification { .. } => "SourceChangedAfterVerification",
            Error::SourceRevisionUnknown { .. } => "SourceRevisionUnknown",
            Error::SourceRevisionMismatch { .. } => "SourceRevisionMismatch",
            Error::XmlParse(_) => "XmlParse",
            Error::SumoModel(_) => "SumoModel",
            Error::Json { .. } => "Json",
            Error::TomlSerialize { .. } => "TomlSerialize",
            Error::Validation { .. } => "Validation",
        };
        let message = match error {
            Error::SumoModel(message) => message,
            other => other.to_string(),
        };
        Self { variant, message }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct HopDiagnostic {
    pub occurrence: usize,
    pub from_lane: u32,
    pub to_lane: u32,
    pub path: ManeuverPath,
}

impl HopDiagnostic {
    fn from_hop(occurrence: usize, hop: &HopOption) -> Self {
        Self {
            occurrence,
            from_lane: hop.from_lane,
            to_lane: hop.to_lane,
            path: hop.path.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FirstGap {
    pub kind: RouteDiagnosticKind,
    pub from_occurrence: usize,
    pub to_occurrence: usize,
    pub from_road: String,
    pub to_road: String,
    pub reachable_lanes: Vec<u32>,
    pub required_from_lanes: Vec<u32>,
    pub candidates: Vec<HopDiagnostic>,
}

#[derive(Clone, Debug)]
pub(crate) struct MaterializedRoute {
    pub edge_ids: Vec<String>,
    pub catalog_id: Option<String>,
    pub bound: Option<BoundPopulationRecord>,
}

#[derive(Clone, Debug)]
pub(crate) struct RouteDiagnostic {
    pub kind: RouteDiagnosticKind,
    pub stage: RouteDiagnosticStage,
    pub population: Option<PopulationRecord>,
    pub road_edge_ids: Vec<String>,
    pub first_gap: Option<FirstGap>,
    pub route: Option<MaterializedRoute>,
    pub original_error: Option<DiagnosticError>,
    pub injected: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct DiagnosticMetrics {
    pub records: usize,
    pub expansion_calls: usize,
    pub state_evaluations: u64,
    pub memo_hits: u64,
    pub materialize_calls: u64,
    pub elapsed_ns: u128,
}

#[derive(Clone, Debug)]
pub(crate) struct RouteDiagnosticReport {
    pub formal: Result<RoutePopulationBundle, DiagnosticError>,
    pub records: Vec<RouteDiagnostic>,
    pub metrics: DiagnosticMetrics,
}

#[derive(Default)]
struct Trace {
    start_lanes: Option<Vec<u32>>,
    candidates: Vec<HopDiagnostic>,
    state_evaluations: u64,
    memo_hits: u64,
    materialize_calls: u64,
    inject_materialize_failure: bool,
}

impl RouteObserver for Trace {
    fn start_lanes(&mut self, lanes: &[u32]) {
        self.start_lanes = Some(lanes.to_vec());
    }

    fn state(&mut self, _occurrence: usize, _lane: u32, _result: bool, cached: bool) {
        if cached {
            self.memo_hits += 1;
        } else {
            self.state_evaluations += 1;
        }
    }

    fn hop(&mut self, occurrence: usize, hop: &HopOption) {
        self.candidates
            .push(HopDiagnostic::from_hop(occurrence, hop));
    }

    fn materialize_entry(&mut self, _start_lane: u32) {
        self.materialize_calls += 1;
    }

    #[cfg(test)]
    fn inject_materialize_failure(&mut self) -> bool {
        self.inject_materialize_failure
    }
}

pub(crate) fn diagnose_normalization_failure(error: Error) -> RouteDiagnostic {
    RouteDiagnostic {
        kind: RouteDiagnosticKind::SourceReference,
        stage: RouteDiagnosticStage::Normalization,
        population: None,
        road_edge_ids: Vec::new(),
        first_gap: None,
        route: None,
        original_error: Some(DiagnosticError::from_error(error)),
        injected: false,
    }
}

pub(crate) fn diagnose_population(
    network: &SumoNetwork,
    topology: &NormalizedTopology,
    population: &[PopulationRecord],
) -> RouteDiagnosticReport {
    diagnose_population_impl(network, topology, population, false)
}

fn diagnose_record(
    network: &SumoNetwork,
    lanes_by_edge: &std::collections::HashMap<&str, Vec<u32>>,
    hops: &std::collections::HashMap<(String, String), Vec<HopOption>>,
    population: &PopulationRecord,
    inject_materialize_failure: bool,
) -> (RouteDiagnostic, Trace) {
    let mut trace = Trace {
        inject_materialize_failure,
        ..Trace::default()
    };
    if let Err(error) =
        validate_known_edges(network, &population.road_edge_ids, &population.vehicle_id)
    {
        return (
            RouteDiagnostic {
                kind: RouteDiagnosticKind::SourceReference,
                stage: RouteDiagnosticStage::SourceValidation,
                population: Some(population.clone()),
                road_edge_ids: population.road_edge_ids.clone(),
                first_gap: None,
                route: None,
                original_error: Some(DiagnosticError::from_error(error)),
                injected: false,
            },
            trace,
        );
    }
    let result = expand_road_route_with_observer(
        network,
        lanes_by_edge,
        hops,
        &population.road_edge_ids,
        &mut trace,
    );
    let injected = trace.inject_materialize_failure;
    let (kind, stage, first_gap, route, original_error) = match result {
        Ok(edge_ids) => (
            RouteDiagnosticKind::Success,
            RouteDiagnosticStage::Materialization,
            None,
            Some(MaterializedRoute {
                edge_ids,
                catalog_id: None,
                bound: None,
            }),
            None,
        ),
        Err(error) if trace.materialize_calls > 0 => (
            RouteDiagnosticKind::MaterializationFailure,
            RouteDiagnosticStage::Materialization,
            None,
            None,
            Some(DiagnosticError::from_error(error)),
        ),
        Err(error) => {
            let gap = first_gap(hops, &population.road_edge_ids, &trace);
            (
                gap.as_ref()
                    .map_or(RouteDiagnosticKind::Unclassified, |gap| gap.kind),
                RouteDiagnosticStage::RouteExpansion,
                gap,
                None,
                Some(DiagnosticError::from_error(error)),
            )
        }
    };
    (
        RouteDiagnostic {
            kind,
            stage,
            population: Some(population.clone()),
            road_edge_ids: population.road_edge_ids.clone(),
            first_gap,
            route,
            original_error,
            injected,
        },
        trace,
    )
}

fn first_gap(
    hops: &std::collections::HashMap<(String, String), Vec<HopOption>>,
    road_edge_ids: &[String],
    trace: &Trace,
) -> Option<FirstGap> {
    let mut current = trace.start_lanes.clone()?;
    current.sort_unstable();
    current.dedup();
    for occurrence in 0..road_edge_ids.len().saturating_sub(1) {
        let from_road = &road_edge_ids[occurrence];
        let to_road = &road_edge_ids[occurrence + 1];
        let options = hops
            .get(&(from_road.clone(), to_road.clone()))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if options.is_empty() {
            return Some(FirstGap {
                kind: RouteDiagnosticKind::RoadHopMissing,
                from_occurrence: occurrence,
                to_occurrence: occurrence + 1,
                from_road: from_road.clone(),
                to_road: to_road.clone(),
                reachable_lanes: current,
                required_from_lanes: Vec::new(),
                candidates: Vec::new(),
            });
        }
        let next = trace
            .candidates
            .iter()
            .filter(|candidate| {
                candidate.occurrence == occurrence && current.contains(&candidate.from_lane)
            })
            .map(|candidate| candidate.to_lane)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if next.is_empty() {
            let mut candidates = options
                .iter()
                .map(|hop| HopDiagnostic::from_hop(occurrence, hop))
                .collect::<Vec<_>>();
            candidates.sort_by(|left, right| {
                left.from_lane
                    .cmp(&right.from_lane)
                    .then(left.to_lane.cmp(&right.to_lane))
                    .then(left.path.id.cmp(&right.path.id))
            });
            return Some(FirstGap {
                kind: RouteDiagnosticKind::LaneContinuityGap,
                from_occurrence: occurrence,
                to_occurrence: occurrence + 1,
                from_road: from_road.clone(),
                to_road: to_road.clone(),
                reachable_lanes: current,
                required_from_lanes: options
                    .iter()
                    .map(|hop| hop.from_lane)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
                candidates,
            });
        }
        current = next;
    }
    None
}

#[cfg(test)]
thread_local! {
    static DIAGNOSTIC_ENTRY_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn note_diagnostic_entry() {
    DIAGNOSTIC_ENTRY_COUNT.with(|count| count.set(count.get() + 1));
}

#[cfg(not(test))]
fn note_diagnostic_entry() {}

#[cfg(test)]
pub(crate) fn reset_diagnostic_entry_count() {
    DIAGNOSTIC_ENTRY_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn diagnostic_entry_count() -> usize {
    DIAGNOSTIC_ENTRY_COUNT.with(|count| count.get())
}

fn diagnose_population_impl(
    network: &SumoNetwork,
    topology: &NormalizedTopology,
    population: &[PopulationRecord],
    inject_materialize_failure: bool,
) -> RouteDiagnosticReport {
    note_diagnostic_entry();
    let hops = build_hop_index(topology);
    let lanes_by_edge = build_external_lanes_by_edge(network);
    let formal = build_routes_and_bind_population(network, topology, population)
        .map_err(DiagnosticError::from_error);
    let started = Instant::now();
    let mut metrics = DiagnosticMetrics::default();
    let mut records = Vec::with_capacity(population.len());
    for population_record in population {
        let (mut record, trace) = diagnose_record(
            network,
            &lanes_by_edge,
            &hops,
            population_record,
            inject_materialize_failure,
        );
        if let Some(route) = record.route.as_mut()
            && let Ok(bundle) = &formal
        {
            let bound = bundle
                .records
                .iter()
                .find(|bound| {
                    bound.population_rank == population_record.population_rank
                        && bound.vehicle_id == population_record.vehicle_id
                })
                .cloned();
            route.catalog_id = bound.as_ref().map(|bound| bound.route_id.clone());
            route.bound = bound;
        }
        metrics.expansion_calls += 1;
        metrics.state_evaluations += trace.state_evaluations;
        metrics.memo_hits += trace.memo_hits;
        metrics.materialize_calls += trace.materialize_calls;
        records.push(record);
    }
    metrics.elapsed_ns = started.elapsed().as_nanos();
    metrics.records = records.len();
    records.sort_by(|left, right| {
        let ordering = match (&left.population, &right.population) {
            (Some(left), Some(right)) => left
                .population_rank
                .cmp(&right.population_rank)
                .then_with(|| left.vehicle_id.cmp(&right.vehicle_id))
                .then_with(|| left.source_file_ordinal.cmp(&right.source_file_ordinal))
                .then_with(|| {
                    left.source_vehicle_ordinal
                        .cmp(&right.source_vehicle_ordinal)
                }),
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        };
        ordering.then_with(|| left.road_edge_ids.cmp(&right.road_edge_ids))
    });
    RouteDiagnosticReport {
        formal,
        records,
        metrics,
    }
}

#[cfg(test)]
pub(crate) fn diagnose_population_with_materialize_injection(
    network: &SumoNetwork,
    topology: &NormalizedTopology,
    population: &[PopulationRecord],
) -> RouteDiagnosticReport {
    diagnose_population_impl(network, topology, population, true)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::convert::{
        junction::normalize_junctions,
        routes::test_support::{
            NO_COMPLETE_ROUTE_ERROR, RouteFixture, RouteFixtureKind, population_record,
        },
    };

    fn only(report: &RouteDiagnosticReport) -> &RouteDiagnostic {
        assert_eq!(report.records.len(), 1);
        &report.records[0]
    }

    fn assert_original(record: &RouteDiagnostic, message: &str) {
        let error = record.original_error.as_ref().expect("original error");
        assert_eq!(error.variant, "SumoModel");
        assert_eq!(error.message, message);
    }

    fn assert_gap(
        record: &RouteDiagnostic,
        kind: RouteDiagnosticKind,
        from: usize,
        to: usize,
        reachable: &[u32],
        required: &[u32],
    ) {
        let gap = record.first_gap.as_ref().expect("first gap");
        assert_eq!(record.kind, kind);
        assert_eq!(gap.kind, kind);
        assert_eq!(gap.from_occurrence, from);
        assert_eq!(gap.to_occurrence, to);
        assert_eq!(gap.from_road, "B");
        assert_eq!(gap.to_road, "C");
        assert_eq!(gap.reachable_lanes, reachable);
        assert_eq!(gap.required_from_lanes, required);
    }

    fn assert_path(hop: &HopDiagnostic, id: &str, entry: &str, internals: &[&str], exit: &str) {
        assert_eq!(hop.path.id, id);
        assert_eq!(hop.path.entry_edge_id, entry);
        assert_eq!(
            hop.path.internal_edge_ids,
            internals
                .iter()
                .map(|edge| (*edge).to_owned())
                .collect::<Vec<_>>()
        );
        assert_eq!(hop.path.exit_edge_id, exit);
    }

    fn assert_success_chain(record: &RouteDiagnostic, expected: &[&str]) {
        assert_eq!(record.kind, RouteDiagnosticKind::Success);
        assert_eq!(record.stage, RouteDiagnosticStage::Materialization);
        assert!(record.first_gap.is_none());
        assert!(record.original_error.is_none());
        let route = record.route.as_ref().expect("materialized route");
        assert_eq!(
            route.edge_ids,
            expected
                .iter()
                .map(|edge| (*edge).to_owned())
                .collect::<Vec<_>>()
        );
        assert_eq!(route.catalog_id.as_deref(), Some("sumo:route-0"));
    }

    #[test]
    fn a1_gap_and_missing_hop_have_distinct_diagnostics() {
        let gap = RouteFixture::new(RouteFixtureKind::Gap);
        let network = gap.network();
        let topology = gap.normalized_topology(&network);
        let population = gap.selected_population();
        let report = diagnose_population(&network, &topology, &population);
        assert_eq!(report.records.len(), population.len());
        for record in &report.records {
            assert_eq!(record.stage, RouteDiagnosticStage::RouteExpansion);
            assert_gap(
                record,
                RouteDiagnosticKind::LaneContinuityGap,
                1,
                2,
                &[0],
                &[1],
            );
            assert_original(record, NO_COMPLETE_ROUTE_ERROR);
            let candidates = &record.first_gap.as_ref().unwrap().candidates;
            assert_eq!(candidates.len(), 1);
            assert_path(
                &candidates[0],
                "sumo:J2:B_1-to-C_0::J2_0_0",
                "sumo:B_1",
                &["sumo::J2_0_0"],
                "sumo:C_0",
            );
        }

        let missing = RouteFixture::new(RouteFixtureKind::MissingHop);
        let network = missing.network();
        let topology = missing.normalized_topology(&network);
        let population = missing.selected_population();
        let missing_report = diagnose_population(&network, &topology, &population);
        for record in &missing_report.records {
            assert_gap(record, RouteDiagnosticKind::RoadHopMissing, 1, 2, &[0], &[]);
            assert!(record.first_gap.as_ref().unwrap().candidates.is_empty());
            assert_original(record, NO_COMPLETE_ROUTE_ERROR);
        }
        assert_eq!(
            report.formal.as_ref().expect_err("gap formal error"),
            missing_report
                .formal
                .as_ref()
                .expect_err("missing formal error")
        );
    }

    #[test]
    fn a1_branch_success_does_not_report_first_dead_branch() {
        let fixture = RouteFixture::new(RouteFixtureKind::Gap);
        let network = fixture.network();
        let topology = fixture.normalized_topology(&network);
        let population = vec![population_record("branch-success", &["B", "C"])];
        let report = diagnose_population(&network, &topology, &population);
        let record = only(&report);
        assert_success_chain(record, &["sumo:B_1", "sumo::J2_0_0", "sumo:C_0"]);
        assert_eq!(
            record
                .route
                .as_ref()
                .unwrap()
                .bound
                .as_ref()
                .unwrap()
                .vehicle_id,
            "branch-success"
        );
        let bundle = report.formal.as_ref().expect("formal branch success");
        assert_eq!(
            bundle.routes[0].edge_ids,
            record.route.as_ref().unwrap().edge_ids
        );

        let fixture = RouteFixture::new(RouteFixtureKind::TwoLaneOrigin);
        let network = fixture.network();
        let topology = fixture.normalized_topology(&network);
        let population = fixture.selected_population();
        let report = diagnose_population(&network, &topology, &population);
        assert_eq!(report.records.len(), 2);
        for record in &report.records {
            assert_success_chain(
                record,
                &[
                    "sumo:A_1",
                    "sumo::J1_1_0",
                    "sumo:B_1",
                    "sumo::J2_0_0",
                    "sumo:C_0",
                ],
            );
        }
        assert_eq!(report.metrics.materialize_calls, 2);
        let ids = report
            .records
            .iter()
            .map(|record| record.population.as_ref().unwrap().vehicle_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["selected-z", "selected-a"]);
    }

    #[test]
    fn a1_all_failed_start_lanes_use_full_reachable_set() {
        let fixture = RouteFixture::new(RouteFixtureKind::MissingHop);
        let network = fixture.network();
        let topology = fixture.normalized_topology(&network);
        let population = vec![population_record("all-fail", &["B", "C"])];
        let report = diagnose_population(&network, &topology, &population);
        let record = only(&report);
        assert_eq!(record.stage, RouteDiagnosticStage::RouteExpansion);
        assert_gap(
            record,
            RouteDiagnosticKind::RoadHopMissing,
            0,
            1,
            &[0, 1],
            &[],
        );
        assert_original(
            record,
            "no complete lane-level path for road route [\"B\", \"C\"]",
        );
        assert_eq!(report.metrics.expansion_calls, 1);
        assert_eq!(report.metrics.materialize_calls, 0);
    }

    #[test]
    fn a1_repeated_occurrence_is_located_by_index() {
        let fixture =
            RouteFixture::new(RouteFixtureKind::LoopBack).with_road_sequence(&["A", "B", "C", "B"]);
        let network = fixture.network();
        let topology = fixture.normalized_topology(&network);
        let population = fixture.selected_population();
        let report = diagnose_population(&network, &topology, &population);
        for record in &report.records {
            assert_success_chain(
                record,
                &[
                    "sumo:A_0",
                    "sumo::J1_0_0",
                    "sumo:B_1",
                    "sumo::J2_0_0",
                    "sumo:C_0",
                    "sumo::J1_1_0",
                    "sumo:B_0",
                ],
            );
        }

        let fixture = RouteFixture::new(RouteFixtureKind::LoopBack)
            .with_road_sequence(&["A", "B", "C", "B", "C"]);
        let network = fixture.network();
        let topology = fixture.normalized_topology(&network);
        let population = fixture.selected_population();
        let report = diagnose_population(&network, &topology, &population);
        for record in &report.records {
            assert_eq!(record.stage, RouteDiagnosticStage::RouteExpansion);
            assert_gap(
                record,
                RouteDiagnosticKind::LaneContinuityGap,
                3,
                4,
                &[0],
                &[1],
            );
            assert_original(
                record,
                "no complete lane-level path for road route [\"A\", \"B\", \"C\", \"B\", \"C\"]",
            );
            let candidates = &record.first_gap.as_ref().unwrap().candidates;
            assert_eq!(candidates.len(), 1);
            assert_path(
                &candidates[0],
                "sumo:J2:B_1-to-C_0::J2_0_0",
                "sumo:B_1",
                &["sumo::J2_0_0"],
                "sumo:C_0",
            );
        }
    }

    #[test]
    fn a1_source_errors_and_unclassified_boundaries_are_distinct() {
        let fixture = RouteFixture::new(RouteFixtureKind::Gap);
        let network = fixture.network();
        let topology = fixture.normalized_topology(&network);

        let population = vec![population_record("unknown-road", &["A", "B", "GHOST"])];
        let report = diagnose_population(&network, &topology, &population);
        let record = only(&report);
        assert_eq!(record.kind, RouteDiagnosticKind::SourceReference);
        assert_eq!(record.stage, RouteDiagnosticStage::SourceValidation);
        assert!(record.first_gap.is_none());
        assert_original(
            record,
            "DUE vehicle \"unknown-road\" references unknown road edge \"GHOST\"",
        );
        assert_eq!(record.population.as_ref().unwrap().population_rank, 0);

        let population = vec![population_record("internal-road", &["A", "B", ":J2_0"])];
        let report = diagnose_population(&network, &topology, &population);
        let record = only(&report);
        assert_eq!(record.kind, RouteDiagnosticKind::SourceReference);
        assert_eq!(record.stage, RouteDiagnosticStage::SourceValidation);
        assert_original(
            record,
            "DUE vehicle \"internal-road\" route references internal edge \":J2_0\"",
        );

        let unknown_lane = RouteFixture::new(RouteFixtureKind::UnknownLane);
        let network = unknown_lane.network();
        let error = normalize_junctions(
            &network,
            &crate::convert::junction::StubWeldGate::Unrestricted,
        )
        .expect_err("unknown lane normalization");
        let record = diagnose_normalization_failure(error);
        assert_eq!(record.kind, RouteDiagnosticKind::SourceReference);
        assert_eq!(record.stage, RouteDiagnosticStage::Normalization);
        assert!(record.population.is_none());
        assert!(record.road_edge_ids.is_empty());
        assert_original(
            &record,
            "connection references unknown lane edge=\"B\" index=9",
        );

        let population = vec![population_record("empty", &[])];
        let report = diagnose_population(&network, &topology, &population);
        let record = only(&report);
        assert_eq!(record.kind, RouteDiagnosticKind::Unclassified);
        assert_eq!(record.stage, RouteDiagnosticStage::RouteExpansion);
        assert!(record.first_gap.is_none());
        assert_original(record, "road-level route must contain at least one edge");

        let population = vec![population_record("single", &["B"])];
        let report = diagnose_population(&network, &topology, &population);
        let record = only(&report);
        assert_success_chain(record, &["sumo:B_0"]);
    }

    #[test]
    fn a1_materialization_failure_is_limited_to_actual_entry_and_injection() {
        let fixture = RouteFixture::new(RouteFixtureKind::Continuous);
        let network = fixture.network();
        let topology = fixture.normalized_topology(&network);
        let population = fixture.selected_population();
        let report = diagnose_population(&network, &topology, &population);
        assert_eq!(report.metrics.materialize_calls, 2);
        for record in &report.records {
            assert_eq!(record.kind, RouteDiagnosticKind::Success);
            assert!(!record.injected);
        }

        let report =
            diagnose_population_with_materialize_injection(&network, &topology, &population);
        assert!(report.formal.is_ok());
        assert_eq!(report.metrics.materialize_calls, 2);
        for record in &report.records {
            assert_eq!(record.kind, RouteDiagnosticKind::MaterializationFailure);
            assert_eq!(record.stage, RouteDiagnosticStage::Materialization);
            assert!(record.injected);
            assert!(record.first_gap.is_none());
            assert!(record.route.is_none());
            assert_original(record, "injected materialize failure");
        }
    }

    #[test]
    fn a1_stability_preserves_formal_behavior_and_diagnostic_order() {
        assert_stable(RouteFixtureKind::Continuous);
        assert_stable(RouteFixtureKind::Gap);
    }

    fn assert_stable(kind: RouteFixtureKind) {
        let fixture = RouteFixture::new(kind);
        let network = fixture.network();
        let topology = fixture.normalized_topology(&network);
        let population = fixture.selected_population();
        let first = diagnose_population(&network, &topology, &population);
        let second = diagnose_population(&network, &topology, &population);
        assert_eq!(diagnostic_json(&first), diagnostic_json(&second));

        let mut reordered_network = network;
        reordered_network.lanes.reverse();
        let mut reordered_topology = topology;
        let entries = reordered_topology
            .path_by_connection
            .into_iter()
            .collect::<Vec<_>>();
        reordered_topology.path_by_connection = entries.into_iter().rev().collect();
        let reordered = diagnose_population(&reordered_network, &reordered_topology, &population);
        assert_eq!(diagnostic_json(&first), diagnostic_json(&reordered));
        assert_eq!(formal_json(&first), formal_json(&reordered));
    }

    #[test]
    fn a1_diagnostic_cost_is_opt_in_and_measured() {
        reset_diagnostic_entry_count();
        let fixture = RouteFixture::new(RouteFixtureKind::Continuous);
        let network = fixture.network();
        let topology = fixture.normalized_topology(&network);
        let population = fixture.selected_population();
        build_routes_and_bind_population(&network, &topology, &population)
            .expect("ordinary build without diagnostics");
        assert_eq!(diagnostic_entry_count(), 0);
        let report = diagnose_population(&network, &topology, &population);
        assert_eq!(diagnostic_entry_count(), 1);
        assert_eq!(report.metrics.records, population.len());
        assert_eq!(report.metrics.expansion_calls, population.len());
        assert!(report.metrics.state_evaluations > 0);
        assert_eq!(report.metrics.materialize_calls, population.len() as u64);
        println!(
            "A1_COST records={} expansion_calls={} state_evaluations={} memo_hits={} materialize_calls={} elapsed_ns={}",
            report.metrics.records,
            report.metrics.expansion_calls,
            report.metrics.state_evaluations,
            report.metrics.memo_hits,
            report.metrics.materialize_calls,
            report.metrics.elapsed_ns
        );
    }

    fn diagnostic_json(report: &RouteDiagnosticReport) -> Value {
        json!({
            "formal": formal_json(report),
            "records": report.records.iter().map(record_json).collect::<Vec<_>>(),
            "metrics": {
                "records": report.metrics.records,
                "expansionCalls": report.metrics.expansion_calls,
                "stateEvaluations": report.metrics.state_evaluations,
                "memoHits": report.metrics.memo_hits,
                "materializeCalls": report.metrics.materialize_calls,
            }
        })
    }

    fn formal_json(report: &RouteDiagnosticReport) -> Value {
        match &report.formal {
            Ok(bundle) => json!({
                "outcome": "success",
                "routes": bundle.routes.iter().map(|route| json!({
                    "id": route.id,
                    "edgeIds": route.edge_ids,
                })).collect::<Vec<_>>(),
                "records": bundle.records.iter().map(bound_json).collect::<Vec<_>>(),
            }),
            Err(error) => json!({
                "outcome": "rejected",
                "error": error_json(error),
            }),
        }
    }

    fn record_json(record: &RouteDiagnostic) -> Value {
        json!({
            "kind": format!("{:?}", record.kind),
            "stage": format!("{:?}", record.stage),
            "population": record.population.as_ref().map(population_json),
            "roadEdgeIds": record.road_edge_ids,
            "firstGap": record.first_gap.as_ref().map(gap_json),
            "route": record.route.as_ref().map(route_json),
            "originalError": record.original_error.as_ref().map(error_json),
            "injected": record.injected,
        })
    }

    fn gap_json(gap: &FirstGap) -> Value {
        json!({
            "kind": format!("{:?}", gap.kind),
            "fromOccurrence": gap.from_occurrence,
            "toOccurrence": gap.to_occurrence,
            "fromRoad": gap.from_road,
            "toRoad": gap.to_road,
            "reachableLanes": gap.reachable_lanes,
            "requiredFromLanes": gap.required_from_lanes,
            "candidates": gap.candidates.iter().map(hop_json).collect::<Vec<_>>(),
        })
    }

    fn hop_json(hop: &HopDiagnostic) -> Value {
        json!({
            "occurrence": hop.occurrence,
            "fromLane": hop.from_lane,
            "toLane": hop.to_lane,
            "path": path_json(&hop.path),
        })
    }

    fn path_json(path: &ManeuverPath) -> Value {
        json!({
            "id": path.id,
            "movementId": path.movement_id,
            "entry": path.entry_edge_id,
            "internals": path.internal_edge_ids,
            "exit": path.exit_edge_id,
        })
    }

    fn route_json(route: &MaterializedRoute) -> Value {
        json!({
            "edgeIds": route.edge_ids,
            "catalogId": route.catalog_id,
            "bound": route.bound.as_ref().map(bound_json),
        })
    }

    fn population_json(record: &PopulationRecord) -> Value {
        json!({
            "populationRank": record.population_rank,
            "vehicleId": record.vehicle_id,
            "vehicleTypeId": record.type_id,
            "depart": record.depart.to_string(),
            "roadEdgeIds": record.road_edge_ids,
            "sourceFileOrdinal": record.source_file_ordinal,
            "sourceVehicleOrdinal": record.source_vehicle_ordinal,
        })
    }

    fn bound_json(record: &BoundPopulationRecord) -> Value {
        json!({
            "populationRank": record.population_rank,
            "vehicleId": record.vehicle_id,
            "vehicleProfileId": record.vehicle_profile_id,
            "depart": record.depart.to_string(),
            "routeId": record.route_id,
            "roadEdgeIds": record.road_edge_ids,
            "sourceFileOrdinal": record.source_file_ordinal,
            "sourceVehicleOrdinal": record.source_vehicle_ordinal,
        })
    }

    fn error_json(error: &DiagnosticError) -> Value {
        json!({
            "variant": error.variant,
            "message": error.message,
        })
    }

    mod evidence;
}
