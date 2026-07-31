//! #308 三个可扩展编译器工作负载的闭合身份。
//!
//! 协议身份来自受信任工作负载清单；本枚举只负责让二进制协议和执行器在类型层面拒绝
//! 当前固定夹具或未知字符串进入规模发现。

use crate::{CORRIDOR_WORKLOAD_ID, JUNCTION_GRID_WORKLOAD_ID};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

pub const WORKLOAD_REVISION_V1: u32 = 1;
pub const GENERATOR_VERSION_V1: u32 = 1;
pub const BASE_SCALE_STRING_PROFILE: &str = "short-unique-v1";
pub const BASELINE_CANDIDATE_ID: &str = "baseline-std-randomstate-stable-vec-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ScalableWorkloadId {
    #[serde(rename = "LF-COMP-ID-v1")]
    Identity,
    #[serde(rename = "LF-COMP-CORRIDOR-v1")]
    Corridor,
    #[serde(rename = "LF-COMP-JUNCTION-GRID-v1")]
    JunctionGrid,
}

impl ScalableWorkloadId {
    pub const ALL: [Self; 3] = [Self::Identity, Self::Corridor, Self::JunctionGrid];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Identity => crate::generator::IDENTITY_WORKLOAD_ID,
            Self::Corridor => CORRIDOR_WORKLOAD_ID,
            Self::JunctionGrid => JUNCTION_GRID_WORKLOAD_ID,
        }
    }
}

impl FromStr for ScalableWorkloadId {
    type Err = ScalableWorkloadParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            crate::generator::IDENTITY_WORKLOAD_ID => Ok(Self::Identity),
            CORRIDOR_WORKLOAD_ID => Ok(Self::Corridor),
            JUNCTION_GRID_WORKLOAD_ID => Ok(Self::JunctionGrid),
            _ => Err(ScalableWorkloadParseError(value.to_owned())),
        }
    }
}

pub fn validate_base_scale_contract(
    manifest: &serde_json::Value,
) -> Result<(), ScalableWorkloadContractError> {
    require_u64(
        manifest,
        "generatorVersion",
        u64::from(GENERATOR_VERSION_V1),
    )?;
    let candidate_registry = require_object(manifest, "candidateRegistry")?;
    let candidates = candidate_registry
        .get("candidates")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ScalableWorkloadContractError::Mismatch {
            path: "candidateRegistry/candidates".to_owned(),
        })?;
    let baseline = candidates
        .iter()
        .find(|candidate| {
            candidate.get("id").and_then(serde_json::Value::as_str) == Some(BASELINE_CANDIDATE_ID)
        })
        .ok_or_else(|| ScalableWorkloadContractError::Mismatch {
            path: format!("candidateRegistry/candidates/{BASELINE_CANDIDATE_ID}"),
        })?;
    require_string_array(
        baseline,
        "allowedKeyDomains",
        &["full-pipeline-baseline"],
        "candidateRegistry/candidates/baseline/allowedKeyDomains",
    )?;

    let selection = require_object(manifest, "scaleSelectionContract")?;
    require_u64(selection, "revision", 1)?;
    require_string(
        selection,
        "baselineCandidateId",
        BASELINE_CANDIDATE_ID,
        "scaleSelectionContract/baselineCandidateId",
    )?;
    let source_profiles = require_object(selection, "scaleSourceStringProfileByWorkload")?;
    let workloads = manifest
        .get("workloads")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ScalableWorkloadContractError::Mismatch {
            path: "workloads".to_owned(),
        })?;
    for workload_id in ScalableWorkloadId::ALL {
        require_string(
            source_profiles,
            workload_id.as_str(),
            BASE_SCALE_STRING_PROFILE,
            &format!(
                "scaleSelectionContract/scaleSourceStringProfileByWorkload/{}",
                workload_id.as_str()
            ),
        )?;
        let workload = workloads
            .iter()
            .find(|workload| {
                workload.get("id").and_then(serde_json::Value::as_str) == Some(workload_id.as_str())
            })
            .ok_or_else(|| ScalableWorkloadContractError::Mismatch {
                path: format!("workloads/{}", workload_id.as_str()),
            })?;
        if workload
            .get("scalable")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        {
            return Err(ScalableWorkloadContractError::Mismatch {
                path: format!("workloads/{}/scalable", workload_id.as_str()),
            });
        }
        require_string_array(
            workload,
            "graphProfiles",
            &["wide-star-v1", "deep-chain-v1", "shared-fanin-dag-v1"],
            &format!("workloads/{}/graphProfiles", workload_id.as_str()),
        )?;
        let profiles = workload
            .get("stringProfiles")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| ScalableWorkloadContractError::Mismatch {
                path: format!("workloads/{}/stringProfiles", workload_id.as_str()),
            })?;
        if !profiles
            .iter()
            .any(|profile| profile.as_str() == Some(BASE_SCALE_STRING_PROFILE))
        {
            return Err(ScalableWorkloadContractError::Mismatch {
                path: format!("workloads/{}/stringProfiles", workload_id.as_str()),
            });
        }
    }
    Ok(())
}

