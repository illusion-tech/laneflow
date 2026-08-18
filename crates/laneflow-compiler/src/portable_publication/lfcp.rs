use laneflow_format::{
    FieldWriteInputV1, FieldWriteValueV1, FormatLimits, ObjectWriteInputV1,
    PostEmissionCheckedBundleV1, RowWriteInputV1, SectionWriteInputV1, TableWriteInputV1,
    encode_prepared_object_v1, preflight_object_values_v1, prepare_object_v1,
};
use laneflow_static_contract::{
    CANONICAL_ARTIFACT_FORMAT_VERSION, NETWORK_REVISION_DERIVATION_VERSION, PortableObjectKind,
    SOURCE_MAP_FORMAT_VERSION,
};

use crate::portable_emitter::{PortableObjectCandidate, close_object, object_key};

use super::{PortablePublicationError, PortablePublicationProvenanceV2};

pub(crate) fn build_lfcp_v2(
    checked: PostEmissionCheckedBundleV1<'_>,
    provenance: &PortablePublicationProvenanceV2,
    limits: FormatLimits,
) -> Result<PortableObjectCandidate, PortablePublicationError> {
    let artifact_fields = [
        field(1, FieldWriteValueV1::U16(CANONICAL_ARTIFACT_FORMAT_VERSION)),
        field(
            2,
            FieldWriteValueV1::U16(NETWORK_REVISION_DERIVATION_VERSION),
        ),
        field(
            3,
            FieldWriteValueV1::Sha256(checked.network_revision().into_digest().into_bytes()),
        ),
        field(
            4,
            FieldWriteValueV1::Sha256(checked.canonical_artifact_digest().into_bytes()),
        ),
        field(
            5,
            FieldWriteValueV1::U64(checked.canonical_artifact_byte_length().get()),
        ),
    ];
    let source_map_fields = [
        field(1, FieldWriteValueV1::U16(SOURCE_MAP_FORMAT_VERSION)),
        field(
            2,
            FieldWriteValueV1::Sha256(checked.source_map_digest().into_bytes()),
        ),
        field(
            3,
            FieldWriteValueV1::U64(checked.source_map_byte_length().get()),
        ),
        field(4, FieldWriteValueV1::Utf8(checked.compiler_build_id())),
        field(
            5,
            FieldWriteValueV1::U16(checked.source_collection_digest_version()),
        ),
        field(
            6,
            FieldWriteValueV1::Sha256(checked.source_collection_digest().into_bytes()),
        ),
    ];
    let artifact_object_key = object_key(checked.canonical_artifact_digest());
    let source_map_object_key = object_key(checked.source_map_digest());
    let mut publication_fields = Vec::with_capacity(6);
    publication_fields.extend([
        field(1, FieldWriteValueV1::U8(provenance.publisher_kind.code())),
        field(2, FieldWriteValueV1::Utf8(&provenance.publisher_build_id)),
        field(3, FieldWriteValueV1::Utf8(&artifact_object_key)),
        field(4, FieldWriteValueV1::Utf8(&source_map_object_key)),
    ]);
    if let Some(value) = provenance.controlled_build_provenance.as_deref() {
        publication_fields.push(field(5, FieldWriteValueV1::Utf8(value)));
    }
    if let Some(value) = provenance.controlled_timestamp.as_deref() {
        publication_fields.push(field(6, FieldWriteValueV1::Utf8(value)));
    }

    let artifact_rows = [RowWriteInputV1 {
        fields: &artifact_fields,
    }];
    let source_map_rows = [RowWriteInputV1 {
        fields: &source_map_fields,
    }];
    let publication_rows = [RowWriteInputV1 {
        fields: &publication_fields,
    }];
    let artifact_tables = [table(&artifact_rows)];
    let source_map_tables = [table(&source_map_rows)];
    let publication_tables = [table(&publication_rows)];
    let sections = [
        SectionWriteInputV1 {
            kind: 1,
            tables: &artifact_tables,
        },
        SectionWriteInputV1 {
            kind: 2,
            tables: &source_map_tables,
        },
        SectionWriteInputV1 {
            kind: 3,
            tables: &publication_tables,
        },
    ];
    let input = ObjectWriteInputV1 {
        kind: PortableObjectKind::CanonicalPublicationDescriptor,
        sections: &sections,
    };
    let prepared = prepare_object_v1(input, limits)?;
    let output_length = usize::try_from(prepared.byte_len())
        .map_err(|_| PortablePublicationError::ArithmeticOverflow)?;
    let mut bytes = vec![0_u8; output_length];
    encode_prepared_object_v1(prepared, &mut bytes)?;
    preflight_object_values_v1(
        &bytes,
        PortableObjectKind::CanonicalPublicationDescriptor,
        limits,
    )?;
    Ok(close_object(bytes.into_boxed_slice()))
}

const fn field(tag: u16, value: FieldWriteValueV1<'_>) -> FieldWriteInputV1<'_> {
    FieldWriteInputV1 { tag, value }
}

const fn table<'a>(rows: &'a [RowWriteInputV1<'a>]) -> TableWriteInputV1<'a> {
    TableWriteInputV1 { kind: 1, rows }
}
