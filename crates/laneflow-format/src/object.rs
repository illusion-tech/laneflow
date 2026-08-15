//! 完整对象的附录 A registry 结构预检。

use laneflow_static_contract::{PortableObjectKind, portable_object_schema};

use crate::{
    FormatError, FormatLimits, FormatStructure, ObjectFramingView, SectionFramingView,
    framing::preflight_object_framing,
    table::{PreflightBudget, preflight_table_with_registry_v1},
    wire::{checked_slice, read_u16, read_u32, read_u64},
};

const SECTION_HEADER_BYTES: u64 = 4;
const TABLE_HEADER_BYTES: u64 = 16;

/// 已完成前导、目录和附录 A section/table/field registry 结构预检的对象借用。
///
/// 本类型只证明闭合结构形状、冗余长度/计数和通用值编码；它不证明行排序键、跨表引用、
/// 摘要绑定、NetworkRevision、语义差异完备性或发布真实性。不得把它包装为
/// validated/trusted artifact view。
#[derive(Clone, Copy, Debug)]
pub struct RegistryCheckedObjectView<'a> {
    framing: ObjectFramingView<'a>,
}

impl<'a> RegistryCheckedObjectView<'a> {
    /// 已与 magic、目录和静态 registry 一致的对象种类。
    #[must_use]
    pub const fn kind(self) -> PortableObjectKind {
        self.framing.kind()
    }

    /// 整个对象的 exact bytes；其中每个 section/table/field 都已通过 registry 结构预检。
    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.framing.bytes()
    }

    /// 附录 A 冻结的精确 section 数量。
    #[must_use]
    pub const fn section_count(self) -> u32 {
        self.framing.section_count()
    }

    /// 取得一个已经完成节内 registry 结构预检的 section。
    #[must_use]
    pub fn section(self, ordinal: u32) -> Option<RegistryCheckedSectionView<'a>> {
        self.framing
            .section(ordinal)
            .map(|framing| RegistryCheckedSectionView { framing })
    }
}

/// 已完成该节全部 table/row/field registry 结构预检的借用。
#[derive(Clone, Copy, Debug)]
pub struct RegistryCheckedSectionView<'a> {
    framing: SectionFramingView<'a>,
}

impl<'a> RegistryCheckedSectionView<'a> {
    #[must_use]
    pub const fn kind(self) -> u16 {
        self.framing.kind()
    }

    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.framing.bytes()
    }
}

