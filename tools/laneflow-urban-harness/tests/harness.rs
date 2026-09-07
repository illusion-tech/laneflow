use std::{fs, process::Command};

use laneflow_runtime::VehicleStatus;
use laneflow_urban_generator::{Scale, UrbanConfig, generate};
use laneflow_urban_harness::{
    Artifacts, ComparisonReport, Harness, ResolvedPlan, UrbanCase, Window, compare_runs,
    run_to_directory,
};
use sha2::{Digest, Sha256};

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn real_fixture_runs_independently_and_detects_changed_inputs_and_logs() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let config =
        UrbanConfig::parse(include_str!("../../../examples/config/cn-urban.toml")).unwrap();
    generate(&config, Scale::Fixture, &source, None).unwrap();
    let manifest_text = fs::read_to_string(source.join("manifest.toml")).unwrap();
    let catalog_text = fs::read_to_string(source.join("routes.toml")).unwrap();
    for scale in ["10k", "100k"] {
        let mut manifest: toml::Value = toml::from_str(&manifest_text).unwrap();
        let mut catalog: toml::Value = toml::from_str(&catalog_text).unwrap();
        manifest["scale"] = scale.into();
        catalog["scale"] = scale.into();
        let catalog_bytes = toml::to_string(&catalog).unwrap().into_bytes();
        manifest["files"]["routes.toml"]["bytes"] = (catalog_bytes.len() as i64).into();
        manifest["files"]["routes.toml"]["sha256"] = sha256(&catalog_bytes).into();
        fs::write(source.join("routes.toml"), catalog_bytes).unwrap();
        fs::write(
            source.join("manifest.toml"),
            toml::to_string(&manifest).unwrap(),
        )
        .unwrap();
        assert!(
            Artifacts::load(&source)
                .err()
                .unwrap()
                .to_string()
                .contains("fixed scale")
        );
    }
    fs::write(source.join("routes.toml"), &catalog_text).unwrap();
    let mut wrong_step: toml::Value = toml::from_str(&manifest_text).unwrap();
    wrong_step["fixed_step_ms"] = 33.into();
    fs::write(
        source.join("manifest.toml"),
        toml::to_string(&wrong_step).unwrap(),
    )
    .unwrap();
    assert!(
        Artifacts::load(&source)
            .err()
            .unwrap()
            .to_string()
            .contains("fixed scale")
    );
    fs::write(source.join("manifest.toml"), manifest_text).unwrap();
    let artifacts = Artifacts::load(&source).unwrap();
    for case in UrbanCase::ALL {
        let case_plan =
            ResolvedPlan::for_case(&artifacts, case, Window::probe(128).unwrap()).unwrap();
        assert_eq!(case_plan.case, case.as_str());
        assert_eq!(case_plan.initial.len(), 2_000);
        let case_harness = Harness::install(&artifacts, &case_plan).unwrap();
        let mut counts = [0; 3];
        for handle in case_harness.world().live_vehicles() {
            counts[match case_harness.world().vehicle(*handle).unwrap().status() {
                VehicleStatus::Active => 0,
                VehicleStatus::Parked => 1,
                VehicleStatus::Completed => 2,
            }] += 1;
        }
        let expected = if case == UrbanCase::GarageEgress {
            [500, 1_500, 0]
        } else {
            [1_500, 500, 0]
        };
        assert_eq!(counts, expected, "{} initial shape", case.as_str());
        if case == UrbanCase::GarageIngress {
            assert_eq!(case_plan.arrivals.len(), 4);
            assert_eq!(case_plan.reservation_rejections.len(), 4);
        }
        if matches!(case, UrbanCase::GarageEgress | UrbanCase::BoundaryBurst) {
            let garages: Vec<_> = artifacts
                .catalog()
                .parking
                .iter()
                .filter(|target| target.kind == "virtual" && target.capacity == 1_000)
                .collect();
            assert_eq!(
                case_plan
                    .initial
                    .iter()
                    .filter(|vehicle| vehicle.role.as_deref() == Some("garage-exit-blocker"))
                    .count(),
                garages.len()
            );
            for garage in garages {
                let exit_edges: std::collections::BTreeSet<_> =
                    garage.exits.iter().map(|anchor| &anchor.edge).collect();
                for vehicle in case_plan.initial.iter().filter(|vehicle| {
                    vehicle.tile == garage.tile
                        && vehicle.parking.is_none()
                        && vehicle.role.is_none()
                }) {
                    let route = artifacts
                        .catalog()
                        .routes
                        .iter()
                        .find(|route| route.key == vehicle.route)
                        .unwrap();
                    assert!(
                        route.edge_keys[vehicle.occurrence as usize + 1..]
                            .iter()
                            .all(|edge| !exit_edges.contains(edge)),
                        "{} tile {} background initial route crosses a reserved garage exit",
                        case.as_str(),
                        garage.tile
                    );
                }
                for route_key in case_plan
                    .departures
                    .iter()
                    .filter(|batch| batch.first_slot / 1_000 == garage.tile)
                    .flat_map(|batch| &batch.routes)
                {
                    let route = artifacts
                        .catalog()
                        .routes
                        .iter()
                        .find(|route| route.key == *route_key)
                        .unwrap();
                    assert!(
                        route
                            .edge_keys
                            .iter()
                            .all(|edge| !exit_edges.contains(edge)),
                        "{} tile {} background departure crosses a reserved garage exit",
                        case.as_str(),
                        garage.tile
                    );
                }
                if case == UrbanCase::BoundaryBurst {
                    let boundary = case_plan
                        .boundary_windows
                        .iter()
                        .find(|window| window.tile == garage.tile)
                        .unwrap();
                    let pulse = case_plan
                        .role_departures
                        .iter()
                        .find(|departure| {
                            departure.slot / 1_000 == garage.tile
                                && departure.role == "garage-exit-blocker"
                        })
                        .unwrap();
                    assert_eq!(pulse.due_tick + 1, boundary.before_tick);
                }
            }
        }
    }
    assert!(
        Window::correctness(&artifacts)
            .unwrap_err()
            .to_string()
            .contains("10k or 100k")
    );
    let cli_plan = temp.path().join("cli-plan.toml");
    let rejected = Command::new(env!("CARGO_BIN_EXE_laneflow-urban-harness"))
        .arg("plan")
        .arg(&source)
        .arg(&cli_plan)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("10k or 100k"));
    assert!(!cli_plan.exists());
    let probe = Command::new(env!("CARGO_BIN_EXE_laneflow-urban-harness"))
        .arg("plan")
        .arg(&source)
        .arg(&cli_plan)
        .args(["--probe-ticks", "128"])
        .output()
        .unwrap();
    assert!(probe.status.success(), "{:?}", probe.stderr);
    assert_eq!(
        ResolvedPlan::read(&cli_plan).unwrap().window.purpose,
        "probe"
    );
    let plan = ResolvedPlan::mixed(
        &artifacts,
        Window {
            purpose: "probe".into(),
            warm_up_ticks: 512,
            observation_ticks: 512,
        },
    )
    .unwrap();
    assert!(
        ResolvedPlan::mixed(
            &artifacts,
            Window {
                purpose: "correctness".into(),
                warm_up_ticks: plan.cycle_ticks,
                observation_ticks: 2 * plan.cycle_ticks,
            },
        )
        .unwrap_err()
        .to_string()
        .contains("10k or 100k")
    );
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
    for line in fs::read_to_string(a.join("ticks.jsonl")).unwrap().lines() {
        let tick: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(tick["domain"], "road_motor_vehicle");
        assert_eq!(tick["N_individual"], 2_000);
        assert_eq!(tick["N_presented"], 0);
        assert_eq!(tick["N_aggregate_records"], 0);
        assert_eq!(tick["N_aggregate_equivalent"], 0);
        assert_eq!(tick["intent_basis"], "exact_active_before_step");
        assert!(tick["N_intent"].as_u64().unwrap() <= 2_000);
        assert_eq!(
            tick["N_active"].as_u64().unwrap()
                + tick["parked"].as_u64().unwrap()
                + tick["completed"].as_u64().unwrap(),
            2_000
        );
    }
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
    for command in commands.iter().filter(|c| {
        c["committed"] == true
            && matches!(c["command"].as_str(), Some("park" | "leave" | "replace"))
    }) {
        let matching: Vec<_> = events
            .iter()
            .filter(|e| {
                e["kind"] == "lifecycle"
                    && e["phase"] == "command"
                    && e["tick"] == command["boundary"]
                    && e["sequence"] == command["sequence"]
                    && e["attempt"] == command["attempt"]
                    && e["command"] == command["command"]
            })
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "missing or duplicate command lifecycle: {command}"
        );
        let event = matching[0];
        assert_eq!(event["individual"], command["individual"]);
        let (before, after) = match command["command"].as_str().unwrap() {
            "park" => (0, 1),
            "leave" => (1, 0),
            "replace" => (2, 0),
            _ => unreachable!(),
        };
        assert_eq!(event["before"], before);
        assert_eq!(event["after"], after);
        assert_eq!(
            event["after_individual"],
            if command["command"] == "replace" {
                &command["details"]["new_individual"]
            } else {
                &command["individual"]
            }
            .clone()
        );
    }
    assert!(
        !events
            .iter()
            .any(|e| e["phase"] == "command" && e["command"] == "reserve")
    );
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
    let comparison = compare_runs(&a, &b).unwrap();
    assert_eq!(comparison.status, "probe-match");
    assert_eq!(comparison.version, "urban-comparison-v1");
    assert_eq!(comparison.plan_digest, first.plan_digest);
    for (dir, receipt) in [(&a, &comparison.left), (&b, &comparison.right)] {
        let bytes = fs::read(dir.join("result.json")).unwrap();
        assert_eq!(receipt.result.sha256, sha256(&bytes));
        assert_eq!(receipt.result.bytes, bytes.len() as u64);
    }
    let comparison_file = temp.path().join("comparison.json");
    let compared = Command::new(env!("CARGO_BIN_EXE_laneflow-urban-harness"))
        .arg("compare")
        .arg(&a)
        .arg(&b)
        .arg(&comparison_file)
        .output()
        .unwrap();
    assert!(compared.status.success(), "{:?}", compared.stderr);
    let retained: ComparisonReport =
        serde_json::from_slice(&fs::read(&comparison_file).unwrap()).unwrap();
    assert_eq!(retained, comparison);
    assert!(comparison.write(&comparison_file).is_err());
    let diagnostics = |dir: &std::path::Path| -> serde_json::Value {
        serde_json::from_slice(&fs::read(dir.join("diagnostics.json")).unwrap()).unwrap()
    };
    assert_ne!(
        diagnostics(&a)["execution_id"],
        diagnostics(&b)["execution_id"]
    );
    assert_eq!(
        comparison.left.execution_id,
        diagnostics(&a)["execution_id"]
    );
    assert_eq!(
        comparison.right.execution_id,
        diagnostics(&b)["execution_id"]
    );
    let copied = temp.path().join("copied-a");
    fs::create_dir(&copied).unwrap();
    for entry in fs::read_dir(&a).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), copied.join(entry.file_name())).unwrap();
    }
    assert!(
        compare_runs(&a, &copied)
            .unwrap_err()
            .to_string()
            .contains("distinct execution identities")
    );
    let failed_comparison = temp.path().join("failed-comparison.json");
    let rejected = Command::new(env!("CARGO_BIN_EXE_laneflow-urban-harness"))
        .arg("compare")
        .arg(&a)
        .arg(&copied)
        .arg(&failed_comparison)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(!failed_comparison.exists());
    fs::remove_file(copied.join("diagnostics.json")).unwrap();
    assert!(compare_runs(&a, &copied).is_err());
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
