//! 证据锚点解析：离线核验 manifest 声明的记录存在且身份命中。
//!
//! 只做存在性、锚点子串与声明身份的对账；不重算摘要、不联网、不执行任何
//! 测量。`expect` 声明的字段逐一与记录比对，缺失或不同即失败关闭。

use std::fs;
use std::path::PathBuf;

use crate::manifest::{EVIDENCE_ROOT_PREFIX, Evidence, GateJsonExpect, SynthSummaryExpect};
use crate::{Error, invalid};

/// 路径解析上下文：仓库根 + 可选的仓库外证据根。
#[derive(Debug, Clone)]
pub struct PathContext {
    pub repo_root: PathBuf,
    pub evidence_root: Option<PathBuf>,
}

impl PathContext {
    /// 把 manifest 里的声明路径解析为实际路径。
    ///
    /// # Errors
    /// 路径使用 `<evidence-root>/` 前缀但未提供 `--evidence-root` 时失败。
    pub fn resolve(&self, path: &str) -> Result<PathBuf, Error> {
        if let Some(rest) = path.strip_prefix(EVIDENCE_ROOT_PREFIX) {
            let root = self.evidence_root.as_ref().ok_or_else(|| {
                invalid(format!(
                    "路径使用了 {EVIDENCE_ROOT_PREFIX} 占位但未提供 --evidence-root：{path}"
                ))
            })?;
            Ok(root.join(rest))
        } else {
            Ok(self.repo_root.join(path))
        }
    }
}

/// 一条证据的核验产物：`label` 进回执正文，`detail` 是核验结论；
/// `identity` 携带记录自身声明的输入身份（当前仅 `l1-record`），供汇总层
/// 与 manifest 声明判等。
#[derive(Debug, Clone)]
pub struct EvidenceOutcome {
    pub label: String,
    pub detail: String,
    pub identity: Option<String>,
}