fn require_object<'a>(
    value: &'a serde_json::Value,
    field: &str,
) -> Result<&'a serde_json::Value, ScalableWorkloadContractError> {
    value
        .get(field)
        .filter(|value| value.is_object())
        .ok_or_else(|| ScalableWorkloadContractError::Mismatch {
            path: field.to_owned(),
        })
}

fn require_u64(
    value: &serde_json::Value,
    field: &str,
    expected: u64,
) -> Result<(), ScalableWorkloadContractError> {
    if value.get(field).and_then(serde_json::Value::as_u64) != Some(expected) {
        return Err(ScalableWorkloadContractError::Mismatch {
            path: field.to_owned(),
        });
    }
    Ok(())
}

fn require_string(
    value: &serde_json::Value,
    field: &str,
    expected: &str,
    path: &str,
) -> Result<(), ScalableWorkloadContractError> {
    if value.get(field).and_then(serde_json::Value::as_str) != Some(expected) {
        return Err(ScalableWorkloadContractError::Mismatch {
            path: path.to_owned(),
        });
    }
    Ok(())
}

fn require_string_array(
    value: &serde_json::Value,
    field: &str,
    expected: &[&str],
    path: &str,
) -> Result<(), ScalableWorkloadContractError> {
    let actual = value
        .get(field)
        .and_then(serde_json::Value::as_array)
        .and_then(|values| {
            values
                .iter()
                .map(serde_json::Value::as_str)
                .collect::<Option<Vec<_>>>()
        });
    if actual.as_deref() != Some(expected) {
        return Err(ScalableWorkloadContractError::Mismatch {
            path: path.to_owned(),
        });
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("未知可扩展编译器工作负载 {0:?}")]
pub struct ScalableWorkloadParseError(String);

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ScalableWorkloadContractError {
    #[error("基础规模工作负载契约字段不匹配：{path}")]
    Mismatch { path: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_workload_identity_round_trips_exact_protocol_ids() {
        for workload in ScalableWorkloadId::ALL {
            assert_eq!(
                workload.as_str().parse::<ScalableWorkloadId>(),
                Ok(workload)
            );
            assert_eq!(
                serde_json::to_string(&workload).expect("serialize workload"),
                format!("\"{}\"", workload.as_str())
            );
        }
        assert!(
            "LF-COMP-RESEARCH-CURRENT-FIXTURES-v1"
                .parse::<ScalableWorkloadId>()
                .is_err()
        );
    }

    #[test]
    fn base_scale_identity_is_bound_to_the_trusted_manifest() {
        let trusted = crate::load_repository_contract().expect("trusted contract");
        validate_base_scale_contract(&trusted.workload_manifest).expect("base-scale contract");

        let mut changed = trusted.workload_manifest.clone();
        changed["scaleSelectionContract"]["baselineCandidateId"] =
            serde_json::json!("different-baseline");
        assert!(matches!(
            validate_base_scale_contract(&changed),
            Err(ScalableWorkloadContractError::Mismatch { path })
                if path == "scaleSelectionContract/baselineCandidateId"
        ));
    }
}
