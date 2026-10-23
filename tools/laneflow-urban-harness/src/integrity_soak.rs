//! #963：有限城市浸泡仪器，只进入测试二进制。性能计时与产品保证不在本仪器范围内。
//! 每段从已写后读回的宿主存档恢复；在用原世界留在父进程，恢复只发生于子进程。

#[path = "../../../crates/laneflow-runtime/tests/support/audit_replay.rs"]
#[allow(dead_code)]
mod audit;

use std::{
    fs,
    num::NonZeroU32,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use laneflow_format::FormatLimits;
use laneflow_runtime::{
    ExecutionConfig, PublishedLfcaReference, SnapshotDigestLimits,
    attest_published_shared_network_revision,
};
use laneflow_static_contract::{ExactByteLength, NetworkRevisionId, Sha256Digest};
use laneflow_static_network::{
    AttestationOutcome, AttestationVerdict, SharedNetworkBuildLimits, SharedNetworkBuildOptions,
    SpatialBuildOption,
};
use laneflow_urban_generator::{Scale, UrbanConfig, generate, verified_write};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Artifacts, Harness, ResolvedPlan, Result, SaveSource, UrbanCase, Window, invalid};

const BATCH_TICKS: u64 = 1_024;
const STRIDE: u64 = 64;
const MAX_SECONDS: u64 = 1_800;
const MAX_DISK_BYTES: u64 = 512 * 1_024 * 1_024;

#[derive(Clone, Serialize, Deserialize)]
struct SourceIdentity {
    revision: [u8; 32],
    digest: [u8; 32],
    length: u64,
}

impl SourceIdentity {
    fn pin(artifacts: &Artifacts) -> Result<Self> {
        // 期望来自已核对的发布目录与文件登记，不从被认证修订的 origin 反向提取。
        let file = artifacts
            .files
            .get("network.lfca")
            .ok_or_else(|| invalid("缺少 LFCA 文件登记"))?;
        let decode = |value: &str| -> Result<[u8; 32]> {
            if value.len() != 64 || !value.is_ascii() {
                return Err(invalid("发布身份不是 32 字节十六进制值"));
            }
            let mut bytes = [0; 32];
            for (index, byte) in bytes.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&value[2 * index..2 * index + 2], 16)
                    .map_err(|error| invalid(format!("发布身份解析失败：{error}")))?;
            }
            Ok(bytes)
        };
        Ok(Self {
            revision: decode(&artifacts.catalog.network_revision)?,
            digest: decode(&file.sha256)?,
            length: file.bytes,
        })
    }

    fn reference(&self) -> PublishedLfcaReference {
        PublishedLfcaReference::new(
            "integrity-soak://network.lfca",
            Sha256Digest::from_bytes(self.digest),
            ExactByteLength::new(self.length),
            NetworkRevisionId::from_digest(Sha256Digest::from_bytes(self.revision)),
        )
        .expect("pinned nonempty published source")
    }
}

#[derive(Serialize, Deserialize)]
struct Request {
    source: PathBuf,
    plan: PathBuf,
    save: PathBuf,
    target_tick: u64,
    identity: SourceIdentity,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Sample {
    ordinal: u32,
    command_cursor: u64,
    tick: u64,
    digest: [u8; 32],
    caller_digest: String,
}

#[derive(Serialize, Deserialize)]
struct Response {
    pid: u32,
    root_attestation: String,
    samples: Vec<Sample>,
    tick_records_digest: String,
    last_state_digest: Option<String>,
    environment_probe_rewrites: u32,
}

fn execution() -> ExecutionConfig {
    ExecutionConfig::new(NonZeroU32::MIN)
}

fn write(path: &Path, bytes: &[u8]) -> Result<u32> {
    verified_write::write_verified(
        path,
        bytes,
        &verified_write::SourceRegistration::of(bytes),
        verified_write::WritePolicy::DEFAULT,
    )
    .map(|result| result.rewrites)
    .map_err(|error| invalid(format!("write-readback {}: {error}", path.display())))
}

// 环境对照复用 #954 的双路径读回，不另写重试机制；内容由进程独立生成。
// 此处只检查文件可见字节，不声称绕过系统缓存或证明物理持久化。
fn probe_bytes(tick: u64) -> Vec<u8> {
    (0_u64..65_536)
        .map(|offset| (offset.wrapping_mul(131) ^ (offset / 4_096) ^ tick) as u8)
        .collect()
}

fn attest(artifacts: &Artifacts, source: &Path, identity: &SourceIdentity) -> Result<String> {
    let bytes = fs::read(source.join("network.lfca"))?;
    let outcome = attest_published_shared_network_revision(
        artifacts.revision(),
        &identity.reference(),
        bytes.as_slice(),
        FormatLimits::HARD,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(2_147_483_648, 2_147_483_648),
        ),
        None,
    );
    match &outcome {
        AttestationOutcome::Concluded(report) if report.verdict == AttestationVerdict::Pass => {
            Ok(format!("{outcome:?}"))
        }
        _ => Err(invalid(format!("root attestation: {outcome:?}"))),
    }
}

