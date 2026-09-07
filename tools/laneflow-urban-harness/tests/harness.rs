use std::fs;

use laneflow_runtime::VehicleStatus;
use laneflow_urban_generator::{Scale, UrbanConfig, generate};
use laneflow_urban_harness::{
    Artifacts, Harness, ResolvedPlan, Window, compare_runs, run_to_directory,
};

#[test]
fn real_fixture_runs_independently_and_detects_changed_inputs_and_logs() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let config =
        UrbanConfig::parse(include_str!("../../../examples/config/cn-urban.toml")).unwrap();
    generate(&config, Scale::Fixture, &source, None).unwrap();
    let artifacts = Artifacts::load(&source).unwrap();
    let plan = ResolvedPlan::mixed(
        &artifacts,
        Window {
            purpose: "probe".into(),
            warm_up_ticks: 512,
            observation_ticks: 512,
        },
    )
    .unwrap();
    let plan_path = temp.path().join("plan.toml");
    plan.write(&plan_path).unwrap();
    assert_eq!(ResolvedPlan::read(&plan_path).unwrap(), plan);
    let harness = Harness::install(&artifacts, &plan).unwrap();
    assert_eq!(harness.world().live_vehicles().len(), 2_000);
    let mut profiles = std::collections::BTreeMap::new();
    let mut parked = 0;
    for handle in harness.world().live_vehicles() {
        let v = harness.world().vehicle(*handle).unwrap();
        *profiles.entry(v.length_mm()).or_insert(0) += 1;
        let id = harness.stable_individual(*handle).unwrap();
        assert_eq!(id.incarnation, 0);
        if v.status() == VehicleStatus::Parked {
            parked += 1;
            assert!(harness.world().parking_binding(*handle).is_some());
        }
    }
    assert_eq!(parked, 500);
    assert_eq!(
        profiles,
        [(4_000, 600), (4_500, 1_200), (6_000, 200)].into()
    );
    drop(harness);
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    let first = run_to_directory(&artifacts, &plan, &a).unwrap();
    let second = run_to_directory(&artifacts, &plan, &b).unwrap();
    assert_eq!(first.status, "probe-complete", "{:?}", first.error);
    assert_eq!(first, second);
    assert_eq!(first.completed_ticks, 1_024);
    assert!(
        first
            .tile_evidence
            .iter()
            .all(|e| e.explicit_parks == 1 && e.virtual_parks == 1)
    );
    let commands: Vec<serde_json::Value> = fs::read_to_string(a.join("commands.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let parks: Vec<_> = commands.iter().filter(|c| c["command"] == "park").collect();
    assert_eq!(parks.len(), 4);
    for park in parks {
        let id = &park["individual"];
        let slot = id["tile"].as_u64().unwrap() * 1_000 + id["slot"].as_u64().unwrap();
        let role = plan
            .arrivals
            .iter()
            .find(|r| u64::from(r.slot) == slot)
            .unwrap();
        assert_eq!(
            id["incarnation"], 0,
            "parking preserves the initial individual"
        );
        assert_eq!(park["committed"], true);
        assert!(park["boundary"].as_u64().unwrap() >= role.park_not_before_tick);
    }
    let events: Vec<serde_json::Value> = fs::read_to_string(a.join("events.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let arrivals: Vec<_> = events
        .iter()
        .filter(|e| e["kind"] == "parking-arrival")
        .collect();
    assert_eq!(arrivals.len(), 4);
    assert!(
        arrivals
            .iter()
            .all(|e| e["tick"].as_u64().unwrap() < plan.window.warm_up_ticks),
        "warm-up arrival must wait for the planned observation-window park"
    );
    assert!(
        first
            .tile_evidence
            .iter()
            .all(|e| e.planned_east >= 7 && e.planned_east * 3 == e.planned_west * 7)
    );
    assert!(first.exhausted_departures > 0);
    assert!(
        first
            .retry_reasons
            .get("leave-overlap")
            .copied()
            .unwrap_or(0)
            > 0
    );
    assert!(first.atomic_rejections.contains_key("leave"));
    assert!(compare_runs(&a, &b).unwrap().contains("not formal"));
    assert!(compare_runs(&a, &a).is_err());
    assert!(run_to_directory(&artifacts, &plan, &a).is_err());
    let mut changed = plan.clone();
    changed.initial[0].progress_mm += 1;
    assert!(Harness::install(&artifacts, &changed).is_err());
    fs::write(b.join("ticks.jsonl"), "\n").unwrap();
    assert!(
        compare_runs(&a, &b)
            .unwrap_err()
            .to_string()
            .contains("run file changed")
    );
    fs::write(source.join("routes.toml"), "\n").unwrap();
    assert!(
        Artifacts::load(&source)
            .err()
            .unwrap()
            .to_string()
            .contains("artifact digest mismatch")
    );
}
