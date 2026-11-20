use super::*;

use laneflow_synth_generator::{Identity, LaneChangeParams, Provenance, generate};

#[test]
fn file_restore_preserves_all_case_suffixes_and_rejects_corruption() {
    for case in [
        Case::W1,
        Case::W2,
        Case::W3,
        Case::W4,
        // #1243 T4 回归：多簇换道输入的捕获/投影走显式 `route_rank`；
        // 旧 `logical_rank / 25` 推导把不同簇的换道者折叠进同一路线槽，
        // 首个检查点即报「捕获路线映射不一致」。
        Case::LaneChange(LaneChangeParams {
            clusters: 2,
            changers_per_cluster: 2,
        }),
    ] {
        let bundle = generate(
            Identity::new(10_000, case.clone(), 16).unwrap(),
            Provenance {
                source_commit: "a".repeat(40),
                generator_commit: "b".repeat(40),
            },
        )
        .unwrap();
        let mut plan = Plan::read(&bundle).unwrap();
        // #1197：局部回归只压缩宿主时间安排；不是正式输入窗口或短程估时探针。
        let old_warm = plan.warm_up_ticks;
        plan.warm_up_ticks = 1;
        plan.observation_ticks = if case == Case::W4 { 2 } else { 5 };
        let mut scheduled = plan
            .scheduled
            .iter()
            .cloned()
            .map(serde_json::from_value)
            .collect::<Result<Vec<Scheduled>, _>>()
            .unwrap();
        for batch in &mut scheduled {
            if batch.boundary > 0 {
                batch.boundary = batch.boundary + 1 - old_warm;
            }
        }
        let execution = ExecutionConfig::new(NonZeroU32::MIN);
        let end = plan.warm_up_ticks + plan.observation_ticks;
        let mut driver = if matches!(plan.identity.case, Case::W3 | Case::W4) {
            Driver::Burst(Box::new(Burst::new(&bundle, &plan, execution).unwrap()))
        } else {
            Driver::Simple(Box::new(Sample::new(&bundle, execution).unwrap()))
        };
        // burst 参考与生产 reference() 同式接线会话（设计 §8：恢复参考与
        // 正常参考共用准入）；恢复等值比较覆盖验证状态（不重复计数）。
        if let Driver::Burst(burst) = &mut driver {
            crate::verification::bind_burst_session(
                &mut burst.sample,
                &bundle,
                &plan.identity.case,
                end,
            )
            .unwrap();
        }
        let directory = tempfile::tempdir().unwrap();
        let baseline = directory.path().join("reference");
        fs::create_dir(&baseline).unwrap();
        let mut records = Records::new(&baseline, end, 10_000, 0, false).unwrap();
        let mut outcome = None;
        for tick in 0..=end {
            records
                .observe(&mut driver, &plan, outcome.as_ref(), end)
                .unwrap();
            for (index, (at, after)) in positions(&plan, false).into_iter().enumerate() {
                if at == tick && !after {
                    save(&driver, &records, &baseline, index, false).unwrap();
                }
            }
            if tick == end {
                break;
            }
            driver.commands(&plan, &scheduled, &mut records).unwrap();
            for (index, (at, after)) in positions(&plan, false).into_iter().enumerate() {
                if at == tick && after {
                    save(&driver, &records, &baseline, index, true).unwrap();
                }
            }
            outcome = Some(driver.step(&plan).unwrap());
        }
        records.finish(&baseline).unwrap();
        let expected = state::projection(driver.sample()).unwrap();
        let expected_host = driver.final_host();
        let expected_verification = driver.verification_final().unwrap();
        drop(driver);
        for index in 0..2 {
            let host: state::Host = serde_json::from_slice(
                &fs::read(baseline.join(format!("checkpoint-{index}.json"))).unwrap(),
            )
            .unwrap();
            let bytes = fs::read(baseline.join(format!("checkpoint-{index}.lfrs"))).unwrap();
            for workers in [1, 16] {
                let execution = ExecutionConfig::new(NonZeroU32::new(workers).unwrap());
                let mut restored = state::restore(
                    &bundle,
                    &plan,
                    end,
                    &host,
                    &bytes,
                    execution,
                    &mut json!({}),
                )
                .unwrap();
                let out = directory.path().join(format!("{index}-{workers}"));
                fs::create_dir(&out).unwrap();
                let mut suffix = Records::new(&out, end, 10_000, host.tick + 1, false).unwrap();
                suffix.journal.count = host.command_count;
                suffix.journal.blocked = host.blocked;
                for boundary in host.tick..end {
                    if boundary != host.tick || !host.after_commands {
                        restored.commands(&plan, &scheduled, &mut suffix).unwrap();
                    }
                    let outcome = restored.step(&plan).unwrap();
                    suffix
                        .observe(&mut restored, &plan, Some(&outcome), end)
                        .unwrap();
                }
                suffix.finish(&out).unwrap();
                assert_eq!(state::projection(restored.sample()).unwrap(), expected);
                assert_eq!(restored.final_host(), expected_host);
                assert_eq!(
                    restored.verification_final().unwrap(),
                    expected_verification,
                    "恢复续跑的验证会话末态须与参考逐字节等值（不重复计数、不丢覆盖）"
                );
                suffix_equal(
                    &baseline.join("ticks.bin"),
                    &out.join("ticks.bin"),
                    TRACE_HEADER,
                    host.trace_offset as usize,
                )
                .unwrap();
                suffix_equal(
                    &baseline.join("events.bin"),
                    &out.join("events.bin"),
                    EVENT_HEADER,
                    host.event_offset,
                )
                .unwrap();
                suffix_equal(
                    &baseline.join("commands.bin"),
                    &out.join("commands.bin"),
                    COMMAND_HEADER,
                    COMMAND_HEADER.len() + host.command_count as usize * 68,
                )
                .unwrap();
                assert_eq!(suffix.batches, records.batches[host.batch_count..]);
            }
            if index == 0 {
                let mut bad: state::Host =
                    serde_json::from_slice(&fs::read(baseline.join("checkpoint-0.json")).unwrap())
                        .unwrap();
                if matches!(case, Case::W3 | Case::W4) {
                    // 验证材料缺失或身份篡改：恢复拒绝（设计 §7：缺必需
                    // 检查状态/错身份不清零续跑宣布原规约通过）。
                    let original = bad.verification.clone();
                    bad.verification = None;
                    assert!(
                        state::restore(
                            &bundle,
                            &plan,
                            end,
                            &bad,
                            &bytes,
                            execution,
                            &mut json!({})
                        )
                        .is_err(),
                        "burst 检查点缺验证材料必须拒绝恢复"
                    );
                    bad.verification = bad.verification.take().map(|mut material| {
                        material["input_identity"] = json!("tampered@identity");
                        material
                    });
                    assert!(
                        state::restore(
                            &bundle,
                            &plan,
                            end,
                            &bad,
                            &bytes,
                            execution,
                            &mut json!({})
                        )
                        .is_err(),
                        "验证材料输入身份错配必须拒绝恢复"
                    );
                    bad.verification = original;
                }
                bad.source_digest[0] ^= 1;
                assert!(
                    state::restore(&bundle, &plan, end, &bad, &bytes, execution, &mut json!({}))
                        .is_err()
                );
                bad.source_digest = host.source_digest;
                bad.revision[0] ^= 1;
                assert!(
                    state::restore(&bundle, &plan, end, &bad, &bytes, execution, &mut json!({}))
                        .is_err()
                );
                bad.revision = host.revision;
                bad.vehicle_ids.swap(0, 26);
                assert!(
                    state::restore(&bundle, &plan, end, &bad, &bytes, execution, &mut json!({}))
                        .is_err()
                );
                assert!(
                    state::restore(
                        &bundle,
                        &plan,
                        end,
                        &host,
                        &bytes[..bytes.len() - 1],
                        execution,
                        &mut json!({})
                    )
                    .is_err()
                );
                assert!(
                    state::restore(
                        &bundle,
                        &plan,
                        end,
                        &host,
                        &bytes,
                        ExecutionConfig::new(NonZeroU32::new(17).unwrap()),
                        &mut json!({})
                    )
                    .is_err()
                );
            }
        }
    }
}

#[test]
fn suffix_comparison_rejects_truncation_extra_data_and_bad_offset() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    fs::write(&a, b"headabcdef").unwrap();
    fs::write(&b, b"headdef").unwrap();
    suffix_equal(&a, &b, b"head", 7).unwrap();
    for bytes in [b"headd".as_slice(), b"headdefx", b"bad_def"] {
        fs::write(&b, bytes).unwrap();
        assert!(suffix_equal(&a, &b, b"head", 7).is_err());
    }
    assert!(suffix_equal(&a, &b, b"head", 99).is_err());
    assert!(suffix_equal(&a, &b, b"head", 0).is_err());
}

#[test]
fn output_directory_is_never_overwritten_and_failure_is_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    assert!(with_output(&out, || Err(failure("injected", "failure"))).is_err());
    assert_eq!(read(&out.join("report.json")).unwrap()["complete"], false);
    assert!(with_output(&out, || Ok(json!({"complete":true}))).is_err());
    assert_eq!(read(&out.join("report.json")).unwrap()["complete"], false);
}
