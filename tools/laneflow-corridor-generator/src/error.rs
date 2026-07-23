use std::path::PathBuf;

/// Corridor authoring and validation failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to read or write {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid corridor TOML: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("could not serialize corridor catalog TOML: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("could not serialize {document} JSON: {source}")]
    Json {
        document: &'static str,
        #[source]
        source: serde_json::Error,
    },

    #[error("invalid corridor configuration: {0}")]
    Config(String),

    #[error("{document} does not satisfy its repository JSON Schema: {message}")]
    Schema {
        document: &'static str,
        message: String,
    },

    #[error("{stage} validation failed: {message}")]
    Validation {
        stage: &'static str,
        message: String,
    },

    #[error("catalog validation failed: {0}")]
    Catalog(String),

    #[error("generated output differs from checked-in file {path}: {detail}")]
    OutputMismatch { path: PathBuf, detail: String },
}

pub(crate) trait IoResultExt<T> {
    fn at(self, path: impl Into<PathBuf>) -> Result<T, Error>;
}

impl<T> IoResultExt<T> for Result<T, std::io::Error> {
    fn at(self, path: impl Into<PathBuf>) -> Result<T, Error> {
        self.map_err(|source| Error::Io {
            path: path.into(),
            source,
        })
    }
}
