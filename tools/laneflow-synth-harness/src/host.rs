//! 测量环境记录与子进程观测接线（`docs/design/host-observe.md` §6）。
//!
//! 矩阵启动任何子进程前读取执行者输入文件并生成测量前环境记录，全部运行结束后
//! 生成测量后记录并比较；每个子进程以“仅直接子进程”范围、1 s 间隔采样。

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, ExitStatus},
    time::{Duration, Instant},
};

use laneflow_host_observe::{
    child::{ChildObserver, Scope},
    environment::{self, CollectOptions, EnvironmentRecord, ManualFields, Phase},
    snapshot,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Error;

/// 环境记录采集的工具链选择，与正式构建使用的工具链一致。
pub const TOOLCHAIN: &str = "1.99.1";
/// 子进程采样间隔（host-observe.md §5.5）。
pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
/// 测量前环境记录文件名。
pub const BEFORE: &str = "environment-before.json";
/// 测量后环境记录文件名。
pub const AFTER: &str = "environment-after.json";

/// 执行者输入文件：环境记录的必填字段与构建声明。不得含自动采集字段。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    /// 环境记录的执行者必填字段。
    pub environment: ManualFields,
    /// 主二进制的构建命令。
    pub build_command: String,
    /// 测试二进制的构建命令；只有使用测试二进制的矩阵要求。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_build_command: Option<String>,
}

/// 已生成测量前环境记录的一次矩阵采集。
pub struct Session {
    repository: PathBuf,
    declaration: Declaration,
    before: EnvironmentRecord,
}

fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn collect(
    phase: Phase,
    repository: &Path,
    manual: &ManualFields,
) -> Result<EnvironmentRecord, Error> {
    environment::collect(&CollectOptions {
        phase,
        repository: repository.to_path_buf(),
        toolchain: Some(TOOLCHAIN.into()),
        manual: manual.clone(),
        snapshot_interval: snapshot::DEFAULT_INTERVAL,
    })
    .map_err(|error| Error::Invalid(error.to_string()))
}

fn write_record(output: &Path, name: &str, record: &EnvironmentRecord) -> Result<Value, Error> {
    let bytes = record.to_json();
    fs::write(output.join(name), &bytes)?;
    Ok(json!({"file":name,"sha256":digest(&bytes),"bytes":bytes.len()}))
}

impl Session {
    /// 读取执行者输入文件，生成并核对测量前环境记录，写入 `output`。返回会话与
    /// 写入矩阵汇总的环境字段。
    ///
    /// # Errors
    ///
    /// 输入文件无效、构建声明为空、工作树不干净、提交与 `implementation` 不同、
    /// 工具链不可用或 P100 指纹不符时返回错误。
    pub fn begin(
        declaration: &Path,
        implementation: &str,
        output: &Path,
        require_test_build: bool,
    ) -> Result<(Self, Value), Error> {
        let declaration: Declaration = serde_json::from_slice(&fs::read(declaration)?)
            .map_err(|error| Error::Invalid(format!("执行者输入文件无效：{error}")))?;
        declaration
            .environment
            .validate()
            .map_err(|error| Error::Invalid(error.to_string()))?;
        let blank = |value: &str| value.trim().is_empty();
        if blank(&declaration.build_command)
            || (require_test_build && declaration.test_build_command.as_deref().is_none_or(blank))
        {
            return Err(Error::Invalid("执行者输入文件缺少构建声明".into()));
        }
        let repository = environment::repository_root(&std::env::current_dir()?)?;
        let before = collect(Phase::Before, &repository, &declaration.environment)?;
        let written = write_record(output, BEFORE, &before)?;
        before
            .require_measurement_ready(implementation, &repository)
            .map_err(|error| Error::Invalid(error.to_string()))?;
        let summary = json!({"declaration":declaration,"before":written});
        Ok((
            Self {
                repository,
                declaration,
                before,
            },
            summary,
        ))
    }

    /// 测量前环境记录在运行目录中的路径。
    pub fn before_path(output: &Path) -> PathBuf {
        output.join(BEFORE)
    }

