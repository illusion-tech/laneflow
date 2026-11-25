//! 环境记录（`host-observe.md` §3）。
//!
//! harness 在测量开始前与全部测量结束后各采集一次，两份记录写入运行目录并列入
//! 文件摘要清单。记录由自动采集字段、执行者必填字段、机器指纹与一份进程快照
//! 组成。自动字段取不到时记为不可用并附原因。

#[cfg(target_os = "linux")]
use std::collections::BTreeSet;
use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::{
    Field, command,
    snapshot::{self, ProcessSnapshot},
};

/// 记录格式名。
pub const FORMAT: &str = "laneflow-environment-record";
/// 当前格式版本。增删字段、改变字段语义或必填字段集合时加一。
pub const VERSION: u32 = 1;
/// 机器指纹方案（`core-runtime-performance-baseline.md` §5）。
pub const FINGERPRINT_SCHEME: &str = "laneflow-p100-hardware-identity-v2";
/// P100 参考机声明相对仓库根的位置。
pub const REFERENCE_MACHINE_PATH: &str =
    "docs/reference/road-editing-source-reference-machine-v1.json";

/// 环境记录读取、校验与核对错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentError(String);

impl EnvironmentError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for EnvironmentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for EnvironmentError {}

impl From<EnvironmentError> for io::Error {
    fn from(error: EnvironmentError) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, error)
    }
}

/// 采集阶段。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// 测量开始前。
    Before,
    /// 全部测量结束后。
    After,
}

/// 硬件角色（`core-runtime-performance-baseline.md` §5）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HardwareRole {
    /// R0 研究参考。
    R0,
    /// P10 产品最低平台。
    P10,
    /// P100 目标产品推荐参考机型；指纹须与参考机声明一致。
    P100,
    /// O1 一百万 observation。
    O1,
    /// 未认证机器。
    #[serde(rename = "uncertified")]
    Uncertified,
}

/// 执行者填写的必填字段（§3.3）。输入文件只能含这些字段。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManualFields {
    /// 厂商控制软件中的性能模式；没有该功能时写明“无厂商性能模式”。
    pub vendor_performance_mode: String,
    /// 硬件角色。
    pub hardware_role: HardwareRole,
}

const PLACEHOLDERS: &[&str] = &[
    "unknown",
    "unavailable",
    "n/a",
    "na",
    "none",
    "null",
    "tbd",
    "-",
    "未知",
    "不可用",
];

impl ManualFields {
    /// 解析并校验执行者输入文件（JSON）。含自动采集字段或其他未知字段时拒绝。
    ///
    /// # Errors
    ///
    /// JSON 格式错误、含未知字段，或字段为空、只含空白、为占位值时返回错误。
    pub fn from_json(bytes: &[u8]) -> Result<Self, EnvironmentError> {
        let fields: Self = serde_json::from_slice(bytes)
            .map_err(|error| EnvironmentError::new(format!("执行者输入文件无效：{error}")))?;
        fields.validate()?;
        Ok(fields)
    }

    /// 校验必填字段：不得为空、只含空白或写成占位值（不区分大小写）。
    ///
    /// # Errors
    ///
    /// 字段不满足要求时返回错误。
    pub fn validate(&self) -> Result<(), EnvironmentError> {
        let value = self.vendor_performance_mode.trim();
        if value.is_empty()
            || PLACEHOLDERS
                .iter()
                .any(|placeholder| value.eq_ignore_ascii_case(placeholder))
        {
            return Err(EnvironmentError::new(
                "必填字段 vendor_performance_mode 为空或为占位值；没有该功能时写明“无厂商性能模式”",
            ));
        }
        Ok(())
    }
}

/// 采集元数据。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Collection {
    /// 采集时刻（UTC，RFC 3339）。
    pub recorded_at: String,
    /// 采集阶段。
    pub phase: Phase,
    /// 本 crate 版本。
    pub collector_version: String,
}

