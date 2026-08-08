//! External package loader 的结构化错误。

use laneflow_core::CoreError;
use laneflow_current_source::{
    CurrentSourceError, CurrentSourceErrorPayload, CurrentSourceIssueContext,
};

/// LaneFlow data package 解析、版本与 Core normalization 错误。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DataError {
    /// JSON token、UTF-8、EOF 或 trailing content 无效。
    #[error("JSON syntax 无效：path={path}, line={line}, column={column}：{source}")]
    JsonSyntax {
        path: String,
        line: usize,
        column: usize,
        #[source]
        source: serde_json::Error,
    },
    /// JSON 字段缺失、类型错误或包含 unknown field。
    #[error("JSON shape 无效：path={path}, line={line}, column={column}：{source}")]
    JsonShape {
        path: String,
        line: usize,
        column: usize,
        #[source]
        source: serde_json::Error,
    },
    /// `formatVersion` 不是当前 loader 支持的版本。
    #[error("不支持 data format version：expected=`{expected}`, actual=`{actual}`")]
    UnsupportedFormatVersion {
        expected: &'static str,
        actual: String,
    },
    /// 声明单位不是当前格式要求的单位。
    #[error("单位无效：path={path}, expected=`{expected}`, actual=`{actual}`")]
    InvalidUnit {
        path: &'static str,
        expected: &'static str,
        actual: String,
    },
    /// Vehicle Profile model 不是当前 v0.10 支持的 `iidm`。
    #[error("Vehicle Profile `{profile_id}` 使用不支持的 model：path={path}, actual=`{actual}`")]
    UnsupportedVehicleProfileModel {
        path: String,
        profile_id: String,
        actual: String,
    },
    /// Vehicle Profile 的 participantClassId 必须引用已声明的 ParticipantClass。
    #[error(
        "Vehicle Profile `{profile_id}` 引用了不存在的 ParticipantClass：path={path}, classId=`{class_id}`"
    )]
    UnknownVehicleProfileParticipantClass {
        path: String,
        profile_id: String,
        class_id: String,
    },
    /// wire package 在转换为 Core types 时违反 domain invariant。
    #[error("Core domain validation 失败：path={path}：{source}")]
    CoreDomain {
        path: String,
        #[source]
        source: Box<CoreError>,
    },
}

impl DataError {
    /// 把 Traffic-only source 错误映射回现有 loader 错误形状；line/column 取自
    /// 立即失败的原始 `serde_json::Error`。
    pub(crate) fn from_current_source(error: CurrentSourceError) -> Self {
        let issues = error.into_issues();
        debug_assert_eq!(issues.len(), 1, "production-compatible source 立即失败");
        let issue = issues
            .into_iter()
            .next()
            .expect("CurrentSourceError 至少含一项 issue");
        let (_document, context, path, payload) = issue.into_parts().into_components();
        debug_assert!(
            matches!(context, CurrentSourceIssueContext::None),
            "Traffic-only 能力不带 scenario 上下文"
        );
        Self::from_traffic_payload(path, payload)
    }

    /// 把 Traffic wire/version payload 映射为现有 `DataError` variant。
    pub(crate) fn from_traffic_payload(path: String, payload: CurrentSourceErrorPayload) -> Self {
        match payload {
            CurrentSourceErrorPayload::JsonSyntax { source } => Self::JsonSyntax {
                path,
                line: source.line(),
                column: source.column(),
                source,
            },
            CurrentSourceErrorPayload::JsonShape { source } => Self::JsonShape {
                path,
                line: source.line(),
                column: source.column(),
                source,
            },
            CurrentSourceErrorPayload::UnsupportedFormatVersion { expected, actual } => {
                Self::UnsupportedFormatVersion {
                    expected,
                    actual: actual.into_string(),
                }
            }
            payload => unreachable!(
                "Traffic wire 校验只产生 JSON/version payload：{}",
                payload.stable_code()
            ),
        }
    }

    pub(crate) fn core(path: impl Into<String>, source: CoreError) -> Self {
        Self::CoreDomain {
            path: path.into(),
            source: Box::new(source),
        }
    }
}
