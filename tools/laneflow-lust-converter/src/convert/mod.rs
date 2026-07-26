//! Static conversion stages.

pub mod junction;
pub mod topology;

pub use topology::{TopologyConvertOptions, convert_network_topology};