/// 对完整 v1 对象执行前导、目录与附录 A 静态 registry 的 fail-closed 结构预检。
pub fn preflight_object_registry_v1(
    bytes: &[u8],
    expected_kind: PortableObjectKind,
    limits: FormatLimits,
) -> Result<RegistryCheckedObjectView<'_>, FormatError> {
    let framing = preflight_object_framing(bytes, expected_kind, limits)?;
    let schema = portable_object_schema(expected_kind);
    let mut budget = PreflightBudget::default();

    for (ordinal, section_schema) in schema.sections.iter().enumerate() {
        let ordinal = u32::try_from(ordinal).map_err(|_| FormatError::ArithmeticOverflow {
            structure: FormatStructure::SectionDirectory,
        })?;
        let section = framing
            .section(ordinal)
            .ok_or(FormatError::BindingMismatch {
                structure: FormatStructure::Section,
            })?;
        let section_bytes = section.bytes();
        let table_count = read_u32(section_bytes, 0, FormatStructure::Section)?;
        if table_count as usize != section_schema.tables.len() {
            return Err(FormatError::LengthMismatch {
                structure: FormatStructure::Section,
                declared: u64::from(table_count),
                actual: section_schema.tables.len() as u64,
            });
        }

        let mut cursor = SECTION_HEADER_BYTES;
        for table_schema in section_schema.tables {
            let actual_table_kind = read_u16(section_bytes, cursor, FormatStructure::Table)?;
            if actual_table_kind == 0
                || usize::from(actual_table_kind) > section_schema.tables.len()
            {
                return Err(FormatError::UnknownKind {
                    structure: FormatStructure::Table,
                    code: u64::from(actual_table_kind),
                });
            }
            if actual_table_kind != table_schema.kind {
                return Err(FormatError::NonCanonicalOrder {
                    structure: FormatStructure::Section,
                    previous: u64::from(table_schema.kind),
                    current: u64::from(actual_table_kind),
                });
            }
            let rows_byte_length = read_u64(
                section_bytes,
                cursor
                    .checked_add(8)
                    .ok_or(FormatError::ArithmeticOverflow {
                        structure: FormatStructure::Table,
                    })?,
                FormatStructure::Table,
            )?;
            let table_byte_length = TABLE_HEADER_BYTES.checked_add(rows_byte_length).ok_or(
                FormatError::ArithmeticOverflow {
                    structure: FormatStructure::Table,
                },
            )?;
            let table_bytes = checked_slice(
                section_bytes,
                cursor,
                table_byte_length,
                FormatStructure::Table,
            )?;
            preflight_table_with_registry_v1(table_bytes, table_schema, limits, &mut budget)?;
            cursor =
                cursor
                    .checked_add(table_byte_length)
                    .ok_or(FormatError::ArithmeticOverflow {
                        structure: FormatStructure::Section,
                    })?;
        }
        if cursor != section_bytes.len() as u64 {
            return Err(FormatError::LengthMismatch {
                structure: FormatStructure::Section,
                declared: cursor,
                actual: section_bytes.len() as u64,
            });
        }
    }

    Ok(RegistryCheckedObjectView { framing })
}

#[cfg(test)]
mod tests {
    use std::vec;
    use std::vec::Vec;

    use laneflow_static_contract::{
        OBJECT_PREAMBLE_V1_BYTE_LENGTH, PortableFieldSchema, PortableFieldType,
        PortableRowCardinality, PortableRowSchema, SECTION_DIRECTORY_ENTRY_V1_BYTE_LENGTH,
        SECTION_FORMAT_VERSION_V1,
    };

    use super::*;
    use crate::{
        FormatErrorClass, FormatLimitConfig, LimitDimension,
        table::preflight_table_with_registry_v1,
    };

    fn default_value(field: &PortableFieldSchema) -> Vec<u8> {
        match field.field_type {
            PortableFieldType::U8 => vec![0],
            PortableFieldType::U16 => vec![0; 2],
            PortableFieldType::U32 | PortableFieldType::F32 | PortableFieldType::I32 => vec![0; 4],
            PortableFieldType::U64 | PortableFieldType::F64 => vec![0; 8],
            PortableFieldType::StableId128 => vec![0; 16],
            PortableFieldType::Sha256 => vec![0; 32],
            PortableFieldType::Utf8 | PortableFieldType::Bytes => Vec::new(),
            PortableFieldType::OrdinalVectorU32 | PortableFieldType::RecordVector => {
                0_u32.to_le_bytes().to_vec()
            }
        }
    }