/// 操作系统。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Os {
    /// 名称。
    pub name: Field<String>,
    /// 版本。
    pub version: Field<String>,
    /// 完整版本描述。
    pub long_version: Field<String>,
    /// 内核版本；Windows 上为 build 号。
    pub kernel: Field<String>,
    /// 架构。
    pub architecture: String,
}

/// CPU。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cpu {
    /// 型号。
    pub model: Field<String>,
    /// 物理核数。
    pub physical_cores: Field<u32>,
    /// 逻辑处理器数。
    pub logical_processors: Field<u32>,
}

/// 内存。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Memory {
    /// OS 可见物理内存字节数。
    pub physical_bytes: Field<u64>,
}

/// Windows 电源计划。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PowerPlan {
    /// 电源计划 GUID（小写）。
    pub guid: String,
    /// 名称：系统预置计划为固定英文名，其他计划取 `powercfg` 输出。
    pub name: Field<String>,
}

/// 电源。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Power {
    /// 交流电是否接通。
    pub ac_online: Field<bool>,
    /// 是否存在电池。
    pub battery_present: Field<bool>,
    /// 电池电量百分比。
    pub battery_percent: Field<u8>,
    /// Windows 电源计划。
    pub windows_power_plan: Field<PowerPlan>,
    /// Linux 各 CPU `scaling_governor` 取值集合。
    pub linux_cpufreq_governors: Field<Vec<String>>,
    /// Linux 平台性能配置 `platform_profile`。
    pub linux_platform_profile: Field<String>,
}

/// BIOS 与固件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Firmware {
    /// BIOS 厂商。
    pub bios_vendor: Field<String>,
    /// BIOS 版本。
    pub bios_version: Field<String>,
    /// BIOS 日期。
    pub bios_date: Field<String>,
}

/// 工具链。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Toolchain {
    /// 调用方指定的工具链选择（如 `1.100.0`，执行时加 `+`）；`None` 为默认工具链。
    pub selector: Option<String>,
    /// `rustc -Vv` 全文。
    pub rustc_vv: String,
    /// `cargo -V`。
    pub cargo_version: String,
    /// `rustc -Vv` 中的主机 target。
    pub host_target: String,
}

/// 源码状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// 提交（40 位十六进制）。
    pub commit: String,
    /// 树对象。
    pub tree: String,
    /// 工作树是否干净：`git status --porcelain --untracked-files=all` 为空，且索引
    /// 中没有 skip-worktree 或 assume-unchanged 条目。
    pub clean: bool,
}

/// 机器指纹：只含方案名与摘要，不含原始 SMBIOS 值。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fingerprint {
    /// 方案名。
    pub scheme: String,
    /// 64 位小写十六进制 SHA-256。
    pub sha256: String,
}

/// 一份环境记录。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRecord {
    /// 格式名，恒为 [`FORMAT`]。
    pub format: String,
    /// 格式版本，恒为 [`VERSION`]。
    pub version: u32,
    /// 采集元数据。
    pub collection: Collection,
    /// 操作系统。
    pub os: Os,
    /// CPU。
    pub cpu: Cpu,
    /// 内存。
    pub memory: Memory,
    /// 电源。
    pub power: Power,
    /// BIOS 与固件。
    pub firmware: Firmware,
    /// 工具链。
    pub toolchain: Field<Toolchain>,
    /// 源码状态。
    pub source: Field<Source>,
    /// 机器指纹。
    pub fingerprint: Field<Fingerprint>,
    /// 执行者填写的必填字段。
    pub manual: ManualFields,
    /// 后台进程记录。
    pub processes: ProcessSnapshot,
}

/// 采集参数。
#[derive(Clone, Debug)]
pub struct CollectOptions {
    /// 采集阶段。
    pub phase: Phase,
    /// 源码 checkout 内的任一目录；git 命令锚定到其顶层目录执行。
    pub repository: PathBuf,
    /// 工具链选择（如 `1.100.0`）；`None` 为默认工具链。
    pub toolchain: Option<String>,
    /// 执行者填写的必填字段。
    pub manual: ManualFields,
    /// 进程快照的读数间隔。
    pub snapshot_interval: Duration,
}

