//! v0.1 route sequence 与 validation 原语。

use crate::{error::CoreError, handle::RouteHandle, id::validate_external_id};

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
        validate_external_id("routes[].id", &id)?;
        let edge_ids: Vec<String> = edge_ids.into_iter().map(Into::into).collect();

        if edge_ids.is_empty() {
            return Err(CoreError::EmptyRoute { route_id: id });
        }

        for edge_id in &edge_ids {
            validate_external_id("routes[].edges[]", edge_id)?;
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

/// route definition 被移除时返回的生命周期记录。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteRemoveRecord {
    /// 被移除的 route handle。
    pub handle: RouteHandle,
    /// 被移除的 route external ID。
    pub external_id: String,
}