fn sample(harness: &Harness<'_>, ordinal: u32) -> Result<Sample> {
    let digest = crate::checked(
        "direct digest",
        harness
            .world()
            .direct_state_digest(SnapshotDigestLimits::new(64 * 1_024 * 1_024)),
    )?;
    Ok(Sample {
        ordinal,
        command_cursor: harness.world().command_cursor(),
        tick: harness.world().tick_index(),
        digest: *digest.as_bytes(),
        caller_digest: harness.caller_digest()?,
    })
}

fn advance(
    harness: &mut Harness<'_>,
    target_tick: u64,
    deadline: Option<Instant>,
    pace: bool,
) -> Result<Response> {
    let mut samples = vec![sample(harness, 0)?];
    let mut ticks = Sha256::new();
    let mut last_state_digest = None;
    while harness.world().tick_index() < target_tick {
        if deadline.is_some_and(|end| Instant::now() >= end) {
            break;
        }
        let started = Instant::now();
        let record = harness.advance()?;
        let encoded = serde_json::to_vec(&record)?;
        ticks.update((encoded.len() as u64).to_le_bytes());
        ticks.update(encoded);
        last_state_digest = Some(record.state_digest.clone());
        if record.tick.is_multiple_of(STRIDE) {
            samples.push(sample(harness, samples.len() as u32)?);
        }
        if pace {
            thread::sleep(Duration::from_millis(16).saturating_sub(started.elapsed()));
        }
    }
    if samples.last().expect("start").tick != harness.world().tick_index() {
        samples.push(sample(harness, samples.len() as u32)?);
    }
    Ok(Response {
        pid: std::process::id(),
        root_attestation: String::new(),
        samples,
        tick_records_digest: crate::hex(&ticks.finalize()),
        last_state_digest,
        environment_probe_rewrites: 0,
    })
}

fn trace(response: &Response) -> audit::Trace {
    audit::Trace {
        points: response
            .samples
            .iter()
            .map(|sample| audit::Point {
                ordinal: sample.ordinal,
                position: audit::Position {
                    command_cursor: sample.command_cursor,
                    tick: sample.tick,
                },
                digest: sample.digest,
            })
            .collect(),
        failure: None,
    }
}

fn compare(expected: &Response, actual: &Response) -> Result<audit::AuditResult> {
    let target = trace(expected)
        .points
        .last()
        .ok_or_else(|| invalid("expected trace has no target"))?
        .position;
    Ok(audit::compare(
        &trace(expected),
        &trace(actual),
        target,
        None,
    ))
}

fn worker(request: &Path) -> Result<()> {
    let request_value: Request = serde_json::from_slice(&fs::read(request)?)?;
    let probe = probe_bytes(request_value.target_tick);
    let readback = fs::read(request.with_extension("parent-probe.bin"))?;
    if readback != probe {
        let first = readback.iter().zip(&probe).position(|(a, b)| a != b);
        return Err(invalid(format!(
            "环境对照读回失配：首偏移 {first:?}，长度 {} / {}；不得直接归因 Runtime 堆",
            readback.len(),
            probe.len()
        )));
    }
    let probe_rewrites = write(&request.with_extension("child-probe.bin"), &probe)?;
    let artifacts = Artifacts::load(&request_value.source)?;
    // 必须先认证，之后才恢复；同 world_id 原世界仍存活于父进程。
    let root_attestation = attest(&artifacts, &request_value.source, &request_value.identity)?;
    let plan = ResolvedPlan::read(&request_value.plan)?;
    let loaded = crate::save::load_save(&request_value.save, &artifacts, &plan.digest()?)?;
    let mut harness = Harness::resume(&artifacts, &plan, execution(), loaded)?;
    let mut response = advance(&mut harness, request_value.target_tick, None, false)?;
    response.root_attestation = root_attestation;
    response.environment_probe_rewrites = probe_rewrites;
    write(
        &request.with_extension("response.json"),
        &serde_json::to_vec_pretty(&response)?,
    )?;
    Ok(())
}

