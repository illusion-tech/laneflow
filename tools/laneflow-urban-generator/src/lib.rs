mod catalog;
mod compare;
mod config;
mod delivery;
mod layout;
mod source;

pub use catalog::{AnchorRoute, Catalog, ParkingTarget, Route};
pub use compare::compare_artifacts;
pub use config::{ProfileConfig, Scale, SignalConfig, UrbanConfig};
pub use delivery::{Manifest, generate};
pub use layout::{Cell, Direction, Layout, Template};
pub use source::{GeneratedSource, generate_source};

pub fn compile_source(source: &GeneratedSource) -> Result<laneflow_compiler::CompilationOutput> {
    use laneflow_compiler::road_editing::RoadEditingModuleInput;
    let mut unit = laneflow_compiler::CompilationUnitBuilder::new(
        laneflow_compiler::CompileLimits::single_network_1m_v2(),
    );
    for (document, bytes) in [
        (source::COMMON_DOCUMENT_KEY, source.common.as_bytes()),
        (source::DOCUMENT_KEY, source.topology.as_bytes()),
    ] {
        unit.add_road_editing_module(
            RoadEditingModuleInput::try_new(document, bytes, None)
                .map_err(|error| validation("source buffer", error))?,
        )?;
    }
    laneflow_compiler::Compiler::new()
        .compile(unit.build()?)
        .map_err(|bundle| Error::Compile(diagnostics(&bundle)))
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid configuration: {0}")]
    Config(String),
    #[error("source construction: {0}")]
    Source(String),
    #[error("compilation: {0}")]
    Compile(String),
    #[error("{stage}: {detail}")]
    Validation { stage: &'static str, detail: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Decode(#[from] toml::de::Error),
    #[error(transparent)]
    Encode(#[from] toml::ser::Error),
}

impl From<laneflow_compiler::DiagnosticBundle> for Error {
    fn from(value: laneflow_compiler::DiagnosticBundle) -> Self {
        Self::Source(diagnostics(&value))
    }
}

pub type Result<T> = std::result::Result<T, Error>;

fn validation(stage: &'static str, error: impl std::fmt::Debug) -> Error {
    Error::Validation {
        stage,
        detail: format!("{error:?}"),
    }
}

fn diagnostics(bundle: &laneflow_compiler::DiagnosticBundle) -> String {
    bundle
        .diagnostics()
        .iter()
        .map(|d| {
            format!(
                "{} {:?}: {:?}",
                d.code().as_str(),
                d.stable_key(),
                d.payload()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