    fn encoded_field_with_value(
        field: &PortableFieldSchema,
        field_type: PortableFieldType,
        value: &[u8],
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&field.tag.to_le_bytes());
        bytes.push(field_type as u8);
        bytes.push(0);
        bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
        bytes.extend_from_slice(value);
        bytes
    }

    fn encoded_field(field: &PortableFieldSchema, field_type: PortableFieldType) -> Vec<u8> {
        encoded_field_with_value(field, field_type, &default_value(field))
    }

    fn encoded_row(schema: &PortableRowSchema, omit_tag: Option<u16>) -> Vec<u8> {
        let fields = schema
            .fields
            .iter()
            .filter(|field| {
                Some(field.tag) != omit_tag
                    && field.presence == laneflow_static_contract::PortableFieldPresence::Required
            })
            .map(|field| encoded_field(field, field.field_type))
            .collect::<Vec<_>>();
        let length = 16 + fields.iter().map(Vec::len).sum::<usize>();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(length as u64).to_le_bytes());
        bytes.extend_from_slice(&(fields.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        for field in fields {
            bytes.extend_from_slice(&field);
        }
        bytes
    }

    fn encoded_variant_row(
        schema: &PortableRowSchema,
        tags: &[u16],
        discriminant_tag: u16,
        discriminant: u8,
    ) -> Vec<u8> {
        let fields = tags
            .iter()
            .map(|tag| {
                let field = schema
                    .fields
                    .iter()
                    .find(|field| field.tag == *tag)
                    .unwrap();
                if *tag == discriminant_tag {
                    encoded_field_with_value(field, field.field_type, &[discriminant])
                } else {
                    encoded_field(field, field.field_type)
                }
            })
            .collect::<Vec<_>>();
        let length = 16 + fields.iter().map(Vec::len).sum::<usize>();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(length as u64).to_le_bytes());
        bytes.extend_from_slice(&(fields.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        for field in fields {
            bytes.extend_from_slice(&field);
        }
        bytes
    }

    fn table_with_rows(
        schema: &laneflow_static_contract::PortableTableSchema,
        rows: &[Vec<u8>],
    ) -> Vec<u8> {
        let rows_length = rows.iter().map(Vec::len).sum::<usize>();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&schema.kind.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(rows_length as u64).to_le_bytes());
        for row in rows {
            bytes.extend_from_slice(row);
        }
        bytes
    }

    fn encoded_table(schema: &laneflow_static_contract::PortableTableSchema) -> Vec<u8> {
        let rows = if schema.cardinality == PortableRowCardinality::ExactlyOne {
            vec![encoded_row(schema.row, None)]
        } else {
            Vec::new()
        };
        table_with_rows(schema, &rows)
    }

    fn encoded_object(kind: PortableObjectKind) -> Vec<u8> {
        let schema = portable_object_schema(kind);
        let sections = schema
            .sections
            .iter()
            .map(|section| {
                let mut bytes = Vec::new();
                bytes.extend_from_slice(&(section.tables.len() as u32).to_le_bytes());
                for table in section.tables {
                    bytes.extend_from_slice(&encoded_table(table));
                }
                bytes
            })
            .collect::<Vec<_>>();
        let total = sections
            .iter()
            .map(Vec::len)
            .try_fold(kind.first_section_offset(), |total, length| {
                total.checked_add(length as u64)
            })
            .unwrap();
        let mut bytes = vec![0_u8; kind.first_section_offset() as usize];
        bytes[0..4].copy_from_slice(&kind.magic());
        bytes[4..6].copy_from_slice(&kind.format_version().to_le_bytes());
        bytes[6..8].copy_from_slice(&OBJECT_PREAMBLE_V1_BYTE_LENGTH.to_le_bytes());
        bytes[12..16].copy_from_slice(&kind.section_count().to_le_bytes());
        bytes[16..24].copy_from_slice(&u64::from(OBJECT_PREAMBLE_V1_BYTE_LENGTH).to_le_bytes());
        bytes[24..32].copy_from_slice(&total.to_le_bytes());
        let mut section_offset = kind.first_section_offset();
        for (ordinal, section) in sections.iter().enumerate() {
            let entry = usize::from(OBJECT_PREAMBLE_V1_BYTE_LENGTH)
                + ordinal * SECTION_DIRECTORY_ENTRY_V1_BYTE_LENGTH as usize;
            bytes[entry..entry + 2]
                .copy_from_slice(&u16::try_from(ordinal + 1).unwrap().to_le_bytes());
            bytes[entry + 2..entry + 4].copy_from_slice(&SECTION_FORMAT_VERSION_V1.to_le_bytes());
            bytes[entry + 8..entry + 16].copy_from_slice(&section_offset.to_le_bytes());
            bytes[entry + 16..entry + 24].copy_from_slice(&(section.len() as u64).to_le_bytes());
            section_offset += section.len() as u64;
            bytes.extend_from_slice(section);
        }
        bytes
    }

    #[test]
    fn every_object_registry_accepts_a_minimal_structural_traversal_fixture() {
        // 这些 bytes 由 registry 生成，只证明全部登记分支可遍历；它不是独立 known vector。
        for kind in PortableObjectKind::ALL {
            let bytes = encoded_object(kind);
            let checked =
                preflight_object_registry_v1(&bytes, kind, FormatLimits::V1_HARD).unwrap();
            assert_eq!(checked.kind(), kind);
            assert_eq!(checked.bytes(), bytes);
            assert_eq!(checked.section_count(), kind.section_count());
            assert_eq!(checked.section(0).unwrap().kind(), 1);
        }
    }

    #[test]
    fn object_preflight_rejects_section_table_count_and_trailing_bytes() {
        let kind = PortableObjectKind::CanonicalPublicationDescriptor;
        let original = encoded_object(kind);
        let first_section = kind.first_section_offset() as usize;

        let mut wrong_count = original.clone();
        wrong_count[first_section..first_section + 4].copy_from_slice(&0_u32.to_le_bytes());
        assert_eq!(
            preflight_object_registry_v1(&wrong_count, kind, FormatLimits::V1_HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::LengthMismatch
        );

        let mut wrong_table_kind = original;
        wrong_table_kind[first_section + 4..first_section + 6]
            .copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            preflight_object_registry_v1(&wrong_table_kind, kind, FormatLimits::V1_HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::UnknownKind
        );

        let mut trailing = encoded_object(kind);
        trailing.push(0);
        let new_object_length = trailing.len() as u64;
        trailing[24..32].copy_from_slice(&new_object_length.to_le_bytes());
        let last_entry = usize::from(OBJECT_PREAMBLE_V1_BYTE_LENGTH)
            + 3 * SECTION_DIRECTORY_ENTRY_V1_BYTE_LENGTH as usize;
        let old_length = u64::from_le_bytes(
            trailing[last_entry + 16..last_entry + 24]
                .try_into()
                .unwrap(),
        );
        trailing[last_entry + 16..last_entry + 24].copy_from_slice(&(old_length + 1).to_le_bytes());
        assert_eq!(
            preflight_object_registry_v1(&trailing, kind, FormatLimits::V1_HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::LengthMismatch
        );

        let lfca_kind = PortableObjectKind::CanonicalArtifact;
        let mut wrong_order = encoded_object(lfca_kind);
        let entity_section_entry = usize::from(OBJECT_PREAMBLE_V1_BYTE_LENGTH)
            + 2 * SECTION_DIRECTORY_ENTRY_V1_BYTE_LENGTH as usize;
        let entity_section_offset = u64::from_le_bytes(
            wrong_order[entity_section_entry + 8..entity_section_entry + 16]
                .try_into()
                .unwrap(),
        ) as usize;
        wrong_order[entity_section_offset + 4..entity_section_offset + 6]
            .copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            preflight_object_registry_v1(&wrong_order, lfca_kind, FormatLimits::V1_HARD)
                .unwrap_err()
                .class(),
            FormatErrorClass::NonCanonicalOrder
        );
    }

    #[test]
    fn registry_table_preflight_rejects_missing_unknown_and_wrong_type_fields() {
        let table_schema =
            &portable_object_schema(PortableObjectKind::CanonicalPublicationDescriptor).sections[0]
                .tables[0];
        let valid_row = encoded_row(table_schema.row, None);

        let empty_singleton = table_with_rows(table_schema, &[]);
        assert_eq!(
            preflight_table_with_registry_v1(
                &empty_singleton,
                table_schema,
                FormatLimits::V1_HARD,
                &mut PreflightBudget::default(),
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );

        let missing = table_with_rows(table_schema, &[encoded_row(table_schema.row, Some(3))]);
        assert_eq!(
            preflight_table_with_registry_v1(
                &missing,
                table_schema,
                FormatLimits::V1_HARD,
                &mut PreflightBudget::default(),
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );

        let mut wrong_type_row = valid_row;
        wrong_type_row[18] = PortableFieldType::U32 as u8;
        let wrong_type = table_with_rows(table_schema, &[wrong_type_row]);
        assert_eq!(
            preflight_table_with_registry_v1(
                &wrong_type,
                table_schema,
                FormatLimits::V1_HARD,
                &mut PreflightBudget::default(),
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );

        let mut unknown_row = encoded_row(table_schema.row, None);
        let unknown_field = {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&6_u16.to_le_bytes());
            bytes.push(PortableFieldType::U8 as u8);
            bytes.push(0);
            bytes.extend_from_slice(&1_u64.to_le_bytes());
            bytes.push(0);
            bytes
        };
        let new_length = unknown_row.len() + unknown_field.len();
        unknown_row[0..8].copy_from_slice(&(new_length as u64).to_le_bytes());
        unknown_row[8..12].copy_from_slice(&6_u32.to_le_bytes());
        unknown_row.extend_from_slice(&unknown_field);
        let unknown = table_with_rows(table_schema, &[unknown_row]);
        assert_eq!(
            preflight_table_with_registry_v1(
                &unknown,
                table_schema,
                FormatLimits::V1_HARD,
                &mut PreflightBudget::default(),
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::UnknownKind
        );
    }

    #[test]
    fn discriminated_rows_enforce_lfsm_and_lfsd_presence_matrices() {
        let source_location =
            &portable_object_schema(PortableObjectKind::SourceMap).sections[1].tables[2];
        let valid_text = encoded_variant_row(source_location.row, &[1, 2, 3, 4, 5, 6, 7, 8], 2, 0);
        let table = table_with_rows(source_location, &[valid_text]);
        preflight_table_with_registry_v1(
            &table,
            source_location,
            FormatLimits::V1_HARD,
            &mut PreflightBudget::default(),
        )
        .unwrap();

        let text_with_road_field =
            encoded_variant_row(source_location.row, &[1, 2, 3, 4, 5, 6, 7, 8, 9], 2, 0);
        let table = table_with_rows(source_location, &[text_with_road_field]);
        assert_eq!(
            preflight_table_with_registry_v1(
                &table,
                source_location,
                FormatLimits::V1_HARD,
                &mut PreflightBudget::default(),
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );

        let static_rule =
            &portable_object_schema(PortableObjectKind::SemanticDiff).sections[4].tables[0];
        let valid_modify = encoded_variant_row(static_rule.row, &[1, 2, 4, 6, 9], 1, 0);
        let table = table_with_rows(static_rule, &[valid_modify]);
        preflight_table_with_registry_v1(
            &table,
            static_rule,
            FormatLimits::V1_HARD,
            &mut PreflightBudget::default(),
        )
        .unwrap();

        let no_payload = encoded_variant_row(static_rule.row, &[1, 2, 4, 6], 1, 0);
        let table = table_with_rows(static_rule, &[no_payload]);
        assert_eq!(
            preflight_table_with_registry_v1(
                &table,
                static_rule,
                FormatLimits::V1_HARD,
                &mut PreflightBudget::default(),
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );

        let wrong_code = encoded_variant_row(static_rule.row, &[1, 2, 4, 6, 9], 1, 2);
        let table = table_with_rows(static_rule, &[wrong_code]);
        assert_eq!(
            preflight_table_with_registry_v1(
                &table,
                static_rule,
                FormatLimits::V1_HARD,
                &mut PreflightBudget::default(),
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::UnknownKind
        );

        let spatial_configuration =
            &portable_object_schema(PortableObjectKind::SemanticDiff).sections[5].tables[0];
        let initialize = encoded_variant_row(spatial_configuration.row, &[1, 3], 1, 0);
        let table = table_with_rows(spatial_configuration, &[initialize.clone(), initialize]);
        assert_eq!(
            preflight_table_with_registry_v1(
                &table,
                spatial_configuration,
                FormatLimits::V1_HARD,
                &mut PreflightBudget::default(),
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );
    }

    #[test]
    fn record_vector_rows_use_the_field_specific_nested_registry() {
        let identity =
            &portable_object_schema(PortableObjectKind::CanonicalArtifact).sections[1].tables[0];
        let nested_schema = identity.row.fields[3].nested_row.unwrap();
        let incomplete_nested = encoded_row(nested_schema, Some(2));
        let mut record_value = Vec::new();
        record_value.extend_from_slice(&1_u32.to_le_bytes());
        record_value.extend_from_slice(&incomplete_nested);

        let mut top_fields = identity.row.fields[..3]
            .iter()
            .map(|field| encoded_field(field, field.field_type))
            .collect::<Vec<_>>();
        top_fields.push(encoded_field_with_value(
            &identity.row.fields[3],
            PortableFieldType::RecordVector,
            &record_value,
        ));
        let row_length = 16 + top_fields.iter().map(Vec::len).sum::<usize>();
        let mut row = Vec::new();
        row.extend_from_slice(&(row_length as u64).to_le_bytes());
        row.extend_from_slice(&(top_fields.len() as u32).to_le_bytes());
        row.extend_from_slice(&0_u32.to_le_bytes());
        for field in top_fields {
            row.extend_from_slice(&field);
        }
        let table = table_with_rows(identity, &[row]);
        assert_eq!(
            preflight_table_with_registry_v1(
                &table,
                identity,
                FormatLimits::V1_HARD,
                &mut PreflightBudget::default(),
            )
            .unwrap_err()
            .class(),
            FormatErrorClass::BindingMismatch
        );
    }

    #[test]
    fn registry_tables_share_the_object_wide_utf8_budget() {
        let schema = portable_object_schema(PortableObjectKind::CanonicalPublicationDescriptor);
        let first = &schema.sections[1].tables[0];
        let second = &schema.sections[3].tables[0];
        let row_with_utf8 = |row_schema: &PortableRowSchema| {
            let fields = row_schema
                .fields
                .iter()
                .filter(|field| {
                    field.presence == laneflow_static_contract::PortableFieldPresence::Required
                })
                .map(|field| {
                    if field.field_type == PortableFieldType::Utf8 {
                        encoded_field_with_value(field, field.field_type, b"x")
                    } else {
                        encoded_field(field, field.field_type)
                    }
                })
                .collect::<Vec<_>>();
            let length = 16 + fields.iter().map(Vec::len).sum::<usize>();
            let mut row = Vec::new();
            row.extend_from_slice(&(length as u64).to_le_bytes());
            row.extend_from_slice(&(fields.len() as u32).to_le_bytes());
            row.extend_from_slice(&0_u32.to_le_bytes());
            for field in fields {
                row.extend_from_slice(&field);
            }
            row
        };
        let first_table = table_with_rows(first, &[row_with_utf8(first.row)]);
        let second_table = table_with_rows(second, &[row_with_utf8(second.row)]);
        let mut config = FormatLimitConfig::V1_HARD;
        // SourceMapBinding 有一个 UTF-8，PublicationProvenance 有四个；第二张表必须在
        // 读取第一个 value 时越过对象累计预算。
        config.max_total_utf8_bytes = 1;
        let limits = FormatLimits::try_new(config).unwrap();
        let mut budget = PreflightBudget::default();
        preflight_table_with_registry_v1(&first_table, first, limits, &mut budget).unwrap();
        assert_eq!(
            preflight_table_with_registry_v1(&second_table, second, limits, &mut budget)
                .unwrap_err(),
            FormatError::LimitExceeded {
                dimension: LimitDimension::TotalUtf8Bytes,
                actual: 2,
                limit: 1,
            }
        );
    }
}