/// 采集一份环境记录。
///
/// # Errors
///
/// 必填字段不合格或无法读取进程表时返回错误；其余字段取不到时记为不可用。
pub fn collect(options: &CollectOptions) -> Result<EnvironmentRecord, EnvironmentError> {
    options.manual.validate()?;
    let processes = snapshot::capture(options.snapshot_interval)
        .map_err(|error| EnvironmentError::new(format!("无法采集进程快照：{error}")))?;
    let (cpu, memory) = cpu_and_memory();
    Ok(EnvironmentRecord {
        format: FORMAT.into(),
        version: VERSION,
        collection: Collection {
            recorded_at: crate::time::now_utc(),
            phase: options.phase,
            collector_version: env!("CARGO_PKG_VERSION").into(),
        },
        os: Os {
            name: Field::from_result(sysinfo::System::name(), "OS 名称"),
            version: Field::from_result(sysinfo::System::os_version(), "OS 版本"),
            long_version: Field::from_result(sysinfo::System::long_os_version(), "OS 完整版本"),
            kernel: Field::from_result(sysinfo::System::kernel_version(), "内核版本"),
            architecture: std::env::consts::ARCH.into(),
        },
        cpu,
        memory,
        power: power(),
        firmware: firmware(),
        toolchain: Field::from_result(toolchain(options.toolchain.as_deref()), "工具链"),
        source: Field::from_result(source(&options.repository), "源码状态"),
        fingerprint: fingerprint(),
        manual: options.manual.clone(),
        processes,
    })
}

impl EnvironmentRecord {
    /// 编码为 JSON。
    pub fn to_json(&self) -> Vec<u8> {
        let mut bytes = serde_json::to_vec_pretty(self).unwrap_or_default();
        bytes.push(b'\n');
        bytes
    }

    /// 读取 JSON 记录：只接受已知版本，拒绝未知字段，不转换旧版本。
    ///
    /// # Errors
    ///
    /// 格式名或版本不符、含未知字段，或必填字段不合格时返回错误。
    pub fn from_json(bytes: &[u8]) -> Result<Self, EnvironmentError> {
        #[derive(Deserialize)]
        struct Header {
            format: String,
            version: u32,
        }
        let header: Header = serde_json::from_slice(bytes)
            .map_err(|error| EnvironmentError::new(format!("环境记录无效：{error}")))?;
        if header.format != FORMAT {
            return Err(EnvironmentError::new(format!(
                "环境记录格式名不是 {FORMAT}"
            )));
        }
        if header.version != VERSION {
            return Err(EnvironmentError::new(format!(
                "不支持的环境记录版本 {}（只接受 {VERSION}）",
                header.version
            )));
        }
        let record: Self = serde_json::from_slice(bytes)
            .map_err(|error| EnvironmentError::new(format!("环境记录无效：{error}")))?;
        record.manual.validate()?;
        Ok(record)
    }

    /// 核对记录可以开始测量：工具链可用；源码可用、干净且提交与
    /// `implementation_commit` 相同；必填字段合格；硬件角色为 P100 时指纹与
    /// `repository_root` 下的参考机声明一致。
    ///
    /// # Errors
    ///
    /// 任一条件不满足时返回错误。
    pub fn require_measurement_ready(
        &self,
        implementation_commit: &str,
        repository_root: &Path,
    ) -> Result<(), EnvironmentError> {
        self.manual.validate()?;
        if self.toolchain.value().is_none() {
            return Err(EnvironmentError::new("环境记录缺少工具链"));
        }
        let source = self
            .source
            .value()
            .ok_or_else(|| EnvironmentError::new("环境记录缺少源码状态"))?;
        if source.commit != implementation_commit {
            return Err(EnvironmentError::new(
                "环境记录的源码提交与实现提交参数不同",
            ));
        }
        if !source.clean {
            return Err(EnvironmentError::new("工作树不干净，不能开始测量"));
        }
        if self.manual.hardware_role == HardwareRole::P100 {
            let path = repository_root.join(REFERENCE_MACHINE_PATH);
            let reference = fs::read(&path).map_err(|error| {
                EnvironmentError::new(format!("无法读取 P100 参考机声明：{error}"))
            })?;
            require_reference_machine(&self.fingerprint, &reference)?;
        }
        Ok(())
    }

