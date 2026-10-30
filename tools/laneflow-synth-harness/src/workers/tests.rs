use super::*;

fn fixture(case: &'static str) -> (Job, Value, Value) {
    let job = Job {
        scale: 10_000,
        case,
        dt: 16,
        workers: 2,
        round: 1,
    };
    let input = json!({"identity":{"scale":job.scale,"case":case,"fixed_delta_ms":job.dt}});
    let (warm, observation) = job.windows();
    let ticks = if case == "W4" {
        vec![0, warm, warm + 1, warm + 2]
    } else {
        vec![
            0,
            warm,
            warm + 1_024,
            warm + 2_048,
            warm + 4_096,
            warm + observation,
        ]
    };
    let points: Vec<_> = ticks
        .into_iter()
        .map(|tick| json!({"tick":tick,"state_digest":"a".repeat(64)}))
        .collect();
    let format = if matches!(case, "W3" | "W4") {
        "lf-synth-burst-run-1"
    } else {
        "lf-synth-semantic-run-1"
    };
    let report = json!({"format":format,"complete":true,"implementation_commit":"c".repeat(40),
        "workers":job.workers,"input_manifest":input,"warm_up_ticks":warm,"observation_ticks":observation,
        "validated_boundaries":warm+observation+1,"checkpoints":points});
    (job, input, report)
}

#[test]
fn nominal_jobs_have_three_fresh_rounds_and_rotated_worker_order() {
    let jobs = jobs();
    assert_eq!(jobs.len(), 120);
    let unique: std::collections::BTreeSet<_> = jobs.iter().map(|job| job.key()).collect();
    assert_eq!(unique.len(), 120);
    assert!(
        jobs.iter()
            .all(|job| matches!((job.scale, job.dt), (10_000, 16) | (100_000, 33)))
    );
    for round in 1..=3 {
        let rows: Vec<_> = jobs.iter().filter(|job| job.round == round).collect();
        assert_eq!(rows.len(), 40);
        for workers in WORKERS {
            assert_eq!(rows.iter().filter(|job| job.workers == workers).count(), 8);
        }
    }
    assert_eq!(
        jobs[40],
        Job {
            round: 2,
            ..jobs[7]
        }
    );
    assert_eq!(
        jobs[80],
        Job {
            round: 3,
            ..jobs[14]
        }
    );
    assert_eq!(jobs.iter().filter(|job| **job == job.baseline()).count(), 8);
}

#[test]
fn receipts_reject_wrong_source_identity_workers_and_incomplete_windows() {
    for case in ["W1", "W2", "W3", "W4"] {
        let (job, input, report) = fixture(case);
        let implementation = "c".repeat(40);
        check_report(job, &report, &input, &implementation).unwrap();
        for (field, value) in [
            ("complete", json!(false)),
            ("implementation_commit", json!("d".repeat(40))),
            ("workers", json!(1)),
            ("workers", Value::Null),
            ("format", json!("other")),
            ("warm_up_ticks", json!(512)),
            ("observation_ticks", json!(1)),
            ("validated_boundaries", json!(1)),
            ("checkpoints", json!([])),
        ] {
            let mut changed = report.clone();
            changed[field] = value;
            assert!(
                check_report(job, &changed, &input, &implementation).is_err(),
                "invalid receipt field"
            );
        }
        let mut changed = report.clone();
        changed["input_manifest"]["identity"]["scale"] = json!(100_000);
        assert!(check_report(job, &changed, &input, &implementation).is_err());
        let wrong_input = changed["input_manifest"].clone();
        assert!(check_report(job, &changed, &wrong_input, &implementation).is_err());
        let mut changed = report.clone();
        changed["checkpoints"][0]["state_digest"] = json!("invalid");
        assert!(check_report(job, &changed, &input, &implementation).is_err());
    }
}

#[test]
fn cross_worker_comparison_ignores_only_workers_and_checks_all_exact_files() {
    for case in ["W1", "W3", "W4"] {
        let root = tempfile::tempdir().unwrap();
        let (job, _, candidate) = fixture(case);
        let mut baseline = candidate.clone();
        baseline["workers"] = json!(1);
        for config in [job, job.baseline()] {
            let directory = root.path().join(config.key());
            fs::create_dir(&directory).unwrap();
            for file in config.files() {
                fs::write(directory.join(file), [1_u8, 2, 3]).unwrap();
            }
        }
        compare(root.path(), job, &baseline, &candidate).unwrap();
        for field in ["implementation_commit", "checkpoints", "extra_metadata"] {
            let mut changed = candidate.clone();
            changed[field] = json!("changed");
            assert!(compare(root.path(), job, &baseline, &changed).is_err());
        }
        for file in job.files() {
            let path = root.path().join(job.key()).join(file);
            fs::write(&path, [1_u8, 9, 3]).unwrap();
            assert!(
                compare(root.path(), job, &baseline, &candidate).is_err(),
                "changed evidence bytes"
            );
            fs::remove_file(&path).unwrap();
            assert!(
                compare(root.path(), job, &baseline, &candidate).is_err(),
                "missing evidence file"
            );
            fs::write(&path, [1_u8, 2, 3]).unwrap();
        }
    }
}
