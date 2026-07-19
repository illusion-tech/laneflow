//! External package loader 的结构化错误。

use laneflow_core::CoreError;
use serde_json::error::Category;

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
    /// Vehicle Profile model 不是当前 v0.5 支持的 `iidm`。
    #[error("Vehicle Profile `{profile_id}` 使用不支持的 model：path={path}, actual=`{actual}`")]
    UnsupportedVehicleProfileModel {
        path: String,
        profile_id: String,
        actual: String,
    },
    /// wire package 在转换为 Core types 时违反 domain invariant。
    #[error("Core domain validation 失败：path={path}：{source}")]
    CoreDomain {
        path: String,
        #[source]
        source: CoreError,
    },
}

impl DataError {
    pub(crate) fn from_path_error(error: serde_path_to_error::Error<serde_json::Error>) -> Self {
        let path = normalize_path(error.path().to_string());
        let source = error.into_inner();
        let category = source.classify();
        Self::from_json_error(path, source, category)
    }

    pub(crate) fn from_json_error(
        path: String,
        source: serde_json::Error,
        category: Category,
    ) -> Self {
        let line = source.line();
        let column = source.column();
        match category {
            Category::Data => Self::JsonShape {
                path,
                line,
                column,
                source,
            },
            Category::Io | Category::Syntax | Category::Eof => Self::JsonSyntax {
                path,
                line,
                column,
                source,
            },
        }
    }

    pub(crate) fn core(path: impl Into<String>, source: CoreError) -> Self {
        Self::CoreDomain {
            path: path.into(),
            source,
        }
    }
}

fn normalize_path(path: String) -> String {
    if path.is_empty() || path == "." {
        "$".to_owned()
    } else {
        path
    }
}