    /// 测量前（`self`）与测量后（`after`）两份记录中须一致而不一致的字段组
    /// （§3.5）。阶段不是“前、后”时也列入差异。电量、采集时刻与进程快照不参与比较。
    pub fn differences(&self, after: &Self) -> Vec<&'static str> {
        let mut differences = Vec::new();
        let mut check = |name: &'static str, same: bool| {
            if !same {
                differences.push(name);
            }
        };
        check(
            "collection.phase",
            self.collection.phase == Phase::Before && after.collection.phase == Phase::After,
        );
        check(
            "os",
            (
                &self.os.name,
                &self.os.version,
                &self.os.long_version,
                &self.os.kernel,
            ) == (
                &after.os.name,
                &after.os.version,
                &after.os.long_version,
                &after.os.kernel,
            ),
        );
        check("firmware", self.firmware == after.firmware);
        check(
            "power.windows_power_plan",
            self.power.windows_power_plan == after.power.windows_power_plan,
        );
        check(
            "power.linux_cpufreq_governors",
            self.power.linux_cpufreq_governors == after.power.linux_cpufreq_governors,
        );
        check(
            "power.linux_platform_profile",
            self.power.linux_platform_profile == after.power.linux_platform_profile,
        );
        check(
            "power.ac_online",
            self.power.ac_online == after.power.ac_online,
        );
        check("toolchain", self.toolchain == after.toolchain);
        check("source", self.source == after.source);
        check("fingerprint", self.fingerprint == after.fingerprint);
        check("manual", self.manual == after.manual);
        differences
    }
}

/// 核对指纹与参考机声明的方案及摘要完全一致。
pub(crate) fn require_reference_machine(
    fingerprint: &Field<Fingerprint>,
    reference: &[u8],
) -> Result<(), EnvironmentError> {
    let reference: serde_json::Value = serde_json::from_slice(reference)
        .map_err(|error| EnvironmentError::new(format!("P100 参考机声明无效：{error}")))?;
    let fingerprint = fingerprint
        .value()
        .ok_or_else(|| EnvironmentError::new("硬件角色为 P100，但机器指纹不可用"))?;
    if reference["hardwareIdentityScheme"] != fingerprint.scheme.as_str()
        || reference["hardwareIdentitySha256"] != fingerprint.sha256.as_str()
    {
        return Err(EnvironmentError::new(
            "硬件角色为 P100，但机器指纹与参考机声明不符",
        ));
    }
    Ok(())
}

