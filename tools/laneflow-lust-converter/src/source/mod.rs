//! Pinned LuST source constants and verification.

pub mod pinned;
pub mod verify;

pub use pinned::{LUST_COMMIT, LUST_REPOSITORY, LUST_TAG, PINNED_SOURCE_FILES, PinnedSourceFile};
pub(crate) use verify::read_verified;
pub use verify::{
    VerifiedLustInputs, VerifiedSourceFile, VerifiedSourceSet, prepare_verified_lust_inputs,
    verify_source_dir,
};
