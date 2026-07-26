//! Static conversion stages.

pub mod junction;
pub mod signals;
pub mod topology;

pub use topology::{
    TopologyConvertOptions, convert_network_topology, convert_network_topology_with_tll,
};
