use laneflow_format::{
    FieldWriteInput, FieldWriteValue, FormatLimits, ObjectWriteInput,
    PostEmissionCheckedBundle, RowWriteInput, SectionWriteInput, TableWriteInput,
    encode_prepared_object, preflight_object_values, prepare_object,
};
use laneflow_static_contract::{
    CANONICAL_ARTIFACT_FORMAT_VERSION, NETWORK_REVISION_DERIVATION_VERSION, PortableObjectKind,
    SOURCE_MAP_FORMAT_VERSION,
};

use crate::portable_emitter::{PortableObjectCandidate, close_object, object_key};

use super::{PortablePublicationError, PortablePublicationProvenance};

pub(crate) fn build_lfcp(
    checked: PostEmissionCheckedBundle<'_>,
    provenance: &PortablePublicationProvenance,
    limits: FormatLimits,
) -> Result<PortableObjectCandidate, PortablePublicationError> {
    let artifact_fields = [
        field(1, FieldWriteValue::U16(CANONICAL_ARTIFACT_FORMAT_VERSION)),
        field(
            2,
            FieldWriteValue::U16(NETWORK_REVISION_DERIVATION_VERSION),
        ),
        field(
            3,
            FieldWriteValue::Sha256(checked.network_revision().into_digest().into_bytes()),
        ),
        field(
            4,
            FieldWriteValue::Sha256(checked.canonical_artifact_digest().into_bytes()),
        ),
        field(
            5,
            FieldWriteValue::U64(checked.canonical_artifact_byte_length().get()),
        ),
    ];
    let source_map_fields = [
        field(1, FieldWriteValue::U16(SOURCE_MAP_FORMAT_VERSION)),
        field(
            2,
            FieldWriteValue::Sha256(checked.source_map_digest().into_bytes()),
        ),
        field(
            3,
            FieldWriteValue::U64(checked.source_map_byte_length().get()),
        ),
        field(4, FieldWriteValue::Utf8(checked.compiler_build_id())),
        field(
            5,
            FieldWriteValue::U16(checked.source_collection_digest_version()),
        ),
        field(
            6,
            FieldWriteValue::Sha256(checked.source_collection_digest().into_bytes()),
        ),
    ];
    let artifact_object_key = object_key(checked.canonical_artifact_digest());
    let source_map_object_key = object_key(checked.source_map_digest());
    let mut publication_fields = Vec::with_capacity(6);
    publication_fields.extend([
        field(1, FieldWriteValue::U8(provenance.publisher_kind.code())),
        field(2, FieldWriteValue::Utf8(&provenance.publisher_build_id)),
        field(3, FieldWriteValue::Utf8(&artifact_object_key)),
        field(4, FieldWriteValue::Utf8(&source_map_object_key)),
    ]);
    if let Some(value) = provenance.controlled_build_provenance.as_deref() {
        publication_fields.push(field(5, FieldWriteValue::Utf8(value)));
    }
    if let Some(value) = provenance.controlled_timestamp.as_deref() {
        publication_fields.push(field(6, FieldWriteValue::Utf8(value)));
    }

    let artifact_rows = [RowWriteInput {
        fields: &artifact_fields,
    }];
    let source_map_rows = [RowWriteInput {
        fields: &source_map_fields,
    }];
    let publication_rows = [RowWriteInput {
        fields: &publication_fields,
    }];
    let artifact_tables = [table(&artifact_rows)];
    let source_map_tables = [table(&source_map_rows)];
    let publication_tables = [table(&publication_rows)];
    let sections = [
        SectionWriteInput {
            kind: 1,
            tables: &artifact_tables,
        },
        SectionWriteInput {
            kind: 2,
            tables: &source_map_tables,
        },
        SectionWriteInput {
            kind: 3,
            tables: &publication_tables,
        },
    ];
    let input = ObjectWriteInput {
        kind: PortableObjectKind::CanonicalPublicationDescriptor,
        sections: &sections,
    };
    let prepared = prepare_object(input, limits)?;
    let output_length = usize::try_from(prepared.byte_len())
        .map_err(|_| PortablePublicationError::ArithmeticOverflow)?;
    let mut bytes = vec![0_u8; output_length];
    encode_prepared_object(prepared, &mut bytes)?;
    preflight_object_values(
        &bytes,
        PortableObjectKind::CanonicalPublicationDescriptor,
        limits,
    )?;
    Ok(close_object(bytes.into_boxed_slice()))
}

const fn field(tag: u16, value: FieldWriteValue<'_>) -> FieldWriteInput<'_> {
    FieldWriteInput { tag, value }
}

const fn table<'a>(rows: &'a [RowWriteInput<'a>]) -> TableWriteInput<'a> {
    TableWriteInput { kind: 1, rows }
}