#[cfg(any(windows, test))]
/// 按 `laneflow-p100-hardware-identity-v2` 计算指纹：每个值去除全部空白并转为
/// 大写，按 UUID、BIOS 序列号、主板序列号的顺序以单个换行拼接、无尾换行，
/// 对 UTF-8 字节计算 SHA-256。任一值规范化后为空时返回 `None`。
pub(crate) fn fingerprint_digest(
    uuid: &str,
    bios_serial: &str,
    board_serial: &str,
) -> Option<String> {
    use sha2::{Digest, Sha256};

    let normalize = |value: &str| -> String {
        value
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>()
            .to_uppercase()
    };
    let values = [
        normalize(uuid),
        normalize(bios_serial),
        normalize(board_serial),
    ];
    if values.iter().any(String::is_empty) {
        return None;
    }
    let digest = Sha256::digest(values.join("\n").as_bytes());
    Some(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(windows)]
fn fingerprint() -> Field<Fingerprint> {
    // 三个原始值只在本函数内存在，不写入记录、日志或错误消息。
    let uuid = sysinfo::Product::uuid().ok();
    let bios_serial = sysinfo::Product::serial_number().ok();
    let board_serial = sysinfo::Motherboard::new()
        .ok()
        .and_then(|board| board.serial_number());
    match (uuid, bios_serial, board_serial) {
        (Some(uuid), Some(bios_serial), Some(board_serial)) => Field::from_option(
            fingerprint_digest(&uuid, &bios_serial, &board_serial).map(|sha256| Fingerprint {
                scheme: FINGERPRINT_SCHEME.into(),
                sha256,
            }),
            "SMBIOS 标识字段为空",
        ),
        _ => Field::unavailable("无法读取 SMBIOS 标识字段"),
    }
}

#[cfg(not(windows))]
fn fingerprint() -> Field<Fingerprint> {
    Field::unavailable("v2 指纹方案按 Windows WMI 类定义，本平台未定义指纹方案")
}

fn cpu_and_memory() -> (Cpu, Memory) {
    let unavailable = |reason: &str| {
        (
            Cpu {
                model: Field::unavailable(reason),
                physical_cores: Field::unavailable(reason),
                logical_processors: Field::unavailable(reason),
            },
            Memory {
                physical_bytes: Field::unavailable(reason),
            },
        )
    };
    let mut system = match sysinfo::System::new() {
        Ok(system) => system,
        Err(error) => return unavailable(&format!("无法初始化系统信息读取：{error}")),
    };
    system.refresh_cpu_list(sysinfo::CpuRefreshKind::nothing());
    system.refresh_memory();
    let cpus = system.cpus();
    let model = cpus
        .first()
        .map(|cpu| cpu.brand().trim().to_owned())
        .filter(|brand| !brand.is_empty());
    let physical = sysinfo::System::physical_core_count()
        .map_err(|error| error.to_string())
        .and_then(|count| u32::try_from(count).map_err(|error| error.to_string()));
    (
        Cpu {
            model: Field::from_option(model, "CPU 型号不可用"),
            physical_cores: Field::from_result(physical, "物理核数"),
            logical_processors: Field::from_option(
                u32::try_from(cpus.len()).ok().filter(|count| *count > 0),
                "逻辑处理器数不可用",
            ),
        },
        Memory {
            physical_bytes: Field::from_option(
                Some(system.total_memory()).filter(|bytes| *bytes > 0),
                "物理内存不可用",
            ),
        },
    )
}

fn power() -> Power {
    let status = laneflow_os_probe::Probe::new().and_then(|mut probe| probe.power());
    let (ac_online, battery_present, battery_percent) = match status {
        Ok(status) => (
            Field::from_option(status.ac_online, "平台未报告交流电状态"),
            Field::from_option(status.battery_present, "平台未报告电池状态"),
            Field::from_option(status.battery_percent, "平台未报告电池电量"),
        ),
        Err(error) => {
            let reason = format!("无法读取电源状态：{error}");
            (
                Field::unavailable(&reason),
                Field::unavailable(&reason),
                Field::unavailable(reason),
            )
        }
    };
    Power {
        ac_online,
        battery_present,
        battery_percent,
        windows_power_plan: windows_power_plan(),
        linux_cpufreq_governors: linux_cpufreq_governors(),
        linux_platform_profile: linux_platform_profile(),
    }
}

#[cfg(any(windows, test))]
/// Windows 系统预置电源计划的 GUID 与固定英文名。
const WELL_KNOWN_POWER_PLANS: &[(&str, &str)] = &[
    ("381b4222-f694-41f0-9685-ff5bb260df2e", "Balanced"),
    ("8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c", "High performance"),
    ("a1841308-3541-4fab-bc81-f71556f20b4a", "Power saver"),
    (
        "e9a42b02-d5df-448d-aa00-03f14749eb61",
        "Ultimate Performance",
    ),
];

#[cfg(any(windows, test))]
/// 解析 `powercfg /getactivescheme` 输出：取第一个 GUID 形式的词；系统预置计划
/// 用固定英文名，其他计划取 GUID 后括号内文字（输出不是 UTF-8 时为不可用）。
pub(crate) fn parse_power_plan(output: &[u8]) -> Option<PowerPlan> {
    let ascii: String = output
        .iter()
        .map(|&byte| if byte.is_ascii() { byte as char } else { ' ' })
        .collect();
    let guid = ascii
        .split(|character: char| !(character.is_ascii_hexdigit() || character == '-'))
        .find(|word| is_guid(word))?
        .to_ascii_lowercase();
    let name = if let Some((_, name)) = WELL_KNOWN_POWER_PLANS
        .iter()
        .find(|(known, _)| *known == guid)
    {
        Field::Value((*name).to_owned())
    } else {
        match std::str::from_utf8(output) {
            Ok(text) => Field::from_option(
                text.rfind('(')
                    .zip(text.rfind(')'))
                    .filter(|(open, close)| open < close)
                    .map(|(open, close)| text[open + 1..close].trim().to_owned())
                    .filter(|name| !name.is_empty()),
                "powercfg 输出中没有计划名称",
            ),
            Err(_) => Field::unavailable("powercfg 输出不是 UTF-8"),
        }
    };
    Some(PowerPlan { guid, name })
}

#[cfg(any(windows, test))]
fn is_guid(word: &str) -> bool {
    let groups: Vec<&str> = word.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(group, length)| group.len() == length)
}

