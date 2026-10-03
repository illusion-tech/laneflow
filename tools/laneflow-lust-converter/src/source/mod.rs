//! Pinned LuST source constants and verification.

pub mod pinned;
pub mod verify;

pub use pinned::{LUST_COMMIT, LUST_REPOSITORY, LUST_TAG, PINNED_SOURCE_FILES};
pub(crate) use verify::read_verified;
pub use verify::{VerifiedSourceSet, recheck_source_revision, verify_source_dir};
