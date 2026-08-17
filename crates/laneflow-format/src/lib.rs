#![no_std]
#![doc = include_str!("../README.md")]

#[cfg(test)]
extern crate std;

mod error;
mod framing;
mod limits;
mod object;
mod table;
mod value;
mod wire;
mod writer;

pub use error::{FormatError, FormatErrorClass, FormatStructure, LimitDimension};
pub use framing::{ObjectFramingView, SectionFramingView, preflight_object_framing};
pub use limits::{FormatLimitConfig, FormatLimits};
pub use object::{
    RegistryCheckedFieldIter, RegistryCheckedFieldValue, RegistryCheckedFieldView,
    RegistryCheckedObjectView, RegistryCheckedOrdinalVectorView, RegistryCheckedRecordVectorView,
    RegistryCheckedRowIter, RegistryCheckedRowView, RegistryCheckedSectionIter,
    RegistryCheckedSectionView, RegistryCheckedTableIter, RegistryCheckedTableView,
    preflight_object_registry_v1,
};
pub use table::{TableStructureSummary, preflight_table_structure_v1};
pub use value::{ValueCheckedObjectView, preflight_object_values_v1};
pub use writer::{
    FieldWriteInputV1, FieldWriteValueV1, ObjectWriteInputV1, PreparedObjectV1, RowWriteInputV1,
    SectionWriteInputV1, TableWriteInputV1, encode_object_v1, encode_prepared_object_v1,
    measure_object_v1, prepare_object_v1,
};