#[cfg(windows)]
fn windows_power_plan() -> Field<PowerPlan> {
    match command::output_bytes("powercfg", &["/getactivescheme"], None) {
        Ok(output) => Field::from_option(parse_power_plan(&output), "powercfg 输出中没有 GUID"),
        Err(error) => Field::unavailable(format!("无法执行 powercfg：{error}")),
    }
}

#[cfg(not(windows))]
fn windows_power_plan() -> Field<PowerPlan> {
    Field::unavailable("仅 Windows 提供")
}

#[cfg(target_os = "linux")]
fn linux_cpufreq_governors() -> Field<Vec<String>> {
    let governors: BTreeSet<String> = fs::read_dir("/sys/devices/system/cpu")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_prefix("cpu"))
                .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
        })
        .filter_map(|entry| fs::read_to_string(entry.path().join("cpufreq/scaling_governor")).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect();
    if governors.is_empty() {
        Field::unavailable("没有可读的 scaling_governor")
    } else {
        Field::Value(governors.into_iter().collect())
    }
}

#[cfg(not(target_os = "linux"))]
fn linux_cpufreq_governors() -> Field<Vec<String>> {
    Field::unavailable("仅 Linux 提供")
}

#[cfg(target_os = "linux")]
fn linux_platform_profile() -> Field<String> {
    read_trimmed("/sys/firmware/acpi/platform_profile")
}

#[cfg(not(target_os = "linux"))]
fn linux_platform_profile() -> Field<String> {
    Field::unavailable("仅 Linux 提供")
}

#[cfg(target_os = "linux")]
fn read_trimmed(path: &str) -> Field<String> {
    match fs::read_to_string(path) {
        Ok(value) => Field::from_option(
            Some(value.trim().to_owned()).filter(|value| !value.is_empty()),
            "文件为空",
        ),
        Err(error) => Field::unavailable(format!("无法读取 {path}：{error}")),
    }
}

#[cfg(any(windows, test))]
/// 解析 `reg query` 输出中名为 `name` 的 `REG_SZ` 值。
pub(crate) fn parse_registry_value(output: &str, name: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let rest = line.trim_start().strip_prefix(name)?;
        let rest = rest.trim_start().strip_prefix("REG_SZ")?;
        Some(rest.trim().to_owned()).filter(|value| !value.is_empty())
    })
}

