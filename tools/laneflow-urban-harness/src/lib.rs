//! Caller-owned, finite LF-CN-URBAN demand and replay. Road authority stays in TrafficWorld.

mod artifacts;
mod observe;
mod plan;
mod report;
mod runner;

pub use artifacts::Artifacts;
pub use plan::{
    DepartureBatch, InitialVehicle, ParkingArrival, ParkingDeparture, ResolvedPlan, Window,
};
pub use report::{ComparedRun, ComparisonReport, RunResult, compare_runs, run_to_directory};
pub use runner::{Harness, IndividualId, TickRecord};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Decode(#[from] toml::de::Error),
    #[error(transparent)]
    Encode(#[from] toml::ser::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Validation(String),
}

pub(crate) fn invalid(message: impl Into<String>) -> Error {
    Error::Validation(message.into())
}

pub(crate) fn checked<T, E: std::fmt::Debug>(
    context: &str,
    result: std::result::Result<T, E>,
) -> Result<T> {
    result.map_err(|e| invalid(format!("{context}: {e:?}")))
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex(&Sha256::digest(bytes))
}

pub(crate) fn catalog_id<K: laneflow_static_contract::EntityKindMarker>(
    bare_hex: &str,
) -> Result<laneflow_static_contract::StableId<K>> {
    checked(
        "catalog identity",
        format!("lfid1_{}_{bare_hex}", K::KIND.slug()).parse(),
    )
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
