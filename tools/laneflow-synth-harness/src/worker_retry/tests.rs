use super::*;

#[test]
fn retry_jobs_cover_identities_failpoints_five_workers_and_three_fresh_rounds() {
    let jobs = jobs();
    assert_eq!(jobs.len(), 120);
    let unique: std::collections::BTreeSet<_> = jobs
        .iter()
        .map(|(job, inject)| (job.key(), *inject))
        .collect();
    assert_eq!(unique.len(), 120);
    // 每（身份×注入断点）组合的第一轮单 worker 即比较基线：4 W + 4 L = 8。
    assert_eq!(
        jobs.iter()
            .filter(|(job, _inject)| *job == job.baseline())
            .count(),
        8
    );
    for round in 1..=3 {
        for workers in WORKERS {
            assert_eq!(
                jobs.iter()
                    .filter(|(job, _inject)| job.round == round && job.workers == workers)
                    .count(),
                8
            );
        }
    }
    // 轮内左移 7（#1218 D6 网格扩展后口径不变）。
    assert_eq!(jobs[40].0.identity(), jobs[7].0.identity());
    assert_eq!(jobs[40].0.workers, jobs[7].0.workers);
    assert_eq!(jobs[40].1, jobs[7].1);
    assert!(jobs.iter().all(|(job, inject)| {
        Task {
            job: *job,
            normal: false,
            inject,
        }
        .configured_workers()
            <= MAX_CONFIGURED_WORKERS
    }));
    // L 族注入断点恰覆盖换道预约相关断点（D6）。
    assert!(
        jobs.iter()
            .filter(|(_, inject)| !inject.is_empty())
            .all(|(_, inject)| *inject == "AfterGrants" || *inject == "AfterTransitions")
    );
}

#[test]
fn retry_tasks_have_six_normal_references_before_retries() {
    // 回归（#1218 正式运行死锁）：前 6 个任务必须是正常参考（W3/W4 × 2
    // 尺度 + L1 × 2 尺度），否则 `normals.len() == 6` 闸门不开启，
    // 120 个重试永不启动。
    let tasks = tasks();
    assert_eq!(tasks.len(), 126);
    assert_eq!(tasks.iter().filter(|task| task.normal).count(), 6);
    assert!(tasks[..6].iter().all(|task| task.normal));
    assert!(tasks[6..].iter().all(|task| !task.normal));
    let mut normal_identities: Vec<String> =
        tasks[..6].iter().map(|task| task.job.identity()).collect();
    normal_identities.sort();
    let expected: Vec<String> = [
        "10000-L1-16",
        "10000-W3-16",
        "10000-W4-16",
        "100000-L1-33",
        "100000-W3-33",
        "100000-W4-33",
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    assert_eq!(normal_identities, expected);
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
    let task = Task {
        job,
        normal: false,
        inject: "",
    };
    let baseline = json!({"workers":1,"error":"ParkingObservationAllocFailed","failed_calls":1});
    let mut report = baseline.clone();
    report["workers"] = json!(16);
    for config in [
        task,
        Task {
            job: job.baseline(),
            ..task
        },
    ] {
        let target = root.path().join(config.key());
        fs::create_dir(&target).unwrap();
        for file in FILES {
            fs::write(target.join(file), [1_u8, 2, 3]).unwrap();
        }
    }
    compare(root.path(), task, &baseline, &report).unwrap();
    for field in ["error", "failed_calls", "extra_field"] {
        let mut changed = report.clone();
        changed[field] = json!("changed");
        assert!(compare(root.path(), task, &baseline, &changed).is_err());
    }
    for file in FILES {
        let path = root.path().join(task.key()).join(file);
        fs::write(&path, [1_u8, 9, 3]).unwrap();
        assert!(
            compare(root.path(), task, &baseline, &report).is_err(),
            "changed evidence bytes"
        );
        fs::remove_file(&path).unwrap();
        assert!(
            compare(root.path(), task, &baseline, &report).is_err(),
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

#[test]
fn lane_change_final_state_projection_is_compared_against_normal_reference() {
    // #1226：末态比较必须覆盖全体 handle 的公开车辆状态投影，缺失或差异均拒绝。
    let identity = "10000-L1-16";
    let normal = json!({
        "implementation_commit": "a".repeat(40),
        "input_manifest": {"identity": {"case": "L1"}},
        "warm_up_ticks": 16,
        "observation_ticks": 304,
        "checkpoints": [
            {"tick": 0, "state_digest": "d", "t_state": null, "persistence_refused": null},
            {"tick": 320, "persistence_refused": "AdmissionHopWithoutGate",
             "t_state": {"journal": [[1, 0, 4, 0]], "states": [[0, 0, 0], [1, 0, 0, 0]]}}
        ]
    });
    let report = json!({
        "format": L_FORMAT,
        "complete": true,
        "implementation_commit": "a".repeat(40),
        "input_manifest": {"identity": {"case": "L1"}},
        "warm_up_ticks": 16,
        "observation_ticks": 304,
        "workers": 1,
        "zero_publication": true,
        "retry_equals_fresh": true,
        "failpoint_consumed": true,
        "final_face_b": {
            "journal": [[1, 0, 4, 0]],
            "states": [[0, 0, 0], [1, 0, 0, 0]],
        }
    });
    verify_lane_change_reports(&report, &normal, identity, 1).unwrap();
    let mut changed_states = report.clone();
    changed_states["final_face_b"]["states"] = json!([[0, 0, 0], [1, 0, 0, 9]]);
    assert!(
        verify_lane_change_reports(&changed_states, &normal, identity, 1).is_err(),
        "全体车辆状态投影差异必须拒绝"
    );
    let mut changed_journal = report.clone();
    changed_journal["final_face_b"]["journal"] = json!([[1, 0, 5, 0]]);
    assert!(
        verify_lane_change_reports(&changed_journal, &normal, identity, 1).is_err(),
        "末态历史差异必须拒绝"
    );
}
