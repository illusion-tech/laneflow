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

pub use error::{FormatError, FormatErrorClass, FormatStructure, LimitDimension};
pub use framing::{ObjectFramingView, SectionFramingView, preflight_object_framing};
pub use limits::{FormatLimitConfig, FormatLimits};
pub use object::{
    RegistryCheckedFieldValue, RegistryCheckedFieldView, RegistryCheckedObjectView,
    RegistryCheckedOrdinalVectorView, RegistryCheckedRecordVectorView, RegistryCheckedRowView,
    RegistryCheckedSectionView, RegistryCheckedTableView, preflight_object_registry_v1,
};
pub use table::{TableStructureSummary, preflight_table_structure_v1};
pub use value::{ValueCheckedObjectView, preflight_object_values_v1};
