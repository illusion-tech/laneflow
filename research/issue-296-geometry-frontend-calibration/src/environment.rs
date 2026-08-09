//! 正式证据环境字段采集（§9.2：操作系统/CPU/内存/电源与固件身份）。全部可采集字段在
//! 测量时从操作系统与 toolchain 独立重报，并与参考机声明逐字段比对；任何漂移即拒绝
//! 正式运行。hardwareId / hardwareIdentityScheme 是声明名而非测量值，由声明原文提供；
//! 硬件身份 SHA-256 按 `laneflow-p100-hardware-identity-v1` 规则在本机重算。

use std::path::Path;

use serde_json::{Value, json};

use crate::container::sha256_hex;
use crate::validator::{contract_artifact, load_contract};

/// 一次性 CIM 采集脚本（stdout 强制 UTF-8，避免本地化中文在 GBK 代码页下损坏）。
const CIM_SCRIPT: &str = r#"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$os = Get-CimInstance Win32_OperatingSystem
$cs = Get-CimInstance Win32_ComputerSystem
$bios = Get-CimInstance Win32_BIOS
$baseboard = Get-CimInstance Win32_BaseBoard | Select-Object -First 1
$product = Get-CimInstance Win32_ComputerSystemProduct | Select-Object -First 1
$planName = ''
try {
  $plan = Get-CimInstance -Namespace root/cimv2/power -ClassName Win32_PowerPlan -ErrorAction Stop | Where-Object { $_.IsActive } | Select-Object -First 1
  $planName = [string]$plan.ElementName
} catch {}
if ([string]::IsNullOrWhiteSpace($planName)) {
  $saved = [Console]::OutputEncoding
  [Console]::OutputEncoding = [System.Text.Encoding]::GetEncoding(936)
  $schemeLine = [string](powercfg /getactivescheme | Select-Object -First 1)
  [Console]::OutputEncoding = $saved
  if ($schemeLine -match '\(([^)]+)\)') { $planName = $Matches[1] }
}
Add-Type -AssemblyName System.Windows.Forms
[pscustomobject]@{
  cpuName = [string]$cpu.Name
  physicalCores = [uint64]$cpu.NumberOfCores
  logicalProcessors = [uint64]$cpu.NumberOfLogicalProcessors
  totalPhysicalMemory = [uint64]$cs.TotalPhysicalMemory
  osCaption = [string]$os.Caption
  osBuild = [string]$os.BuildNumber
  biosVersion = [string]$bios.SMBIOSBIOSVersion
  biosReleaseDate = $bios.ReleaseDate.ToString('yyyy-MM-dd')
  biosSerial = [string]$bios.SerialNumber
  baseboardSerial = [string]$baseboard.SerialNumber
  smbiosUuid = [string]$product.UUID
  powerPlan = $planName
  powerLineStatus = [string][System.Windows.Forms.SystemInformation]::PowerStatus.PowerLineStatus
} | ConvertTo-Json -Compress
"#;

/// 从 OS 与 toolchain 采集的环境字段（与参考机声明同名字段一一对应）。
pub struct CollectedEnvironment {
    pub hardware_identity_sha256: String,
    pub cpu: String,
    pub physical_core_count: u64,
    pub logical_processor_count: u64,
    pub physical_memory_bytes: u64,
    pub operating_system: String,
    pub operating_system_build: String,
    pub target_triple: String,
    pub rustc: String,
    pub llvm: String,
    pub power_source: String,
    pub power_plan: String,
    pub bios_firmware: String,
}

/// 运行外部命令并返回 UTF-8 stdout（去 BOM、去首尾空白）；非零退出即 panic。
fn run_command(program: &str, args: &[&str]) -> String {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("启动 {program} 失败：{error}"));
    assert!(
        output.status.success(),
        "{program} 退出失败：{}",
        output.status
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|_| panic!("{program} 输出必须是 UTF-8"))
        .trim_start_matches('\u{feff}')
        .trim()
        .to_string()
}

