//! 校准 fixture 容器格式与字节摘要。容器把 workload 的全部 geometry 源文档绑定为
//! 一个可哈希文件；manifest 以容器文件的字节长度与 SHA-256 登记 fixture 身份。

use serde_json::json;
use sha2::{Digest, Sha256};

/// 容器 schema 常量（`schema` 字段值）。
pub const CONTAINER_SCHEMA: &str = "laneflow.geometry-frontend-calibration-fixture";

/// 一个源模块条目：`sourcePath` 即 harness 传入的 `source_document_key`，必须与文档内
/// `module.documentKey` 一致。
#[derive(Clone, Debug)]
pub struct FixtureModule {
    pub source_path: String,
    pub source: String,
}

/// 序列化容器为紧凑 JSON 字节（键序由 serde_json BTreeMap 固定，逐字节确定）。
#[must_use]
pub fn encode_container(workload_id: &str, modules: &[FixtureModule]) -> Vec<u8> {
    let modules: Vec<_> = modules
        .iter()
        .map(|module| {
            json!({
                "sourcePath": module.source_path,
                "source": module.source,
            })
        })
        .collect();
    let container = json!({
        "schema": CONTAINER_SCHEMA,
        "schemaVersion": 1,
        "workloadId": workload_id,
        "modules": modules,
    });
    serde_json::to_string(&container)
        .expect("容器序列化")
        .into_bytes()
}

/// 计算字节的 SHA-256 并格式化为 64 位小写十六进制。
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// 解码容器字节并校验自报 schema / schemaVersion；返回 `(workloadId, 模块集合)`。
/// 任何格式偏差即 panic：校准容器必须由生成器逐字节产生。
#[must_use]
pub fn decode_container(bytes: &[u8]) -> (String, Vec<FixtureModule>) {
    let value: serde_json::Value = serde_json::from_slice(bytes).expect("容器必须是合法 JSON");
    let schema = value
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .expect("容器缺少 schema 字段");
    assert_eq!(schema, CONTAINER_SCHEMA, "容器 schema 字段不匹配");
    let schema_version = value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .expect("容器缺少 schemaVersion 字段");
    assert_eq!(schema_version, 1, "不支持的容器 schemaVersion");
    let workload_id = value
        .get("workloadId")
        .and_then(serde_json::Value::as_str)
        .expect("容器缺少 workloadId 字段")
        .to_string();
    let modules = value
        .get("modules")
        .and_then(serde_json::Value::as_array)
        .expect("容器缺少 modules 字段")
        .iter()
        .map(|module| FixtureModule {
            source_path: module
                .get("sourcePath")
                .and_then(serde_json::Value::as_str)
                .expect("容器模块缺少 sourcePath 字段")
                .to_string(),
            source: module
                .get("source")
                .and_then(serde_json::Value::as_str)
                .expect("容器模块缺少 source 字段")
                .to_string(),
        })
        .collect();
    (workload_id, modules)
}