#[test]
#[ignore = "独立审计子进程；只由有限浸泡父进程调用"]
fn replay_worker() {
    worker(&PathBuf::from(
        std::env::var_os("LF_SOAK_REQUEST").expect("request"),
    ))
    .expect("independent audit worker");
}

fn replay(request: &Path) -> Result<Response> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args([
            "--ignored",
            "--exact",
            "integrity_soak::replay_worker",
            "--nocapture",
        ])
        .env("LF_SOAK_REQUEST", request)
        .stdout(Stdio::from(fs::File::create(
            request.with_extension("stdout.log"),
        )?))
        .stderr(Stdio::from(fs::File::create(
            request.with_extension("stderr.log"),
        )?));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command.spawn()?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                return Err(invalid(format!("Incomplete: replay child {status}")));
            }
            break;
        }
        if started.elapsed() > Duration::from_secs(30) {
            child.kill()?;
            child.wait()?;
            return Err(invalid("Incomplete: replay child timeout"));
        }
        thread::sleep(Duration::from_millis(20));
    }
    let path = request.with_extension("response.json");
    if fs::metadata(&path)?.len() > 128 * 1_024 {
        return Err(invalid("Incomplete: oversized replay response"));
    }
    let response: Response = serde_json::from_slice(&fs::read(path)?)?;
    if response.pid == std::process::id() || response.root_attestation.is_empty() {
        return Err(invalid(
            "Incomplete: independent authenticated child missing",
        ));
    }
    Ok(response)
}

fn directory_bytes(path: &Path) -> Result<u64> {
    let mut total = 0;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        total += if entry.file_type()?.is_dir() {
            directory_bytes(&entry.path())?
        } else {
            entry.metadata()?.len()
        };
    }
    Ok(total)
}

