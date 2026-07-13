#![doc = include_str!("../README.md")]

mod error;
mod wire;

use laneflow_core::{
    EdgeLength, IidmProfileSpec, InitialTrafficData, LaneEdge, LaneGraph, Route, VehicleProfile,
    VehicleProfileRegistry,
};
use serde::de::DeserializeOwned;
use serde_json::error::Category;

pub use error::DataError;

use wire::{WirePackage, WireVersionHeader};

/// 当前 production loader 接受的唯一 data format 版本。
pub const CURRENT_FORMAT_VERSION: &str = "0.3";

/// 已解析并完成 Core normalization 的当前 data package。
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedPackage {
    initial_traffic_data: InitialTrafficData,
}

impl LoadedPackage {
    /// 返回已验证的 Core 初始交通输入。
    pub const fn initial_traffic_data(&self) -> &InitialTrafficData {
        &self.initial_traffic_data
    }

    /// 消费 loaded package 并返回 Core 初始交通输入。
    pub fn into_initial_traffic_data(self) -> InitialTrafficData {
        self.initial_traffic_data
    }
}

/// 从 UTF-8 JSON bytes 解析并完成 Core normalization。
///
/// # Errors
///
/// JSON syntax/shape、format version、units 或 Core domain validation 失败时返回
/// 结构化 `DataError`。本函数不读取文件，也不返回部分初始化结果。
pub fn from_json_slice(input: &[u8]) -> Result<LoadedPackage, DataError> {
    let header: WireVersionHeader = deserialize_json(input)?;
    if header.format_version != CURRENT_FORMAT_VERSION {
        return Err(DataError::UnsupportedFormatVersion {
            expected: CURRENT_FORMAT_VERSION,
            actual: header.format_version,
        });
    }

    let wire: WirePackage = deserialize_json(input)?;
    debug_assert_eq!(wire.format_version, header.format_version);
    normalize(wire)
}

fn deserialize_json<T>(input: &[u8]) -> Result<T, DataError>
where
    T: DeserializeOwned,
{
    let mut deserializer = serde_json::Deserializer::from_slice(input);
    let value =
        serde_path_to_error::deserialize(&mut deserializer).map_err(DataError::from_path_error)?;
    deserializer
        .end()
        .map_err(|source| DataError::from_json_error("$".to_owned(), source, Category::Syntax))?;
    Ok(value)
}

/// 从 UTF-8 JSON string 解析并完成 Core normalization。
///
/// # Errors
///
/// 与 `from_json_slice` 相同。
pub fn from_json_str(input: &str) -> Result<LoadedPackage, DataError> {
    from_json_slice(input.as_bytes())
}

fn normalize(wire: WirePackage) -> Result<LoadedPackage, DataError> {
    validate_unit("units.distance", "meter", &wire.units.distance)?;
    validate_unit("units.time", "second", &wire.units.time)?;

    let mut normalized_profiles = Vec::with_capacity(wire.vehicle_profiles.len());
    for (index, profile) in wire.vehicle_profiles.iter().enumerate() {
        if profile.model != "iidm" {
            return Err(DataError::UnsupportedVehicleProfileModel {
                path: format!("vehicleProfiles[{index}].model"),
                profile_id: profile.id.clone(),
                actual: profile.model.clone(),
            });
        }

        let spec = IidmProfileSpec {
            length: profile.length,
            desired_speed: profile.desired_speed,
            min_gap: profile.min_gap,
            time_headway: profile.time_headway,
            max_acceleration: profile.max_acceleration,
            comfortable_deceleration: profile.comfortable_deceleration,
            emergency_deceleration: profile.emergency_deceleration,
        };
        let normalized = VehicleProfile::try_new_iidm(profile.id.clone(), spec)
            .map_err(|source| DataError::core(format!("vehicleProfiles[{index}]"), source))?;
        normalized_profiles.push(normalized);
    }
    let profile_registry = VehicleProfileRegistry::try_new(normalized_profiles)
        .map_err(|source| DataError::core("vehicleProfiles", source))?;

    let initial_traffic_data = normalize_core_data(wire, profile_registry)?;
    Ok(LoadedPackage {
        initial_traffic_data,
    })
}

fn normalize_core_data(
    wire: WirePackage,
    profile_registry: VehicleProfileRegistry,
) -> Result<InitialTrafficData, DataError> {
    let mut edges = Vec::with_capacity(wire.lane_graph.edges.len());
    for (index, edge) in wire.lane_graph.edges.into_iter().enumerate() {
        let length = EdgeLength::try_new(edge.length).map_err(|source| {
            DataError::core(format!("laneGraph.edges[{index}].length"), source)
        })?;
        edges.push(LaneEdge::new(
            edge.id,
            length,
            edge.connections.into_iter().map(|connection| connection.to),
        ));
    }
    let lane_graph =
        LaneGraph::try_new(edges).map_err(|source| DataError::core("laneGraph.edges", source))?;

    let mut routes = Vec::with_capacity(wire.routes.len());
    for (index, route) in wire.routes.into_iter().enumerate() {
        let normalized = Route::try_new(route.id, route.edges)
            .map_err(|source| DataError::core(format!("routes[{index}]"), source))?;
        routes.push(normalized);
    }

    InitialTrafficData::try_new(lane_graph, routes, profile_registry)
        .map_err(|source| DataError::core("initialTrafficData", source))
}

fn validate_unit(
    path: &'static str,
    expected: &'static str,
    actual: &str,
) -> Result<(), DataError> {
    if actual == expected {
        Ok(())
    } else {
        Err(DataError::InvalidUnit {
            path,
            expected,
            actual: actual.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn package_name_matches_data_crate_boundary() {
        assert_eq!(env!("CARGO_PKG_NAME"), "laneflow-data");
    }
}
