//! 受界 fuzz 执行回执：绑定精确提交、工具链、二进制、输入库存与完整日志。
use std::{
    error::Error,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const TARGETS: &[&str] = &[
    "lfre_arbitrary_bytes",
    "lfre_writer_differential",
    "portable_arbitrary_bytes",
    "portable_bundle_mutation",
    "lfrs_restore",
    "cutover_inputs",
];
const PREPARED_DIR: &str = "fuzz/target/prepared-campaigns";
const PREPARED_ASAN_DIR: &str = "fuzz/target/prepared-asan";
const SANITIZER: &str = "address";

fn capture(program: &str, args: &[&str]) -> Result<String, Box<dyn Error>> {
    let output = Command::new(program).args(args).output()?;
    if !output.status.success() {
        return Err(format!("身份查询失败 {program} {args:?}: {}", output.status).into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn hash_file(path: &Path) -> Result<String, Box<dyn Error>> {
    let mut hash = Sha256::new();
    let mut file = File::open(path)?;
    let mut buffer = [0; 65_536];
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        hash.update(&buffer[..size]);
    }
    Ok(format!(
        "{:x}",
        laneflow_static_contract::Sha256Digest::from_bytes(hash.finalize().into())
    ))
}

fn inventory(root: &Path) -> Result<Vec<Value>, Box<dyn Error>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut paths = fs::read_dir(root)?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    let mut result = Vec::new();
    for path in paths {
        if !path.is_file() {
            return Err(format!("非预期语料对象: {}", path.display()).into());
        }
        result.push(json!({"name":path.file_name().unwrap().to_string_lossy(),
            "bytes":fs::metadata(&path)?.len(),"sha256":hash_file(&path)?}));
    }
    Ok(result)
}

fn save(path: &Path, receipt: &Value) -> Result<(), Box<dyn Error>> {
    let mut bytes = serde_json::to_vec_pretty(receipt)?;
    bytes.push(b'\n');
    fs::write(path, bytes)?;
    Ok(())
}

fn checkout_identity() -> Result<Value, Box<dyn Error>> {
    Ok(json!({
        "source_commit": capture("git", &["rev-parse", "HEAD"])?,
        "tracked_diff": capture("git", &["diff", "--stat", "HEAD"])?,
        "root_lock_sha256": hash_file(Path::new("Cargo.lock"))?,
        "fuzz_lock_sha256": hash_file(Path::new("fuzz/Cargo.lock"))?,
    }))
}

fn toolchain_identity() -> Result<Value, Box<dyn Error>> {
    let rustc = capture("rustc", &["+nightly", "--version", "--verbose"])?;
    let host = rustc
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .ok_or("rustc 缺少 host triple")?;
    Ok(json!({
        "rustc": rustc,
        "host": host,
        "cargo": capture("cargo", &["+nightly", "--version"])?,
    }))
}

fn target_binary(host: &str, filename: &str) -> PathBuf {
    Path::new("fuzz/target")
        .join(host)
        .join("release")
        .join(filename)
}

fn prepare() -> Result<(), Box<dyn Error>> {
    let checkout = checkout_identity()?;
    if checkout["tracked_diff"] != "" {
        return Err("输入准备要求受检文件保持不变".into());
    }
    let toolchain = toolchain_identity()?;
    let output = Path::new(PREPARED_DIR);
    fs::create_dir(output)?;
    let launcher = output.join(format!("run_campaign{}", std::env::consts::EXE_SUFFIX));
    fs::copy(std::env::current_exe()?, &launcher)?;

    let mut seeds = serde_json::Map::new();
    for &target in TARGETS {
        let initial_corpus = inventory(&Path::new("fuzz/corpus").join(target))?;
        if initial_corpus.is_empty() {
            return Err(format!("缺少 {target} 的准备语料").into());
        }
        seeds.insert(target.to_owned(), json!(initial_corpus));
    }

    save(
        &output.join("inputs.json"),
        &json!({
            "checkout": checkout,
            "toolchain": toolchain,
            "run": {
                "run_id": std::env::var("GITHUB_RUN_ID").ok(),
                "run_attempt": std::env::var("GITHUB_RUN_ATTEMPT").ok(),
                "image_version": std::env::var("ImageVersion").ok(),
            },
            "launcher_sha256": hash_file(&launcher)?,
            "seeds": seeds,
        }),
    )
}

fn recorded_sha256<'a>(report: &'a str, filename: &str) -> Option<&'a str> {
    report
        .lines()
        .filter_map(|line| line.split_once("  "))
        .find_map(|(digest, name)| (name == filename).then_some(digest))
}

struct PreparedCampaign {
    toolchain: Value,
    build_run: Value,
    inputs_run: Value,
}

