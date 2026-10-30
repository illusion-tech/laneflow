use super::*;

#[test]
fn retry_jobs_cover_four_identities_five_workers_and_three_fresh_rounds() {
    let jobs = jobs();
    assert_eq!(jobs.len(), 60);
    let unique: std::collections::BTreeSet<_> = jobs.iter().map(|job| job.key()).collect();
    assert_eq!(unique.len(), 60);
    assert_eq!(jobs.iter().filter(|job| **job == job.baseline()).count(), 4);
    for round in 1..=3 {
        for workers in WORKERS {
            assert_eq!(
                jobs.iter()
                    .filter(|job| job.round == round && job.workers == workers)
                    .count(),
                4
            );
        }
    }
    assert_eq!(jobs[20].identity(), jobs[7].identity());
    assert_eq!(jobs[20].workers, jobs[7].workers);
    assert!(jobs.iter().all(|job| {
        Task {
            job: *job,
            normal: false,
        }
        .configured_workers()
            <= MAX_CONFIGURED_WORKERS
    }));
}

fn observations_fixture() -> (Value, Value) {
    let report = json!({"failure_boundary":10,"final_tick":12,"final_cursor":7,"cursor_before_commands":2,
        "cursor_after_commands":5,"cursor_after_retry":5,"input_manifest":{"identity":{"fixed_delta_ms":16}}});
    let point = |tick, cursor, outcome| {
        json!({"tick":tick,"time_ms":tick*16,"command_cursor":cursor,
        "signals":[[1,2]],"events":[[tick,0,0,0,0,0,1,0,0,8,0]],"outcome":if outcome {
            json!({"tick":tick,"time_ms":tick*16,"parking_arrivals":[[0,1]]})
        } else { Value::Null }})
    };
    let observations = json!({"format":"lf-synth-retry-observations-1","pre_command":point(10,2,false),
        "post_command":point(10,5,false),"fresh_post_command":point(10,5,false),"failed":point(10,5,false),
        "retry":point(11,5,true),"fresh":point(11,5,true),"final":point(12,7,true)});
    (observations, report)
}

#[test]
fn boundary_observations_reject_missing_fields_wrong_times_and_failed_publication() {
    let (observations, report) = observations_fixture();
    check_observations(&observations, &report).unwrap();
    for stage in [
        "pre_command",
        "post_command",
        "fresh_post_command",
        "failed",
        "retry",
        "fresh",
        "final",
    ] {
        for (field, value) in [
            ("tick", json!(0)),
            ("time_ms", json!(0)),
            ("command_cursor", Value::Null),
            ("events", json!(true)),
            ("events", json!([[1, 2]])),
            ("signals", json!([[1, "green"]])),
        ] {
            let mut changed = observations.clone();
            changed[stage][field] = value;
            assert!(
                check_observations(&changed, &report).is_err(),
                "invalid boundary observation"
            );
        }
    }
    for stage in ["failed", "fresh_post_command", "fresh"] {
        let mut changed = observations.clone();
        changed[stage]["events"][0][1] = json!(9);
        assert!(check_observations(&changed, &report).is_err());
    }
    let mut changed = observations.clone();
    changed["retry"]["outcome"]["parking_arrivals"] = Value::Null;
    assert!(check_observations(&changed, &report).is_err());
    let mut changed = observations.clone();
    changed["failed"]["outcome"] = observations["retry"]["outcome"].clone();
    assert!(check_observations(&changed, &report).is_err());
}

#[test]
fn cross_worker_comparison_rejects_every_changed_snapshot_and_observation_file() {
    let root = tempfile::tempdir().unwrap();
    let job = Job {
        scale: 10_000,
        case: "W4",
        dt: 16,
        workers: 16,
        round: 3,
    };
    let baseline = json!({"workers":1,"error":"ParkingObservationAllocFailed","failed_calls":1});
    let mut report = baseline.clone();
    report["workers"] = json!(16);
    for config in [job, job.baseline()] {
        let target = root.path().join(config.key());
        fs::create_dir(&target).unwrap();
        for file in FILES {
            fs::write(target.join(file), [1_u8, 2, 3]).unwrap();
        }
    }
    compare(root.path(), job, &baseline, &report).unwrap();
    for field in ["error", "failed_calls", "extra_field"] {
        let mut changed = report.clone();
        changed[field] = json!("changed");
        assert!(compare(root.path(), job, &baseline, &changed).is_err());
    }
    for file in FILES {
        let path = root.path().join(job.key()).join(file);
        fs::write(&path, [1_u8, 9, 3]).unwrap();
        assert!(
            compare(root.path(), job, &baseline, &report).is_err(),
            "changed evidence bytes"
        );
        fs::remove_file(&path).unwrap();
        assert!(
            compare(root.path(), job, &baseline, &report).is_err(),
            "missing evidence file"
        );
        fs::write(&path, [1_u8, 2, 3]).unwrap();
    }
}

#[test]
fn local_retry_validation_rejects_snapshot_and_observation_tampering() {
    let root = tempfile::tempdir().unwrap();
    let (observations, report) = observations_fixture();
    for file in &FILES[..7] {
        fs::write(root.path().join(file), [1_u8, 2, 3]).unwrap();
    }
    super::super::write(&root.path().join("observations.json"), &observations).unwrap();
    check_local_files(root.path(), &report).unwrap();
    fs::write(root.path().join("failed.lfrs"), [4_u8]).unwrap();
    assert!(check_local_files(root.path(), &report).is_err());
    fs::write(root.path().join("failed.lfrs"), [1_u8, 2, 3]).unwrap();
    let mut changed = observations;
    changed["failed"]["command_cursor"] = json!(8);
    super::super::write(&root.path().join("observations.json"), &changed).unwrap();
    assert!(check_local_files(root.path(), &report).is_err());
}