/// `laneflow-p100-hardware-identity-v1`：smbiosUuid / biosSerial / baseboardSerial 去全部
/// 空白并转大写，`\n` 拼接、无尾换行，SHA-256。
fn hardware_identity_sha256(uuid: &str, bios_serial: &str, baseboard_serial: &str) -> String {
    fn normalized(value: &str) -> String {
        value
            .chars()
            .filter(|c| !c.is_whitespace())
            .flat_map(char::to_uppercase)
            .collect()
    }
    let material = format!(
        "{}\n{}\n{}",
        normalized(uuid),
        normalized(bios_serial),
        normalized(baseboard_serial)
    );
    sha256_hex(material.as_bytes())
}

fn required_string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("CIM 采集缺少 {key} 字段"))
        .trim()
        .to_string()
}

fn required_u64(value: &Value, key: &str) -> u64 {
    value
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("CIM 采集缺少 {key} 字段"))
}

/// 采集 OS / 固件 / 电源 / toolchain 字段（不读参考机声明；采集必须独立）。
pub fn collect_environment() -> CollectedEnvironment {
    let cim_stdout = run_command(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", CIM_SCRIPT],
    );
    let cim: Value = serde_json::from_str(&cim_stdout).expect("CIM 采集输出必须是合法 JSON");
    let power_line_status = required_string(&cim, "powerLineStatus");
    let power_source = match power_line_status.as_str() {
        "Online" => "交流电",
        "Offline" => "电池",
        other => panic!("无法判读的电源状态：{other}"),
    }
    .to_string();
    let bios_firmware = format!(
        "{} ({})",
        required_string(&cim, "biosVersion"),
        required_string(&cim, "biosReleaseDate")
    );

    let rustc_version = run_command("rustc", &["+1.96.0", "--version"]);
    let rustc = rustc_version
        .strip_prefix("rustc ")
        .expect("rustc --version 输出必须以 rustc 开头")
        .to_string();
    let mut target_triple = String::new();
    let mut llvm = String::new();
    for line in run_command("rustc", &["+1.96.0", "-vV"]).lines() {
        if let Some(host) = line.strip_prefix("host: ") {
            target_triple = host.trim().to_string();
        }
        if let Some(version) = line.strip_prefix("LLVM version: ") {
            llvm = version.trim().to_string();
        }
    }
    assert!(!target_triple.is_empty(), "rustc -vV 缺少 host 行");
    assert!(!llvm.is_empty(), "rustc -vV 缺少 LLVM version 行");

    CollectedEnvironment {
        hardware_identity_sha256: hardware_identity_sha256(
            &required_string(&cim, "smbiosUuid"),
            &required_string(&cim, "biosSerial"),
            &required_string(&cim, "baseboardSerial"),
        ),
        cpu: required_string(&cim, "cpuName"),
        physical_core_count: required_u64(&cim, "physicalCores"),
        logical_processor_count: required_u64(&cim, "logicalProcessors"),
        physical_memory_bytes: required_u64(&cim, "totalPhysicalMemory"),
        operating_system: required_string(&cim, "osCaption"),
        operating_system_build: required_string(&cim, "osBuild"),
        target_triple,
        rustc,
        llvm,
        power_source,
        power_plan: required_string(&cim, "powerPlan"),
        bios_firmware,
    }
}

fn declaration_string<'a>(declaration: &'a Value, key: &str) -> &'a str {
    declaration
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("参考机声明缺少 {key} 字段"))
}

fn declaration_u64(declaration: &Value, key: &str) -> u64 {
    declaration
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("参考机声明缺少 {key} 字段"))
}