/// 核验一条证据锚点。
///
/// # Errors
/// 文件缺失、解析失败、锚点未命中或 `expect` 身份不符时失败（失败关闭，
/// 不产出半张回执）。
pub fn resolve(evidence: &Evidence, ctx: &PathContext) -> Result<EvidenceOutcome, Error> {
    match evidence {
        Evidence::RepoDoc { path, anchors } => {
            let full = ctx.resolve(path)?;
            let text = fs::read_to_string(&full)
                .map_err(|e| invalid(format!("仓库文档读取失败 {path}：{e}")))?;
            let mut hits = 0;
            for anchor in anchors {
                if !text.contains(anchor) {
                    return Err(invalid(format!("锚点未命中：{path} 不包含「{anchor}」")));
                }
                hits += 1;
            }
            let detail = if anchors.is_empty() {
                "存在（未声明锚点）".to_owned()
            } else {
                format!("锚点 {hits}/{} 命中", anchors.len())
            };
            Ok(EvidenceOutcome {
                label: path.clone(),
                detail,
                identity: None,
            })
        }
        Evidence::Github { url } => {
            const PREFIX: &str = "https://github.com/illusion-tech/laneflow/";
            if !url.starts_with(PREFIX) {
                return Err(invalid(format!("GitHub 证据须以 {PREFIX} 开头：{url}")));
            }
            Ok(EvidenceOutcome {
                label: url.clone(),
                detail: "链接（离线工具只做形态校验，不联网核验）".to_owned(),
                identity: None,
            })
        }
        Evidence::SynthSummaryJson { path, expect } => {
            let full = ctx.resolve(path)?;
            let bytes =
                fs::read(&full).map_err(|e| invalid(format!("记录读取失败 {path}：{e}")))?;
            let value: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|e| invalid(format!("记录不是 JSON {path}：{e}")))?;
            let get = |key: &str| value.get(key).and_then(serde_json::Value::as_str);
            // 二进制身份有两种生产端形状：语义矩阵的顶层 `binary_sha256`，与
            // F1/F2/F3 矩阵的嵌套 `binary.sha256`（`file_identity` 的
            // {bytes, sha256}）；两种都消费，回执注明来源字段。
            let binary_top = get("binary_sha256");
            let binary_nested = value
                .pointer("/binary/sha256")
                .and_then(serde_json::Value::as_str);
            let complete = value.get("complete").and_then(serde_json::Value::as_bool);
            check_synth_summary(&value, binary_top, binary_nested, expect.as_ref(), path)?;
            let mut detail = Vec::new();
            if let Some(format_id) = get("format") {
                detail.push(format!("format={format_id}"));
            }
            if let Some(commit) = get("implementation_commit") {
                detail.push(format!("implementation_commit={commit}"));
            }
            match (binary_top, binary_nested) {
                (Some(_), Some(_)) => {
                    return Err(invalid(format!(
                        "记录同时携带 binary_sha256 与 binary.sha256：身份字段形状不合法：{path}"
                    )));
                }
                (Some(digest), None) => detail.push(format!("binary_sha256={digest}")),
                (None, Some(digest)) => detail.push(format!("binary.sha256={digest}")),
                (None, None) => {}
            }
            if let Some(complete) = complete {
                detail.push(format!("complete={complete}"));
            }
            if detail.is_empty() {
                return Err(invalid(format!(
                    "记录缺少身份字段（format/implementation_commit/binary_sha256|binary.sha256/complete）：{path}"
                )));
            }
            Ok(EvidenceOutcome {
                label: path.clone(),
                detail: detail.join("，"),
                identity: None,
            })
        }
        Evidence::GateJson { path, expect } => {
            let full = ctx.resolve(path)?;
            let bytes =
                fs::read(&full).map_err(|e| invalid(format!("记录读取失败 {path}：{e}")))?;
            let value: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|e| invalid(format!("记录不是 JSON {path}：{e}")))?;
            let status = value.get("status").and_then(serde_json::Value::as_str);
            let commit = value
                .pointer("/source/commit")
                .and_then(serde_json::Value::as_str);
            let binary_digest = value
                .pointer("/source/binary/digest")
                .and_then(serde_json::Value::as_str);
            check_gate_json(status, commit, binary_digest, expect.as_ref(), path)?;
            let mut detail = Vec::new();
            if let Some(status) = status {
                detail.push(format!("status={status}"));
            }
            if let Some(commit) = commit {
                detail.push(format!("source.commit={commit}"));
            }
            if let Some(digest) = binary_digest {
                detail.push(format!("source.binary.digest={digest}"));
            }
            if detail.is_empty() {
                return Err(invalid(format!(
                    "记录缺少身份字段（status/source.commit/source.binary.digest）：{path}"
                )));
            }
            Ok(EvidenceOutcome {
                label: path.clone(),
                detail: detail.join("，"),
                identity: None,
            })
        }
        Evidence::RoutingKv { path, expect } => {
            let full = ctx.resolve(path)?;
            let text = fs::read_to_string(&full)
                .map_err(|e| invalid(format!("记录读取失败 {path}：{e}")))?;
            let headers = routing_headers(&text);
            let Some(&(marker, _)) = headers.first() else {
                return Err(invalid(format!(
                    "记录缺少墙钟头行（{}）：{path}",
                    ROUTING_HEADER_MARKERS.join(" / ")
                )));
            };
            if let Some(&(other, _)) = headers.iter().find(|(m, _)| *m != marker) {
                return Err(invalid(format!(
                    "记录混有两种墙钟头行（{marker} 与 {other}）：{path}"
                )));
            }
            // 期望键须覆盖该头行的身份键：缺了就不能证明绑定的是哪个 case。
            for required in routing_required_keys(marker) {
                if !expect
                    .iter()
                    .any(|wanted| wanted.split_once('=').is_some_and(|(k, _)| k == *required))
                {
                    return Err(invalid(format!(
                        "{marker} 记录的 expect 须包含 {required}：{path}"
                    )));
                }
            }
            // 多轮记录拼在同一文件时，每一个头行都必须命中，不能只看第一行。
            for (_, header) in &headers {
                let keys = parse_kv_header(header);
                for wanted in expect {
                    let (key, value) = wanted
                        .split_once('=')
                        .ok_or_else(|| invalid(format!("expect 条目须为 key=value：{wanted}")))?;
                    match keys.get(key) {
                        Some(actual) if *actual == value => {}
                        Some(actual) => {
                            return Err(invalid(format!(
                                "身份不符：{path} 的 {key}={actual}，期望 {key}={value}"
                            )));
                        }
                        None => {
                            return Err(invalid(format!(
                                "记录缺少期望键 {key}（未知键前向容忍，期望键缺失不容忍）：{path}"
                            )));
                        }
                    }
                }
            }
            let keys = parse_kv_header(headers[0].1);
            let mut identity = String::new();
            for key in routing_required_keys(marker) {
                if let Some(value) = keys.get(key) {
                    identity.push_str(&format!("{key}={value}，"));
                }
            }
            Ok(EvidenceOutcome {
                label: path.clone(),
                detail: format!(
                    "{identity}头行 {} 个，期望键 {}/{} 逐行命中",
                    headers.len(),
                    expect.len(),
                    expect.len()
                ),
                identity: None,
            })
        }
        Evidence::L1Record { path, expect } => {
            let full = ctx.resolve(path)?;
            let bytes =
                fs::read(&full).map_err(|e| invalid(format!("记录读取失败 {path}：{e}")))?;
            let value: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|e| invalid(format!("记录不是 JSON {path}：{e}")))?;
            let format_id = value.get("format").and_then(serde_json::Value::as_str);
            if format_id != Some(crate::l1check::L1_RECORD_FORMAT) {
                return Err(invalid(format!(
                    "L1 核查记录格式不符：{path} 的 format={format_id:?}"
                )));
            }
            let verdict = value
                .get("verdict")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| invalid(format!("L1 核查记录缺少 verdict：{path}")))?;
            // 结论无条件要求 clean：#1271 处置规定含这种拓扑的路网不得用于
            // 安全验收，`violations-found` 不能依赖可选 expect 才被拦下。
            if verdict != "clean" {
                return Err(invalid(format!(
                    "L1 核查结论必须为 clean（含 #1271 拓扑的输入不得用于安全验收）：{path} 的 verdict={verdict}"
                )));
            }
            if let Some(expect) = expect
                && let Some(wanted) = &expect.verdict
                && verdict != wanted
            {
                return Err(invalid(format!(
                    "L1 核查结论不符：{path} 的 verdict={verdict}，期望 {wanted}"
                )));
            }
            let Some(identity) = value
                .pointer("/bundle/identity")
                .and_then(serde_json::Value::as_str)
            else {
                return Err(invalid(format!("L1 核查记录缺少 bundle.identity：{path}")));
            };
            Ok(EvidenceOutcome {
                label: path.clone(),
                detail: format!("identity={identity}，verdict={verdict}"),
                identity: Some(identity.to_owned()),
            })
        }
    }
}