#[cfg(windows)]
fn firmware() -> Firmware {
    match command::output(
        "reg",
        &["query", r"HKLM\HARDWARE\DESCRIPTION\System\BIOS"],
        None,
    ) {
        Ok(output) => {
            let value = |name: &str| {
                Field::from_option(parse_registry_value(&output, name), "注册表中没有该值")
            };
            Firmware {
                bios_vendor: value("BIOSVendor"),
                bios_version: value("BIOSVersion"),
                bios_date: value("BIOSReleaseDate"),
            }
        }
        Err(error) => {
            let reason = format!("无法读取 BIOS 注册表项：{error}");
            Firmware {
                bios_vendor: Field::unavailable(&reason),
                bios_version: Field::unavailable(&reason),
                bios_date: Field::unavailable(reason),
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn firmware() -> Firmware {
    Firmware {
        bios_vendor: read_trimmed("/sys/class/dmi/id/bios_vendor"),
        bios_version: read_trimmed("/sys/class/dmi/id/bios_version"),
        bios_date: read_trimmed("/sys/class/dmi/id/bios_date"),
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn firmware() -> Firmware {
    Firmware {
        bios_vendor: Field::unavailable("本平台未实现"),
        bios_version: Field::unavailable("本平台未实现"),
        bios_date: Field::unavailable("本平台未实现"),
    }
}

fn toolchain(selector: Option<&str>) -> io::Result<Toolchain> {
    let plus = selector.map(|selector| format!("+{selector}"));
    let args = |flag: &'static str| -> Vec<&str> {
        plus.as_deref()
            .into_iter()
            .chain(std::iter::once(flag))
            .collect()
    };
    let rustc_vv = command::output("rustc", &args("-Vv"), None)?;
    let host_target = rustc_vv
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(str::to_owned)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "rustc -Vv 中没有 host 行"))?;
    Ok(Toolchain {
        selector: selector.map(str::to_owned),
        cargo_version: command::output("cargo", &args("-V"), None)?,
        rustc_vv,
        host_target,
    })
}

/// 判定 `git ls-files -v` 输出中是否有 skip-worktree（`S`）或 assume-unchanged
/// （小写标签）条目。
pub(crate) fn has_hidden_index_entries(ls_files: &str) -> bool {
    ls_files
        .lines()
        .any(|line| line.starts_with(|c: char| c.is_ascii_lowercase()) || line.starts_with('S'))
}

/// `directory` 所在 git checkout 的顶层目录，用于定位 [`REFERENCE_MACHINE_PATH`]。
/// 路径只供本机读取，不写入记录。
///
/// # Errors
///
/// 不在 git checkout 内或 git 不可用时返回错误。
pub fn repository_root(directory: &Path) -> io::Result<PathBuf> {
    command::output("git", &["rev-parse", "--show-toplevel"], Some(directory)).map(PathBuf::from)
}

pub(crate) fn source(repository: &Path) -> io::Result<Source> {
    let top = command::output("git", &["rev-parse", "--show-toplevel"], Some(repository))?;
    let top_path = Path::new(&top);
    // --work-tree 压住仓库本地 config 的 core.worktree：读取锚定到该 checkout。
    let git = |args: &[&str]| -> io::Result<String> {
        let mut full = vec!["--work-tree", top.as_str()];
        full.extend_from_slice(args);
        command::output("git", &full, Some(top_path))
    };
    let commit = git(&["rev-parse", "HEAD"])?;
    if commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "HEAD 不是 40 位提交",
        ));
    }
    let status = git(&["status", "--porcelain", "--untracked-files=all"])?;
    let ls_files = git(&["ls-files", "-v"])?;
    Ok(Source {
        tree: git(&["rev-parse", "HEAD^{tree}"])?,
        clean: status.is_empty() && !has_hidden_index_entries(&ls_files),
        commit,
    })
}
