//! Static conversion stages.

#[cfg(test)]
mod acceptance;
pub mod junction;
pub mod population;
pub mod profiles;
pub mod routes;
pub mod signals;
pub mod topology;

pub(crate) use topology::DEFAULT_FIXED_DELTA_MS;
pub use topology::TopologyConvertOptions;