fn check_synth_summary(
    value: &serde_json::Value,
    binary_top: Option<&str>,
    binary_nested: Option<&str>,
    expect: Option<&SynthSummaryExpect>,
    path: &str,
) -> Result<(), Error> {
    let Some(expect) = expect else {
        return Ok(());
    };
    let get = |key: &str| value.get(key).and_then(serde_json::Value::as_str);
    for (key, wanted) in [
        ("format", expect.format.as_deref()),
        (
            "implementation_commit",
            expect.implementation_commit.as_deref(),
        ),
    ] {
        let Some(wanted) = wanted else { continue };
        let actual = get(key);
        if actual != Some(wanted) {
            return Err(invalid(format!(
                "身份不符：{path} 的 {key}={actual:?}，期望 {wanted}"
            )));
        }
    }
    // 二进制摘要期望匹配任一合法形状（顶层 `binary_sha256` 或嵌套
    // `binary.sha256`）；两者都没有而声明了期望即失败。
    if let Some(wanted) = expect.binary_sha256.as_deref()
        && binary_top != Some(wanted)
        && binary_nested != Some(wanted)
    {
        return Err(invalid(format!(
            "身份不符：{path} 的二进制摘要（顶层={binary_top:?}，嵌套={binary_nested:?}）与期望 {wanted} 不符"
        )));
    }
    if let Some(wanted) = expect.complete {
        let actual = value.get("complete").and_then(serde_json::Value::as_bool);
        if actual != Some(wanted) {
            return Err(invalid(format!(
                "身份不符：{path} 的 complete={actual:?}，期望 {wanted}"
            )));
        }
    }
    Ok(())
}