fn read_prepared(
    checkout: &Value,
    target: &str,
    binary: &Path,
    initial_corpus: &[Value],
) -> Result<PreparedCampaign, Box<dyn Error>> {
    let inputs: Value =
        serde_json::from_reader(File::open(Path::new(PREPARED_DIR).join("inputs.json"))?)?;
    let metadata = Path::new(PREPARED_ASAN_DIR).join("metadata");
    let read = |name: &str| fs::read_to_string(metadata.join(name));
    let context = read("context")?;
    let context: Vec<_> = context.lines().collect();
    let [build_source, build_run_id, build_attempt, build_image] = context.as_slice() else {
        return Err("缺少 ASan 构建身份".into());
    };
    let run_id = std::env::var("GITHUB_RUN_ID")?;
    if *build_run_id != run_id
        || inputs["run"]["run_id"] != run_id
        || checkout["source_commit"].as_str() != Some(*build_source)
        || &inputs["checkout"] != checkout
        || checkout["tracked_diff"] != ""
    {
        return Err("ASan 构建、输入准备和本次执行的运行或源码身份不一致".into());
    }
    let lockfiles = read("locks.sha256")?;
    if recorded_sha256(&lockfiles, "Cargo.lock") != checkout["root_lock_sha256"].as_str()
        || recorded_sha256(&lockfiles, "fuzz/Cargo.lock") != checkout["fuzz_lock_sha256"].as_str()
    {
        return Err("ASan 构建与本次执行的锁文件摘要不一致".into());
    }
    let mut toolchain = inputs["toolchain"].clone();
    if toolchain["rustc"] != read("rustc")?.trim() || toolchain["cargo"] != read("cargo")?.trim() {
        return Err("ASan 构建与输入准备的 Rust 工具链不一致".into());
    }
    toolchain["cargo_fuzz"] = json!(read("cargo-fuzz")?.trim());

    let binaries = read("binaries.sha256")?;
    let binary_sha256 = hash_file(binary)?;
    if recorded_sha256(&binaries, target) != Some(binary_sha256.as_str())
        || inputs["seeds"][target].as_array().map(Vec::as_slice) != Some(initial_corpus)
    {
        return Err(format!("{target} 的二进制或初始语料与准备制品不一致").into());
    }
    Ok(PreparedCampaign {
        toolchain,
        build_run: json!({
            "run_id": build_run_id,
            "run_attempt": build_attempt,
            "image_version": build_image,
        }),
        inputs_run: inputs["run"].clone(),
    })
}

