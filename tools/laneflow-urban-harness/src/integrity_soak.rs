//! #963：有界城市浸泡仪器，只进入测试二进制。性能计时与产品保证不在本仪器范围内。
//! 每段从已写后读回的宿主存档恢复；在用原世界留在父进程，恢复只发生于子进程。

use std::{
    fs,
    num::NonZeroU32,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

mod control;
mod supervisor;
mod tests;

use control::{Clock, Controller};

use laneflow_audit_test_support as audit;
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

use crate::{Artifacts, Harness, ResolvedPlan, Result, SaveSource, UrbanCase, Window, invalid};

const BATCH_TICKS: u64 = 1_024;
const STRIDE: u64 = 64;
const MAX_SECONDS: u64 = 7 * 24 * 60 * 60;
pub(super) const MAX_TICKS: u64 = MAX_SECONDS * 1_000 / 16 + 4_098;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    seconds: u64,
    wall_seconds: u64,
    max_disk_bytes: u64,
    max_memory_bytes: u64,
    batch_ticks: u64,
    replay_seconds: u64,
    paced: bool,
}

impl Config {
    fn new(seconds: u64) -> Self {
        Self {
            seconds,
            wall_seconds: seconds.saturating_mul(4).saturating_add(300),
            max_disk_bytes: 2 * 1_024 * 1_024 * 1_024,
            max_memory_bytes: 2 * 1_024 * 1_024 * 1_024,
            batch_ticks: BATCH_TICKS,
            replay_seconds: 30,
            paced: true,
        }
    }

    fn validate(&self) -> Result<()> {
        if !(1..=MAX_SECONDS).contains(&self.seconds)
            || !(self.seconds..=30 * 24 * 60 * 60).contains(&self.wall_seconds)
            || !(1..=64 * 1_024 * 1_024 * 1_024).contains(&self.max_disk_bytes)
            || !(1..=64 * 1_024 * 1_024 * 1_024).contains(&self.max_memory_bytes)
            || !(1..=4_096).contains(&self.batch_ticks)
            || !(1..=120).contains(&self.replay_seconds)
        {
            return Err(invalid("浸泡配置超出有界范围"));
        }
        Ok(())
    }

    fn from_env() -> Result<Self> {
        let seconds = std::env::var("LF_SOAK_SECONDS")
            .map_err(|error| invalid(format!("LF_SOAK_SECONDS: {error}")))?
            .parse()
            .map_err(|error| invalid(format!("LF_SOAK_SECONDS: {error}")))?;
        let mut config = Self::new(seconds);
        for (key, value) in [
            ("LF_SOAK_WALL_SECONDS", &mut config.wall_seconds),
            ("LF_SOAK_MAX_DISK_BYTES", &mut config.max_disk_bytes),
            ("LF_SOAK_MAX_MEMORY_BYTES", &mut config.max_memory_bytes),
            ("LF_SOAK_BATCH_TICKS", &mut config.batch_ticks),
            ("LF_SOAK_REPLAY_SECONDS", &mut config.replay_seconds),
        ] {
            match std::env::var(key) {
                Ok(raw) => {
                    *value = raw
                        .parse()
                        .map_err(|error| invalid(format!("{key}: {error}")))?
                }
                Err(std::env::VarError::NotPresent) => {}
                Err(error) => return Err(invalid(format!("{key}: {error}"))),
            }
        }
        config.validate()?;
        Ok(config)
    }
}

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Sample {
    ordinal: u32,
    command_cursor: u64,
    tick: u64,
    world_id: u64,
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
        world_id: harness.world().world_id(),
        digest: *digest.as_bytes(),
        caller_digest: harness.caller_digest()?,
    })
}

fn advance(
    harness: &mut Harness<'_>,
    target_tick: u64,
    deadline: Option<Instant>,
    pace: bool,
    control: Option<&Controller>,
) -> Result<Response> {
    let mut samples = vec![sample(harness, 0)?];
    let mut ticks = crate::digest::Domain::SoakTicks.hasher();
    let mut last_state_digest = None;
    while harness.world().tick_index() < target_tick {
        if deadline.is_some_and(|end| Instant::now() >= end) {
            break;
        }
        if let Some(control) = control {
            control.check_supervisor()?;
            if control.pending()?.is_some() {
                break;
            }
        }
        let started = Instant::now();
        let record = harness.advance()?;
        let encoded = serde_json::to_vec(&record)?;
        ticks.update(&(encoded.len() as u64).to_le_bytes());
        ticks.update(&encoded);
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
        tick_records_digest: crate::hex(ticks.finalize().as_bytes()),
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
    let mut response = advance(&mut harness, request_value.target_tick, None, false, None)?;
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

fn replay(request: &Path, timeout: Duration) -> Result<Response> {
    replay_entry(request, timeout, "integrity_soak::replay_worker")
}

fn replay_entry(request: &Path, timeout: Duration, entry: &str) -> Result<Response> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["--ignored", "--exact", entry, "--nocapture"])
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
    let mut child = supervisor::ChildGuard::new(command.spawn()?);
    let child_pid = child.child.id();
    let started = Instant::now();
    loop {
        if let Some(status) = child.child.try_wait()? {
            if !status.success() {
                return Err(invalid(format!("Incomplete: replay child {status}")));
            }
            break;
        }
        if started.elapsed() > timeout {
            child.terminate()?;
            return Err(invalid("Incomplete: replay child timeout"));
        }
        thread::sleep(Duration::from_millis(20));
    }
    child.terminate()?;
    let path = request.with_extension("response.json");
    if fs::metadata(&path)?.len() > 128 * 1_024 {
        return Err(invalid("Incomplete: oversized replay response"));
    }
    let response: Response = serde_json::from_slice(&fs::read(path)?)?;
    if response.pid != child_pid
        || response.pid == std::process::id()
        || response.root_attestation.is_empty()
    {
        return Err(invalid(
            "Incomplete: independent authenticated child missing",
        ));
    }
    Ok(response)
}

