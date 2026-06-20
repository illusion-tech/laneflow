//! v0.1 route sequence 与 validation 原语。

use crate::error::CoreError;

/// v0.1 最小 route edge sequence。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    id: String,
    edge_ids: Vec<String>,
}

impl Route {
    /// 创建并校验最小 route。edge 存在性和连通性由 `CoreWorld` 校验。
    pub fn try_new<I, S>(id: impl Into<String>, edge_ids: I) -> Result<Self, CoreError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let id = id.into();
        let edge_ids: Vec<String> = edge_ids.into_iter().map(Into::into).collect();

        if edge_ids.is_empty() {
            return Err(CoreError::EmptyRoute { route_id: id });
        }

        Ok(Self { id, edge_ids })
    }

    /// 返回 route id。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 返回 route edge sequence。
    pub fn edge_ids(&self) -> &[String] {
        &self.edge_ids
    }
}