fn run(target: &str, use_prepared: bool) -> Result<(), Box<dyn Error>> {
    if !TARGETS.contains(&target) {
        return Err("未登记的 fuzz target".into());
    }
    let duration: u64 = std::env::var("FUZZ_MAX_TOTAL_TIME")
        .unwrap_or_else(|_| "300".into())
        .parse()?;
    if ![300, 1_800].contains(&duration) {
        return Err("只接受已冻结的 smoke/正式窗口".into());
    }
    let receipts = Path::new("fuzz/receipts");
    fs::create_dir_all(receipts)?;
    let output = receipts.join(target);
    fs::create_dir(&output)?;
    let receipt_path = output.join("receipt.json");
    let corpus = Path::new("fuzz/corpus").join(target);
    let artifacts = Path::new("fuzz/artifacts").join(target);
    let initial_corpus = inventory(&corpus)?;
    let checkout = checkout_identity()?;
    let filename = format!("{target}{}", std::env::consts::EXE_SUFFIX);
    let prepared_binary = Path::new(PREPARED_ASAN_DIR).join(&filename);
    let prepared = if use_prepared {
        Some(read_prepared(
            &checkout,
            target,
            &prepared_binary,
            &initial_corpus,
        )?)
    } else {
        None
    };
    let toolchain = match &prepared {
        Some(prepared) => prepared.toolchain.clone(),
        None => {
            let mut toolchain = toolchain_identity()?;
            toolchain["cargo_fuzz"] = json!(capture("cargo", &["+nightly", "fuzz", "--version"])?);
            toolchain
        }
    };
    let host = toolchain["host"].as_str().ok_or("缺少构建主机")?;
    let binary = if use_prepared {
        prepared_binary
    } else {
        target_binary(host, &filename)
    };
    let mut receipt = json!({"format":"laneflow-security-fuzz-1","complete":false,
        "target":target,"source_commit":checkout["source_commit"],"tracked_diff":checkout["tracked_diff"],
        "rustc":toolchain["rustc"],"cargo":toolchain["cargo"],"cargo_fuzz":toolchain["cargo_fuzz"],
        "host":host,"started_unix_seconds":SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        "duration_limit_seconds":duration,"input_timeout_seconds":25,"rss_limit_mib":2_048,
        "max_input_bytes":262_144,"seed":53_601,"sanitizer":SANITIZER,
        "initial_corpus":initial_corpus,"root_lock_sha256":checkout["root_lock_sha256"],
        "fuzz_lock_sha256":checkout["fuzz_lock_sha256"],
        "limitations":"bounded exploration; RSS/timeout stops are failures; no exhaustive security proof"});
    if let Some(prepared) = &prepared {
        receipt["prepared_build"] = prepared.build_run.clone();
        receipt["prepared_inputs"] = prepared.inputs_run.clone();
    }
    for key in [
        "GITHUB_SHA",
        "GITHUB_REF",
        "GITHUB_RUN_ID",
        "GITHUB_RUN_ATTEMPT",
        "GITHUB_EVENT_NAME",
        "RUNNER_OS",
        "RUNNER_ARCH",
        "ImageOS",
        "ImageVersion",
    ] {
        receipt["environment"][key] = std::env::var(key).map(Value::String).unwrap_or(Value::Null);
    }
    save(&receipt_path, &receipt)?;
    let engine_arguments = vec![
        format!("-max_total_time={duration}"),
        "-timeout=25".into(),
        "-rss_limit_mb=2048".into(),
        "-max_len=262144".into(),
        "-seed=53601".into(),
        "-print_final_stats=1".into(),
    ];
    let root = std::env::current_dir()?;
    let (program, arguments) = if use_prepared {
        fs::create_dir_all(&artifacts)?;
        let mut arguments = vec![format!(
            "-artifact_prefix={}{}",
            root.join(&artifacts).display(),
            std::path::MAIN_SEPARATOR,
        )];
        arguments.extend(engine_arguments.iter().cloned());
        arguments.push(root.join(&corpus).to_string_lossy().into_owned());
        (binary.to_string_lossy().into_owned(), arguments)
    } else {
        let mut arguments = vec![
            "+nightly".to_owned(),
            "fuzz".into(),
            "run".into(),
            target.to_owned(),
            "--target".into(),
            host.to_owned(),
            "--target-dir".into(),
            root.join("fuzz/target").to_string_lossy().into_owned(),
            "--sanitizer".into(),
            SANITIZER.into(),
            "--".into(),
        ];
        arguments.extend(engine_arguments.iter().cloned());
        ("cargo".to_owned(), arguments)
    };

    // 与 cargo-fuzz 0.13.2 的 AddressSanitizer 运行环境保持一致。
    let mut asan_options = std::env::var("ASAN_OPTIONS").unwrap_or_default();
    if !asan_options.is_empty() {
        asan_options.push(':');
    }
    asan_options.push_str("detect_odr_violation=0");
    receipt["libfuzzer_arguments"] = json!(engine_arguments);
    receipt["asan_options"] = json!(asan_options);
    receipt["command"] = json!({"program":program,"arguments":arguments});
    save(&receipt_path, &receipt)?;
    let log_path = output.join("run.log");
    let log = File::create(&log_path)?;
    let started = Instant::now();
    let mut command = Command::new(program);
    if use_prepared {
        command.env("ASAN_OPTIONS", asan_options);
    }
    let status = command
        .args(&arguments)
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .status()?;
    receipt["process_elapsed_seconds"] = json!(started.elapsed().as_secs_f64());
    receipt["exit_code"] = json!(status.code());
    receipt["final_corpus"] = json!(inventory(&corpus)?);
    receipt["artifacts"] = json!(inventory(&artifacts)?);
    receipt["log_sha256"] = json!(hash_file(&log_path)?);
    let log_text = fs::read_to_string(&log_path)?;
    receipt["final_statistics"] = json!(log_text
        .lines()
        .filter(|line| line.starts_with("stat::")
            || line.contains(" DONE ")
            || line.starts_with("Done "))
        .collect::<Vec<_>>());
    if binary.is_file() {
        receipt["binary_sha256"] = json!(hash_file(&binary)?);
    }
    let lockfiles_unchanged = receipt["root_lock_sha256"] == hash_file(Path::new("Cargo.lock"))?
        && receipt["fuzz_lock_sha256"] == hash_file(Path::new("fuzz/Cargo.lock"))?;
    receipt["lockfiles_unchanged"] = json!(lockfiles_unchanged);
    receipt["complete"] = json!(
        status.success()
            && lockfiles_unchanged
            && receipt["tracked_diff"] == ""
            && receipt["binary_sha256"].is_string()
            && log_text.lines().any(|line| line.starts_with("Done "))
    );
    save(&receipt_path, &receipt)?;
    io::copy(&mut File::open(log_path)?, &mut io::stdout().lock())?;
    io::stdout().flush()?;
    if receipt["complete"] != true {
        return Err(format!("fuzz 未完成，保留回执 {}", receipt_path.display()).into());
    }
    Ok(())
}

fn run_command() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let first = args.next().ok_or("缺少 fuzz target 或 --prepare")?;
    match (first.as_str(), args.next().as_deref(), args.next()) {
        ("--prepare", None, None) => prepare(),
        (target, mode @ (None | Some("--prepared")), None) => run(target, mode.is_some()),
        _ => Err("预期 --prepare 或 <target> [--prepared]".into()),
    }
}

fn main() -> ExitCode {
    match run_command() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