fn directory_bytes(path: &Path) -> Result<u64> {
    let mut total = 0_u64;
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        let kind = match entry.file_type() {
            Ok(kind) => kind,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let bytes = if kind.is_dir() {
            directory_bytes(&entry.path())?
        } else {
            match entry.metadata() {
                Ok(metadata) => metadata.len(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            }
        };
        total = total
            .checked_add(bytes)
            .ok_or_else(|| invalid("证据字节数溢出"))?;
    }
    Ok(total)
}

fn run(output: &Path, seconds: u64, pace: bool) -> Result<()> {
    let mut config = Config::new(seconds);
    config.paced = pace;
    let spec = supervisor::create_output(output, config)?;
    let result = run_in_created_directory(output, &spec, false);
    record_failure(output, &result);
    result
}

fn record_failure(output: &Path, result: &Result<()>) {
    if let Err(error) = result {
        let value = serde_json::json!({
            "status":"Incomplete", "reason":error.to_string(), "pid":std::process::id(),
        });
        // 驻留进程与看护进程的原始失败分别保留；顶层记录只作最近失败入口。
        let original = output
            .join("failures")
            .join(format!("{}.json", std::process::id()));
        if !original.exists() {
            let _ = control::publish(&original, &value);
        }
        let _ = control::publish(&output.join("failure.json"), &value);
    }
}

fn run_in_created_directory(
    output: &Path,
    spec: &supervisor::Spec,
    supervised: bool,
) -> Result<()> {
    let config = &spec.config;
    let mut control = Controller::new(output, spec, supervised)?;
    let source = output.join("source");
    let city_config = crate::checked(
        "city config",
        UrbanConfig::parse(include_str!("../../../examples/config/cn-urban.toml")),
    )?;
    crate::checked(
        "generate city",
        generate(&city_config, Scale::Fixture, &source, None),
    )?;
    let artifacts = Artifacts::load(&source)?;
    let identity = SourceIdentity::pin(&artifacts)?;
    let plan = ResolvedPlan::for_case(
        &artifacts,
        UrbanCase::SustainedActive,
        Window {
            purpose: "integrity-soak".into(),
            warm_up_ticks: 0,
            observation_ticks: (config.seconds * 1_000 / 16 + config.batch_ticks + 2).max(2),
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
    let mut clock = Clock::new();
    let mut batches = 0;
    let mut covered_ticks = 0;
    let mut stopped = false;
    control.status("Running", &harness, &clock)?;
    while clock.active() < Duration::from_secs(config.seconds) {
        control.check_supervisor()?;
        if control.boundary(&harness, &artifacts, &source, &identity, &mut clock)? {
            stopped = true;
            break;
        }
        if directory_bytes(output)? > config.max_disk_bytes {
            return Err(invalid("证据容量超限（Incomplete）"));
        }
        let segment = output.join("segments").join(format!("{batches:06}"));
        fs::create_dir(&segment)?;
        let start_tick = harness.world().tick_index();
        let path = segment.join("request.json");
        let result_path = path.with_extension("result.json");
        control::publish(
            &result_path,
            &serde_json::json!({"classification":"Incomplete","reason":"audit pending"}),
        )?;
        let (save, _) = crate::save::write_save(
            &segment,
            &harness,
            &state_digest,
            &plan.digest()?,
            SaveSource {
                execution_id: "integrity-soak".into(),
                git_commit: std::env::var("LF_SOAK_COMMIT").ok(),
                binary_digest: std::env::var("LF_SOAK_BINARY_DIGEST").ok(),
                workers: 1,
            },
        )?;
        let deadline =
            Instant::now() + Duration::from_secs(config.seconds).saturating_sub(clock.active());
        let mut expected = advance(
            &mut harness,
            start_tick + config.batch_ticks,
            Some(deadline),
            config.paced,
            Some(&control),
        )?;
        if let Some(last_state) = &expected.last_state_digest {
            state_digest.clone_from(last_state);
        }
        expected.root_attestation = attest(&artifacts, &source, &identity)?;
        let request = Request {
            source: source.clone(),
            plan: plan_path.clone(),
            save: segment.join(save.path),
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
        let actual = replay(&path, Duration::from_secs(config.replay_seconds))?;
        let result = compare(&expected, &actual)?;
        let host_equal = expected.tick_records_digest == actual.tick_records_digest
            && expected
                .samples
                .iter()
                .zip(&actual.samples)
                .all(|(a, b)| a.caller_digest == b.caller_digest && a.world_id == b.world_id);
        let classification = match result {
            audit::AuditResult::CompleteMatch if host_equal => "CompleteMatch",
            audit::AuditResult::CompleteMatch | audit::AuditResult::Diverged { .. } => "Diverged",
            audit::AuditResult::Incomplete { .. } => "Incomplete",
        };
        let record = serde_json::json!({
            "classification": classification, "runtime_result": format!("{result:?}"),
            "host_equal": host_equal, "start_tick": start_tick, "target_tick": request.target_tick,
            "points": expected.samples.len(), "active_seconds": clock.active().as_secs_f64(),
            "paused_seconds": clock.paused().as_secs_f64(), "wall_seconds": clock.wall().as_secs_f64(),
            "run_id": spec.run_id, "segment": batches,
            "parent_pid": std::process::id(), "child_pid": actual.pid,
            "environment_probe_rewrites": [expected.environment_probe_rewrites, actual.environment_probe_rewrites],
        });
        control::publish(&result_path, &record)?;
        println!("SOAK {record}");
        if classification != "CompleteMatch" {
            return Err(invalid(format!("audit stopped: {record}")));
        }
        batches += 1;
        covered_ticks += request.target_tick - start_tick;
        control.status("Running", &harness, &clock)?;
        if !config.paced {
            break;
        }
    }
    // 先处理已观察到的末尾请求，再发布终态；终态不再接受控制请求。
    if !stopped && control.pending()?.is_some() {
        stopped = control.boundary(&harness, &artifacts, &source, &identity, &mut clock)?;
    }
    let status = if stopped { "Incomplete" } else { "Completed" };
    let summary = serde_json::json!({
        "status": status, "reason": if stopped { "user stop" } else { "requested interval finished" },
        "scope": if config.paced { "bounded-resident-soak" } else { "short-soak-chain-validation" },
        "run_id": spec.run_id, "requested_seconds": config.seconds,
        "active_seconds": clock.active().as_secs_f64(), "paused_seconds": clock.paused().as_secs_f64(),
        "wall_seconds": clock.wall().as_secs_f64(), "complete_match_batches": batches,
        "covered_ticks": covered_ticks, "final_tick": harness.world().tick_index(),
        "initial_tick": initial_tick,
        "world_id": harness.world().world_id(), "resident_pid": std::process::id(),
        "initial_attestation": initial_attestation, "runtime_workers": 1, "paced": config.paced,
        "nominal_individuals": artifacts.individuals, "replacements": harness.replacements,
        "evidence_bytes": directory_bytes(output)?, "uncovered": ["long-term fault-host acceptance", "physical heap attribution", "between-checkpoint transient changes", "pause intervals have no periodic sampling", "initial installation and first tick"],
    });
    write(
        &output.join("summary.json"),
        &serde_json::to_vec_pretty(&summary)?,
    )?;
    control.status(
        if stopped { "Stopped" } else { "Completed" },
        &harness,
        &clock,
    )?;
    println!("SOAK_SUMMARY {summary}");
    Ok(())
}

#[test]
fn short_chain_replays_in_an_independent_process() {
    let temp = crate::child_diagnostics::DiagnosticDirectory::new().unwrap();
    let output = temp.path().join("short");
    run(&output, 30, false).unwrap();
    // 真正启动失败子进程：修订期望值失真时不得恢复，也不得沿用旧成功响应。
    let original = output.join("segments/000000/request.json");
    let mut request: Request = serde_json::from_slice(&fs::read(&original).unwrap()).unwrap();
    request.identity.revision[0] ^= 0x80;
    let rejected = output.join("rejected.json");
    write(&rejected, &serde_json::to_vec(&request).unwrap()).unwrap();
    write(
        &rejected.with_extension("parent-probe.bin"),
        &probe_bytes(request.target_tick),
    )
    .unwrap();
    assert!(replay(&rejected, Duration::from_secs(30)).is_err());
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
    supervisor::supervise(
        &output,
        Config::from_env().expect("bounded config"),
        "integrity_soak::resident_worker",
    )
    .expect("bounded soak");
}

#[test]
#[ignore = "驻留世界子进程；只由独立资源看护启动"]
fn resident_worker() {
    let output = PathBuf::from(std::env::var_os("LF_SOAK_OUTPUT").expect("output"));
    let spec = control::read_json(&output.join("config.json")).expect("run config");
    let result = run_in_created_directory(&output, &spec, true);
    record_failure(&output, &result);
    result.expect("resident soak");
}

#[test]
#[ignore = "显式操作指定的浸泡目录，不参与默认测试"]
fn control_soak() {
    let output = PathBuf::from(std::env::var_os("LF_SOAK_OUTPUT").expect("output"));
    control::issue(&output, &std::env::var("LF_SOAK_COMMAND").expect("command"))
        .expect("soak command");
}