/// 采集值与参考机声明逐字段比对；任何漂移即 panic（正式校准必须发生在声明机器上）。
pub fn verify_against_declaration(collected: &CollectedEnvironment, declaration: &Value) {
    let string_fields: [(&str, &str, &str); 11] = [
        (
            "hardwareIdentitySha256",
            declaration_string(declaration, "hardwareIdentitySha256"),
            &collected.hardware_identity_sha256,
        ),
        (
            "cpu",
            declaration_string(declaration, "cpu"),
            &collected.cpu,
        ),
        (
            "operatingSystem",
            declaration_string(declaration, "operatingSystem"),
            &collected.operating_system,
        ),
        (
            "operatingSystemBuild",
            declaration_string(declaration, "operatingSystemBuild"),
            &collected.operating_system_build,
        ),
        (
            "targetTriple",
            declaration_string(declaration, "targetTriple"),
            &collected.target_triple,
        ),
        (
            "rustc",
            declaration_string(declaration, "rustc"),
            &collected.rustc,
        ),
        (
            "llvm",
            declaration_string(declaration, "llvm"),
            &collected.llvm,
        ),
        (
            "powerSource",
            declaration_string(declaration, "powerSource"),
            &collected.power_source,
        ),
        (
            "powerPlan",
            declaration_string(declaration, "powerPlan"),
            &collected.power_plan,
        ),
        (
            "biosFirmware",
            declaration_string(declaration, "biosFirmware"),
            &collected.bios_firmware,
        ),
        (
            "hardwareIdentityScheme",
            declaration_string(declaration, "hardwareIdentityScheme"),
            "laneflow-p100-hardware-identity-v1",
        ),
    ];
    for (key, expected, actual) in string_fields {
        assert_eq!(actual, expected, "环境字段 {key} 与参考机声明不符");
    }
    let integer_fields: [(&str, u64, u64); 3] = [
        (
            "physicalCoreCount",
            declaration_u64(declaration, "physicalCoreCount"),
            collected.physical_core_count,
        ),
        (
            "logicalProcessorCount",
            declaration_u64(declaration, "logicalProcessorCount"),
            collected.logical_processor_count,
        ),
        (
            "physicalMemoryBytes",
            declaration_u64(declaration, "physicalMemoryBytes"),
            collected.physical_memory_bytes,
        ),
    ];
    for (key, expected, actual) in integer_fields {
        assert_eq!(actual, expected, "环境字段 {key} 与参考机声明不符");
    }
}

/// 读取 contract 绑定的参考机声明，独立采集并比对，然后装配证据 `environment` 对象。
/// 两个观察标志按 #308 操作者声明模式置 false：正式运行全程交流电，无睡眠/锁屏、
/// 无热/电源节流被观察到；本函数只在操作者确认该事实的正式运行中调用。
pub fn environment_json(repo_root: &Path) -> Value {
    let contract = load_contract(repo_root);
    let declaration_path = contract_artifact(&contract, "referenceMachineDeclaration")
        .get("path")
        .and_then(Value::as_str)
        .expect("contract 参考机声明缺少 path");
    let declaration_bytes = std::fs::read(repo_root.join(declaration_path))
        .unwrap_or_else(|error| panic!("读取参考机声明失败：{error}"));
    let declaration: Value =
        serde_json::from_slice(&declaration_bytes).expect("参考机声明必须是合法 JSON");
    let collected = collect_environment();
    verify_against_declaration(&collected, &declaration);
    json!({
        "hardwareId": declaration_string(&declaration, "hardwareId"),
        "hardwareIdentityScheme": declaration_string(&declaration, "hardwareIdentityScheme"),
        "hardwareIdentitySha256": collected.hardware_identity_sha256,
        "cpu": collected.cpu,
        "physicalCoreCount": collected.physical_core_count,
        "logicalProcessorCount": collected.logical_processor_count,
        "physicalMemoryBytes": collected.physical_memory_bytes,
        "operatingSystem": collected.operating_system,
        "operatingSystemBuild": collected.operating_system_build,
        "targetTriple": collected.target_triple,
        "rustc": collected.rustc,
        "llvm": collected.llvm,
        "powerSource": collected.power_source,
        "powerPlan": collected.power_plan,
        "biosFirmware": collected.bios_firmware,
        "sleepOrSessionLockObserved": false,
        "thermalOrPowerThrottlingObserved": false,
    })
}
