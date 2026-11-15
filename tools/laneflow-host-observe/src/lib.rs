//! 测量环境记录、进程快照与子进程观测（`docs/design/host-observe.md`）。
//!
//! 全部接口都可能分配内存、枚举进程或启动外部命令，只能在计时区间外调用。
//! 帧间高频计数器由 `laneflow-os-probe` 提供，不在本 crate 范围内。
//!
//! - [`environment`]：带版本号的环境记录（§3）；
//! - [`snapshot`]：本机全部进程的两次读数快照（§4）；
//! - [`child`]：按身份追踪子进程及其后代、退出汇总与按身份终止（§5）。

#![deny(missing_docs)]

use serde::{Deserialize, Serialize};

pub mod child;
mod command;
pub mod environment;
pub mod snapshot;
mod time;

#[cfg(test)]
mod tests;

/// 可能不可用的字段：取得时为 `{"value": …}`，取不到时为
/// `{"unavailable": "<原因>"}`。不以空串、零或省略表示不可用。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Field<T> {
    /// 取得的值。
    Value(T),
    /// 不可用及其原因。
    Unavailable(String),
}

impl<T> Field<T> {
    /// 取得的值；不可用时为 `None`。
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Unavailable(_) => None,
        }
    }

    pub(crate) fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable(reason.into())
    }

    pub(crate) fn from_result<E: std::fmt::Display>(result: Result<T, E>, context: &str) -> Self {
        match result {
            Ok(value) => Self::Value(value),
            Err(error) => Self::Unavailable(format!("{context}：{error}")),
        }
    }

    pub(crate) fn from_option(value: Option<T>, reason: &str) -> Self {
        value.map_or_else(|| Self::unavailable(reason), Self::Value)
    }
}