    /// 生成测量后环境记录并与测量前比较，写入 `output`。返回写入矩阵汇总的字段。
    ///
    /// # Errors
    ///
    /// 采集失败，或 §3.5 的字段前后不一致时返回错误；差异字段写在错误消息中。
    pub fn finish(&self, output: &Path) -> Result<Value, Error> {
        let after = collect(
            Phase::After,
            &self.repository,
            &self.declaration.environment,
        )?;
        let written = write_record(output, AFTER, &after)?;
        let differences = self.before.differences(&after);
        if !differences.is_empty() {
            return Err(Error::Invalid(format!(
                "测量前后环境记录不一致：{}",
                differences.join("、")
            )));
        }
        Ok(written)
    }
}

/// 读取并核对子进程收到的测量前环境记录：格式与版本有效，提交与 `implementation`
/// 相同且工作树干净。返回记录字节的 SHA-256。
///
/// # Errors
///
/// 记录无效或不满足测量条件时返回错误。
pub fn verify_record(path: &Path, implementation: &str) -> Result<String, Error> {
    let bytes = fs::read(path)?;
    let record =
        EnvironmentRecord::from_json(&bytes).map_err(|error| Error::Invalid(error.to_string()))?;
    let repository = environment::repository_root(&std::env::current_dir()?)?;
    record
        .require_measurement_ready(implementation, &repository)
        .map_err(|error| Error::Invalid(error.to_string()))?;
    Ok(digest(&bytes))
}

/// 被观测的子进程：每次轮询时，距上次采样满 [`SAMPLE_INTERVAL`] 则先采样，再
/// 非阻塞查询退出。
pub struct Observed {
    observer: ChildObserver,
    last_sample: Option<Instant>,
}

impl Observed {
    /// 接管已启动的子进程。
    ///
    /// # Errors
    ///
    /// 无法初始化进程表读取时返回错误；此时子进程已被终止。
    pub fn new(child: Child) -> Result<Self, Error> {
        let mut observed = Self {
            observer: ChildObserver::new(child, Scope::DirectChild)?,
            last_sample: None,
        };
        observed.sample_if_due();
        Ok(observed)
    }

    /// 直接子进程的 PID。
    pub fn id(&self) -> u32 {
        self.observer.id()
    }

    fn sample_if_due(&mut self) {
        if self
            .last_sample
            .is_none_or(|last| last.elapsed() >= SAMPLE_INTERVAL)
        {
            self.observer.sample();
            self.last_sample = Some(Instant::now());
        }
    }

    /// 到期时采样，然后非阻塞查询子进程是否已退出。
    ///
    /// # Errors
    ///
    /// 查询失败时返回错误。
    pub fn poll(&mut self) -> Result<Option<ExitStatus>, Error> {
        self.sample_if_due();
        Ok(self.observer.try_wait()?)
    }

    /// 写入矩阵汇总的子进程记录：退出状态、采样峰值、采样 CPU 时间与采样参数。
    pub fn record(&self) -> Value {
        json!({"pid":self.id(),"sample_interval_ms":SAMPLE_INTERVAL.as_millis(),
            "summary":self.observer.summary()})
    }
}

/// 子进程自报的生命周期驻留峰值（`resources.json` 与 `final_process` 中
/// Windows `peak_working_set_bytes` 或 Linux `linux_vm_hwm_bytes` 的最大值）与父进程
/// 采样峰值的核对（host-observe.md §5.3）。
pub fn self_report_check(process_records: &[&Value], sampled_peak: Option<u64>) -> Value {
    let reported = process_records
        .iter()
        .filter_map(|process| {
            process["peak_working_set_bytes"]
                .as_u64()
                .or_else(|| process["linux_vm_hwm_bytes"].as_u64())
        })
        .max();
    let consistent = match (reported, sampled_peak) {
        (Some(reported), Some(sampled)) => Some(reported >= sampled),
        _ => None,
    };
    json!({"self_reported_peak_resident_bytes":reported,
        "sampled_peak_resident_bytes":sampled_peak,"consistent":consistent})
}