fn run(output: &Path, seconds: u64, pace: bool) -> Result<()> {
    if seconds == 0 || seconds > MAX_SECONDS || output.exists() {
        return Err(invalid("require fresh output and 1..=1800 seconds"));
    }
    fs::create_dir_all(output)?;
    let source = output.join("source");
    let config = crate::checked(
        "city config",
        UrbanConfig::parse(include_str!("../../../examples/config/cn-urban.toml")),
    )?;
    crate::checked(
        "generate city",
        generate(&config, Scale::Fixture, &source, None),
    )?;
    let artifacts = Artifacts::load(&source)?;
    let identity = SourceIdentity::pin(&artifacts)?;
    let plan = ResolvedPlan::for_case(
        &artifacts,
        UrbanCase::SustainedActive,
        Window {
            purpose: "integrity-soak".into(),
            warm_up_ticks: 0,
            observation_ticks: 200_000,
        },
    )?;
    plan.validate(&artifacts)?;
    let plan_path = output.join("plan.toml");
    write(&plan_path, toml::to_string_pretty(&plan)?.as_bytes())?;
    let initial_attestation = attest(&artifacts, &source, &identity)?;
    let mut harness = Harness::install(&artifacts, &plan, execution())?;
    // 存档沿用 advance 返回的观测摘要；不重复调用带宿主观测副作用的 state。
    let mut state_digest = harness.advance()?.state_digest;
    let initial_tick = harness.world().tick_index();
    let started = Instant::now();
    let deadline = started + Duration::from_secs(seconds);
    let mut batches = 0;
    let mut covered_ticks = 0;
    while Instant::now() < deadline {
        if directory_bytes(output)? > MAX_DISK_BYTES {
            return Err(invalid("Incomplete: 512 MiB evidence limit"));
        }
        let start_tick = harness.world().tick_index();
        let path = output.join(format!("batch-{batches:04}.json"));
        let result_path = path.with_extension("result.json");
        write(
            &result_path,
            br#"{"classification":"Incomplete","reason":"audit pending"}"#,
        )?;
        let (save, _) = crate::save::write_save(
            output,
            &harness,
            &state_digest,
            &plan.digest()?,
            SaveSource {
                execution_id: "integrity-soak".into(),
                git_commit: std::env::var("LF_SOAK_COMMIT").ok(),
                binary_sha256: std::env::var("LF_SOAK_BINARY_SHA256").ok(),
                workers: 1,
            },
        )?;
        let mut expected = advance(&mut harness, start_tick + BATCH_TICKS, Some(deadline), pace)?;
        if let Some(last_state) = &expected.last_state_digest {
            state_digest.clone_from(last_state);
        }
        expected.root_attestation = attest(&artifacts, &source, &identity)?;
        let request = Request {
            source: source.clone(),
            plan: plan_path.clone(),
            save: output.join(save.path),
            target_tick: harness.world().tick_index(),
            identity: identity.clone(),
        };
        expected.environment_probe_rewrites = write(
            &path.with_extension("parent-probe.bin"),
            &probe_bytes(request.target_tick),
        )?;
        write(
            &path.with_extension("expected.json"),
            &serde_json::to_vec_pretty(&expected)?,
        )?;
        write(&path, &serde_json::to_vec_pretty(&request)?)?;
        // 本段尚未取得独立结果；进程被终止时仍保留 Incomplete，不借用前段结论。
        let actual = replay(&path)?;
        let result = compare(&expected, &actual)?;
        let host_equal = expected.tick_records_digest == actual.tick_records_digest
            && expected
                .samples
                .iter()
                .zip(&actual.samples)
                .all(|(a, b)| a.caller_digest == b.caller_digest);
        let classification = match result {
            audit::AuditResult::CompleteMatch if host_equal => "CompleteMatch",
            audit::AuditResult::CompleteMatch | audit::AuditResult::Diverged { .. } => "Diverged",
            audit::AuditResult::Incomplete { .. } => "Incomplete",
        };
        let record = serde_json::json!({
            "classification": classification, "runtime_result": format!("{result:?}"),
            "host_equal": host_equal, "start_tick": start_tick, "target_tick": request.target_tick,
            "points": expected.samples.len(), "elapsed_seconds": started.elapsed().as_secs_f64(),
            "parent_pid": std::process::id(), "child_pid": actual.pid,
            "environment_probe_rewrites": [expected.environment_probe_rewrites, actual.environment_probe_rewrites],
        });
        write(&result_path, &serde_json::to_vec_pretty(&record)?)?;
        println!("SOAK {record}");
        if classification != "CompleteMatch" {
            return Err(invalid(format!("audit stopped: {record}")));
        }
        batches += 1;
        covered_ticks += request.target_tick - start_tick;
        if !pace {
            break;
        }
    }
    let summary = serde_json::json!({
        "status": "completed", "scope": "short-soak-chain-validation", "requested_seconds": seconds,
        "elapsed_seconds": started.elapsed().as_secs_f64(), "complete_match_batches": batches,
        "covered_ticks": covered_ticks, "final_tick": harness.world().tick_index(),
        "initial_tick": initial_tick,
        "initial_attestation": initial_attestation, "runtime_workers": 1, "paced": pace,
        "nominal_individuals": artifacts.individuals, "replacements": harness.replacements,
        "evidence_bytes": directory_bytes(output)?, "uncovered": ["long-term fault-host acceptance", "physical heap attribution", "between-checkpoint transient changes"],
    });
    write(
        &output.join("summary.json"),
        &serde_json::to_vec_pretty(&summary)?,
    )?;
    println!("SOAK_SUMMARY {summary}");
    Ok(())
}

#[test]
fn short_chain_replays_in_an_independent_process() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("short");
    run(&output, 30, false).unwrap();
    // 真正启动失败子进程：修订期望值失真时不得恢复，也不得沿用旧成功响应。
    let original = output.join("batch-0000.json");
    let mut request: Request = serde_json::from_slice(&fs::read(&original).unwrap()).unwrap();
    request.identity.revision[0] ^= 0x80;
    let rejected = output.join("rejected.json");
    write(&rejected, &serde_json::to_vec(&request).unwrap()).unwrap();
    write(
        &rejected.with_extension("parent-probe.bin"),
        &probe_bytes(request.target_tick),
    )
    .unwrap();
    assert!(replay(&rejected).is_err());
    assert!(!rejected.with_extension("response.json").exists());
    let stderr = fs::read_to_string(rejected.with_extension("stderr.log")).unwrap();
    assert!(
        stderr.contains("root attestation"),
        "must fail at authentication"
    );
}

#[test]
#[ignore = "需要用户指定浸泡时长和资源窗口，不参与默认测试"]
fn bounded_soak() {
    let output = PathBuf::from(std::env::var_os("LF_SOAK_OUTPUT").expect("output"));
    let seconds = std::env::var("LF_SOAK_SECONDS")
        .expect("seconds")
        .parse()
        .expect("integer seconds");
    let result = run(&output, seconds, true);
    if let Err(error) = &result {
        let _ = write(
            &output.join("failure.json"),
            &serde_json::to_vec_pretty(&serde_json::json!({
                "status": "Incomplete", "reason": error.to_string(),
            }))
            .expect("failure JSON"),
        );
    }
    result.expect("bounded soak");
}
