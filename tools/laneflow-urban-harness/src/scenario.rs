use crate::UrbanCase;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PopulationStrategy {
    Standard,
    GarageEgress,
    GarageIngress,
    AllActive,
}

impl PopulationStrategy {
    pub fn active_per_tile(self) -> u32 {
        match self {
            Self::AllActive => 1_000,
            Self::GarageEgress => 250,
            Self::Standard | Self::GarageIngress => 750,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DemandStrategy {
    FiniteDirectional,
    CompletedRecycling,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CorrectnessRoles {
    None,
    MixedPeak,
    GarageEgress,
    GarageIngress,
    WaitingRelease,
    PermissiveLeft,
    UncontrolledYield,
    BoundaryBurst,
}

pub(crate) struct ScenarioRecipe {
    pub population: PopulationStrategy,
    pub demand: DemandStrategy,
    pub roles: CorrectnessRoles,
}

impl ScenarioRecipe {
    pub fn for_case(case: UrbanCase) -> Self {
        use CorrectnessRoles as Roles;
        use PopulationStrategy as Population;
        let (population, demand, roles) = match case {
            UrbanCase::MixedPeak => (
                Population::Standard,
                DemandStrategy::FiniteDirectional,
                Roles::MixedPeak,
            ),
            UrbanCase::GarageEgress => (
                Population::GarageEgress,
                DemandStrategy::FiniteDirectional,
                Roles::GarageEgress,
            ),
            UrbanCase::GarageIngress => (
                Population::GarageIngress,
                DemandStrategy::FiniteDirectional,
                Roles::GarageIngress,
            ),
            UrbanCase::WaitingRelease => (
                Population::Standard,
                DemandStrategy::FiniteDirectional,
                Roles::WaitingRelease,
            ),
            UrbanCase::PermissiveLeft => (
                Population::Standard,
                DemandStrategy::FiniteDirectional,
                Roles::PermissiveLeft,
            ),
            UrbanCase::UncontrolledYield => (
                Population::Standard,
                DemandStrategy::FiniteDirectional,
                Roles::UncontrolledYield,
            ),
            UrbanCase::BoundaryBurst => (
                Population::Standard,
                DemandStrategy::FiniteDirectional,
                Roles::BoundaryBurst,
            ),
            UrbanCase::SustainedActive => (
                Population::AllActive,
                DemandStrategy::CompletedRecycling,
                Roles::None,
            ),
        };
        Self {
            population,
            demand,
            roles,
        }
    }
}
