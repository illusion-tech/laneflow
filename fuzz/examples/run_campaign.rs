//! 受界 fuzz 执行回执：绑定精确提交、工具链、二进制、输入库存与完整日志。
use std::{
    error::Error,
    fs::{self, File},
    io::{self, Read, Write},
    path::Path,
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

fn run() -> Result<(), Box<dyn Error>> {
    let target = std::env::args().nth(1).ok_or("缺少 fuzz target")?;
    if !TARGETS.contains(&target.as_str()) {
        return Err("未登记的 fuzz target".into());
    }
    let duration: u64 = std::env::var("FUZZ_MAX_TOTAL_TIME")
        .unwrap_or_else(|_| "300".into())
        .parse()?;
    if ![300, 1_800].contains(&duration) {
        return Err("只接受已冻结的 smoke/正式窗口".into());
    }
    let output = Path::new("fuzz/receipts").join(&target);
    fs::create_dir_all(output.parent().unwrap())?;
    fs::create_dir(&output)?;
    let receipt_path = output.join("receipt.json");
    let corpus = Path::new("fuzz/corpus").join(&target);
    let artifacts = Path::new("fuzz/artifacts").join(&target);
    let rustc = capture("rustc", &["+nightly", "--version", "--verbose"])?;
    let host = rustc
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .ok_or("rustc 缺少 host triple")?;
    let mut receipt = json!({"format":"laneflow-security-fuzz-1","complete":false,
        "target":target,"source_commit":capture("git", &["rev-parse", "HEAD"])?,
        "tracked_diff":capture("git", &["diff", "--stat", "HEAD"])?,
        "rustc":rustc,"cargo":capture("cargo", &["+nightly", "--version"])?,
        "cargo_fuzz":capture("cargo", &["+nightly", "fuzz", "--version"])?,
        "host":host,"started_unix_seconds":SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        "duration_limit_seconds":duration,"input_timeout_seconds":25,"rss_limit_mib":2_048,
        "max_input_bytes":262_144,"seed":53_601,"sanitizer":"address",
        "initial_corpus":inventory(&corpus)?,"root_lock_sha256":hash_file(Path::new("Cargo.lock"))?,
        "fuzz_lock_sha256":hash_file(Path::new("fuzz/Cargo.lock"))?,
        "limitations":"bounded exploration; RSS/timeout stops are failures; no exhaustive security proof"});
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
    let arguments = vec![
        "+nightly".to_owned(),
        "fuzz".into(),
        "run".into(),
        target.clone(),
        "--target".into(),
        host.to_owned(),
        "--target-dir".into(),
        std::env::current_dir()?
            .join("fuzz/target")
            .to_string_lossy()
            .into_owned(),
        "--sanitizer".into(),
        "address".into(),
        "--".into(),
        format!("-max_total_time={duration}"),
        "-timeout=25".into(),
        "-rss_limit_mb=2048".into(),
        "-max_len=262144".into(),
        "-seed=53601".into(),
        "-print_final_stats=1".into(),
    ];
    receipt["command"] = json!({"program":"cargo","arguments":arguments});
    save(&receipt_path, &receipt)?;
    let log_path = output.join("run.log");
    let log = File::create(&log_path)?;
    let started = Instant::now();
    let status = Command::new("cargo")
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
    let binary = Path::new("fuzz/target")
        .join(host)
        .join("release")
        .join(format!("{target}{}", std::env::consts::EXE_SUFFIX));
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

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
