use std::path::PathBuf;

/// LuST converter / source verification failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to read or write {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid LuST converter TOML: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("invalid LuST converter configuration: {0}")]
    Config(String),

    #[error("LuST source file missing: {relative_path} (expected under {source_dir})")]
    MissingSourceFile {
        source_dir: PathBuf,
        relative_path: &'static str,
    },

    #[error(
        "LuST source size mismatch for {relative_path}: expected {expected} bytes, got {actual}"
    )]
    SourceSizeMismatch {
        relative_path: &'static str,
        expected: u64,
        actual: u64,
    },

    #[error("LuST source SHA-256 mismatch for {relative_path}: expected {expected}, got {actual}")]
    SourceDigestMismatch {
        relative_path: &'static str,
        expected: &'static str,
        actual: String,
    },

    #[error("failed to parse SUMO XML: {0}")]
    XmlParse(String),

    #[error("invalid SUMO network model: {0}")]
    SumoModel(String),

    #[error("could not serialize {document} JSON: {source}")]
    Json {
        document: &'static str,
        #[source]
        source: serde_json::Error,
    },

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

    #[error("static conversion is not implemented yet (source verify passed)")]
    StaticConversionNotImplemented,
}

/// Convenience result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;
