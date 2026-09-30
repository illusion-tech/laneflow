//! Static conversion stages.

pub mod junction;
pub mod population;
pub mod profiles;
pub mod routes;
pub mod signals;
pub mod topology;

pub use junction::{
    StubWeldDisposition, StubWeldRecord, scan_stub_weld_candidates, stub_weld_manifest_json,
};
pub use population::{
    POPULATION_CANDIDATE_COUNT, POPULATION_DEPART_END_SECONDS, POPULATION_DEPART_START_SECONDS,
    POPULATION_SELECTED_COUNT, PopulationRecord, select_population,
};
pub(crate) use profiles::convert_vehicle_profiles;
pub use profiles::{LUST_PASSENGER_VTYPE_IDS, select_passenger_vtypes};
pub(crate) use topology::DEFAULT_FIXED_DELTA_MS;
pub use topology::{
    StaticConversionArtifacts, TopologyConvertOptions, convert_network_topology,
    convert_network_topology_with_tll,
};
pub(crate) use topology::{
    convert_network_topology_with_tll_and_profiles, convert_static_with_due,
};
