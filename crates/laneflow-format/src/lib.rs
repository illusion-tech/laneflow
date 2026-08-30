#![no_std]
#![doc = include_str!("../README.md")]

#[cfg(any(feature = "std", test))]
extern crate std;

mod canonical_network;
mod error;
mod framing;
mod limits;
mod object;
mod object_source;
mod post_emission;
#[cfg(test)]
mod security_tests;
#[cfg(feature = "std")]
mod staged_object;
mod table;
mod value;
mod wire;
mod writer;

pub use canonical_network::{
    CanonicalNetworkInputError, CheckedCanonicalNetworkInput, check_canonical_network_input,
};
pub use error::{FormatError, FormatErrorClass, FormatStructure, LimitDimension};
pub use framing::{ObjectFramingView, SectionFramingView, preflight_object_framing};
pub use limits::{FormatLimitConfig, FormatLimits};
pub use object::{
    RegistryCheckedFieldIter, RegistryCheckedFieldValue, RegistryCheckedFieldView,
    RegistryCheckedObjectView, RegistryCheckedOrdinalVectorView, RegistryCheckedRecordVectorView,
    RegistryCheckedRowIter, RegistryCheckedRowView, RegistryCheckedSectionIter,
    RegistryCheckedSectionView, RegistryCheckedTableIter, RegistryCheckedTableView,
    preflight_object_registry,
};
pub use object_source::{BoundedReReadableObjectSource, ObjectSourceError};
pub use post_emission::{
    ExpectedSemanticDiffBase, PostEmissionCheckError, PostEmissionCheckedBundle,
    check_post_emission_bundle,
};
#[cfg(feature = "std")]
pub use staged_object::{
    ClosedStagedObjectSource, ImmutableObjectSource, StagedObjectError, StagedObjectWriter,
};
pub use table::{TableStructureSummary, preflight_table_structure};
pub use value::{ValueCheckedObjectView, preflight_object_values};
pub use writer::{
    FieldWriteInput, FieldWriteValue, ObjectWriteInput, PreparedObject, RowWriteInput,
    SectionWriteInput, TableWriteInput, encode_object, encode_prepared_object, measure_object,
    prepare_object,
};
