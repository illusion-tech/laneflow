//! 测量环境记录接线（`docs/design/host-observe.md` §3、§6）。
//!
//! 执行者经环境变量 [`INPUT_VARIABLE`] 指定输入文件，只含环境记录的必填字段
//! （厂商性能模式与硬件角色）；原 `LANEFLOW_HARDWARE_ROLE` 与 `LANEFLOW_POWER_ROLE`
//! 退役，电源状态改为自动采集。

use std::{fmt::Display, fs, path::PathBuf};

use laneflow_host_observe::{
    Field,
    environment::{self, CollectOptions, EnvironmentRecord, ManualFields, Phase},
    snapshot,
};
#[cfg(feature = "adapter")]
use serde_json::{Value, json};

use crate::{Result, invalid};

/// 指向执行者输入文件的环境变量。
pub(crate) const INPUT_VARIABLE: &str = "LANEFLOW_ENVIRONMENT_INPUT";
/// 测量前环境记录文件名。
pub(crate) const BEFORE: &str = "environment-before.json";
/// 测量后环境记录文件名。
pub(crate) const AFTER: &str = "environment-after.json";

/// 读取 [`INPUT_VARIABLE`] 指定的执行者输入文件。
pub(crate) fn manual() -> Result<ManualFields> {
    let path = std::env::var_os(INPUT_VARIABLE)
        .ok_or_else(|| invalid(format!("performance requires {INPUT_VARIABLE}")))?;
    ManualFields::from_json(&fs::read(path)?).map_err(|error| invalid(error.to_string()))
}

/// 在当前目录所在 checkout 内采集一份环境记录，并返回 checkout 顶层目录。
pub(crate) fn collect(phase: Phase, manual: &ManualFields) -> Result<(EnvironmentRecord, PathBuf)> {
    let repository = environment::repository_root(&std::env::current_dir()?)?;
    // 与正式测量构建同一工具链：rustup 在 checkout 内强制的钉版 channel。
    let toolchain =
        environment::pinned_toolchain(&repository).map_err(|error| invalid(error.to_string()))?;
    let record = environment::collect(&CollectOptions {
        phase,
        repository: repository.clone(),
        toolchain: Some(toolchain),
        manual: manual.clone(),
        snapshot_interval: snapshot::DEFAULT_INTERVAL,
    })
    .map_err(|error| invalid(error.to_string()))?;
    Ok((record, repository))
}

/// 核对记录可以开始测量：工具链可用且等于仓库钉版、工作树干净、必填字段合格，
/// P100 角色的指纹与参考机声明一致。提交取记录自身的 HEAD。
pub(crate) fn require_ready(
    record: &EnvironmentRecord,
    repository: &std::path::Path,
) -> Result<()> {
    let commit = record
        .source
        .value()
        .map(|source| source.commit.clone())
        .ok_or_else(|| invalid("environment record has no source state"))?;
    record
        .require_measurement_ready(&commit, repository)
        .map_err(|error| invalid(error.to_string()))
}

/// 字段值的文本形式；不可用时为 `unavailable`。
pub(crate) fn text<T: Display>(field: &Field<T>) -> String {
    field
        .value()
        .map_or_else(|| "unavailable".to_owned(), ToString::to_string)
}

/// 电源来源：`ac`、`battery` 或 `unavailable`。
pub(crate) fn power_source(record: &EnvironmentRecord) -> String {
    match record.power.ac_online.value() {
        Some(true) => "ac".into(),
        Some(false) => "battery".into(),
        None => "unavailable".into(),
    }
}

/// 电源计划：Windows 为计划 GUID，Linux 为调频策略集合与平台性能配置，取不到时为
/// `unavailable`。
pub(crate) fn power_plan(record: &EnvironmentRecord) -> String {
    if let Some(plan) = record.power.windows_power_plan.value() {
        return plan.guid.clone();
    }
    let governors = record
        .power
        .linux_cpufreq_governors
        .value()
        .map(|values| values.join(","));
    let profile = record.power.linux_platform_profile.value();
    match (governors, profile) {
        (None, None) => "unavailable".into(),
        (governors, profile) => format!(
            "governors={};platform_profile={}",
            governors.as_deref().unwrap_or("unavailable"),
            profile.map_or("unavailable", String::as_str)
        ),
    }
}

/// 机器指纹摘要；不可用时为 `unavailable`。
pub(crate) fn fingerprint(record: &EnvironmentRecord) -> String {
    record
        .fingerprint
        .value()
        .map_or_else(|| "unavailable".to_owned(), |value| value.sha256.clone())
}

/// 同一机器与配置下前后两次采集应相同的环境摘要，用于证据入口的来源比较。
#[cfg(feature = "adapter")]
fn stable_summary(record: &EnvironmentRecord) -> Value {
    json!({
        "format":environment::FORMAT, "version":environment::VERSION,
        "os":text(&record.os.long_version), "os_build":text(&record.os.kernel),
        "architecture":record.os.architecture,
        "cpu":text(&record.cpu.model), "physical_memory_bytes":text(&record.memory.physical_bytes),
        "bios_version":text(&record.firmware.bios_version),
        "power_source":power_source(record), "power_plan":power_plan(record),
        "machine_fingerprint":fingerprint(record),
        "hardware_role":serde_json::to_value(record.manual.hardware_role).unwrap_or(Value::Null),
        "vendor_performance_mode":record.manual.vendor_performance_mode,
    })
}

/// 证据入口的环境摘要：先按正式测量的同一规则核对（工作树干净、工具链可用、
/// 必填字段合格、P100 角色的指纹与参考机声明一致，host-observe.md §3.3），
/// 再生成摘要。
#[cfg(feature = "adapter")]
pub(crate) fn ready_summary(
    record: &EnvironmentRecord,
    repository: &std::path::Path,
) -> Result<Value> {
    require_ready(record, repository)?;
    Ok(stable_summary(record))
}