fn check_gate_json(
    status: Option<&str>,
    commit: Option<&str>,
    binary_digest: Option<&str>,
    expect: Option<&GateJsonExpect>,
    path: &str,
) -> Result<(), Error> {
    let Some(expect) = expect else {
        return Ok(());
    };
    for (key, wanted, actual) in [
        ("status", expect.status.as_deref(), status),
        ("source.commit", expect.commit.as_deref(), commit),
        (
            "source.binary.digest",
            expect.binary_digest.as_deref(),
            binary_digest,
        ),
    ] {
        let Some(wanted) = wanted else { continue };
        if actual != Some(wanted) {
            return Err(invalid(format!(
                "身份不符：{path} 的 {key}={actual:?}，期望 {wanted}"
            )));
        }
    }
    Ok(())
}

/// 解析 `key=value` 空格分隔的头行；重复键取首个（记录方按首行头定义）。
/// 墙钟证据入口的头行标记：§7.1 的 `LF-ROUTING-G2-LINEAR-v1` 与 #1267 的
/// LF-SYNTH 入口（#1279 运行）。
const ROUTING_HEADER_MARKERS: [&str; 2] = ["routing-g2-wall-clock", "routing-lfsynth-wall-clock"];

/// 每种头行必须由 `expect` 绑定的身份键。
fn routing_required_keys(marker: &str) -> &'static [&'static str] {
    if marker == "routing-lfsynth-wall-clock" {
        &["workload_id", "identity"]
    } else {
        &["workload_id"]
    }
}

/// 找出全部头行，返回（标记，从标记起至行尾）。
///
/// 头行定位容忍 libtest 前缀：`--nocapture` 单线程输出的首行会被 pretty
/// 格式化器冠以 `test <name> ... `，所以标记不必在行首，但前后都必须落在
/// 词边界上。同一前缀的逐操作统计行带 `operation=` 键，不算头行。
fn routing_headers(text: &str) -> Vec<(&'static str, &str)> {
    let mut headers = Vec::new();
    for line in text.lines() {
        for marker in ROUTING_HEADER_MARKERS {
            let Some(position) = line.find(marker) else {
                continue;
            };
            let rest = &line[position..];
            let starts_token = position == 0 || line[..position].ends_with(' ');
            let ends_token =
                rest[marker.len()..].is_empty() || rest[marker.len()..].starts_with(' ');
            if starts_token && ends_token && !parse_kv_header(rest).contains_key("operation") {
                headers.push((marker, rest));
            }
        }
    }
    headers
}

fn parse_kv_header(header: &str) -> std::collections::BTreeMap<&str, &str> {
    let mut map = std::collections::BTreeMap::new();
    for token in header.split_whitespace() {
        let Some((key, value)) = token.split_once('=') else {
            continue;
        };
        map.entry(key).or_insert(value);
    }
    map
}
