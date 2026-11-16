//! 受检字段值的确定性展示：统一截断协议与值格式化。
//!
//! 截断协议（不变量）：一个值要么完整展示，要么带明确截断标记，要么声明
//! 「限额内省略」——绝不静默丢内容。UTF-8 的预算按转义后的输出域计，以
//! 一个转义单元为原子追加；预算耗尽的嵌套结构（LFSD 内嵌成员）产出
//! 省略状态，与真空值结构性可区分。
//!
//! 展示只影响可读性：截断时给出总长度与 SHA-256 摘要，不改变比较结论。

use laneflow_format::{
    FormatError, FormatLimits, RegistryCheckedFieldValue, RegistryCheckedFieldView,
    RegistryCheckedOrdinalVectorView, RegistryCheckedRecordVectorView, checked_policy_member_row,
};
use laneflow_static_contract::Sha256Digest;
use sha2::{Digest, Sha256};

use crate::session::ReportLimits;
use crate::walk::{FieldPath, NestedStep};

/// 一个字段值的确定性展示快照。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValueDisplay {
    /// `U8` 值。
    U8(u8),
    /// `U16` 值。
    U16(u16),
    /// `U32` 值。
    U32(u32),
    /// `U64` 值。
    U64(u64),
    /// `F32` 值：十进制与位模式。
    F32 {
        /// 最短往返十进制。
        decimal: String,
        /// IEEE 位模式。
        bits: u32,
    },
    /// `F64` 值：十进制与位模式。
    F64 {
        /// 最短往返十进制。
        decimal: String,
        /// IEEE 位模式。
        bits: u64,
    },
    /// `I32` 值。
    I32(i32),
    /// 128 位稳定标识的十六进制。
    StableId128(String),
    /// SHA-256 摘要的十六进制。
    Sha256(String),
    /// 已转义的 UTF-8 字符串；超限截断时带总长度。
    Utf8 {
        /// 转义并截断后的文本。
        text: String,
        /// 是否因限额截断（有输入字符未消费）。
        truncated: bool,
        /// 原始字节总长。
        total: usize,
    },
    /// 不透明字节串的十六进制预览；截断时带总长度与摘要。
    Bytes {
        /// 预览十六进制。
        hex: String,
        /// 原始字节总长。
        total: usize,
        /// 截断时的 SHA-256 摘要（未截断为空串）。
        digest: String,
    },
    /// `u32` 序号向量的有界项列表。
    OrdinalVector {
        /// 展示的项。
        items: Vec<u32>,
        /// 向量总项数。
        total: u32,
    },
    /// 一层 `RecordVector`：总项数与受行数限额约束的逐项字段展示。
    RecordVector {
        /// 受限额约束的逐项字段展示。
        rows: Vec<Vec<(u16, ValueDisplay)>>,
        /// 向量总项数。
        total: u32,
    },
    /// LFSD 已登记 Bytes 内嵌行：`fields` 为 `None` 表示行数限额耗尽、内容
    /// 整体省略（区别于空行）。
    EmbeddedRow {
        /// 成员种类（LFSD 变更行 tag 3）。
        kind: u8,
        /// 内嵌行字段展示；`None` = 限额内省略。
        fields: Option<Vec<(u16, ValueDisplay)>>,
    },
}

/// 展开行预算：跨值构造传递的行数限额句柄。
///
/// 预算耗尽可能性由构造侧如实反映（省略状态 / 截断标记），调用方不再
/// 各自猜测预算状态。
pub struct RowBudget<'a> {
    remaining: &'a mut usize,
}

impl<'a> RowBudget<'a> {
    /// 绑定剩余行数。
    pub fn new(remaining: &'a mut usize) -> Self {
        Self { remaining }
    }

    /// 尝试消费一行预算；耗尽时返回 `false`。
    pub fn try_consume(&mut self) -> bool {
        if *self.remaining == 0 {
            return false;
        }
        *self.remaining -= 1;
        true
    }
}

/// 按登记类型解码并生成字段值的展示快照。
///
/// # Errors
///
/// 字段值按登记类型解码失败时返回 [`FormatError`]（正常输入不触发；
/// 该分支是预检已证实路径的防御性回退）。
pub fn display_field_value(
    field: RegistryCheckedFieldView<'_>,
    path: &FieldPath,
    limits: &ReportLimits,
    rows: &mut RowBudget<'_>,
) -> Result<ValueDisplay, FormatError> {
    Ok(match field.value()? {
        RegistryCheckedFieldValue::U8(v) => ValueDisplay::U8(v),
        RegistryCheckedFieldValue::U16(v) => ValueDisplay::U16(v),
        RegistryCheckedFieldValue::U32(v) => ValueDisplay::U32(v),
        RegistryCheckedFieldValue::U64(v) => ValueDisplay::U64(v),
        RegistryCheckedFieldValue::I32(v) => ValueDisplay::I32(v),
        RegistryCheckedFieldValue::F32(v) => ValueDisplay::F32 {
            decimal: format!("{v}"),
            bits: v.to_bits(),
        },
        RegistryCheckedFieldValue::F64(v) => ValueDisplay::F64 {
            decimal: format!("{v}"),
            bits: v.to_bits(),
        },
        RegistryCheckedFieldValue::StableId128(v) => {
            ValueDisplay::StableId128(hex_string(v.as_bytes(), 32))
        }
        RegistryCheckedFieldValue::Sha256(v) => ValueDisplay::Sha256(hex_string(v.as_bytes(), 64)),
        RegistryCheckedFieldValue::Utf8(text) => {
            escape_bounded(text, limits.max_value_display_bytes)
        }
        RegistryCheckedFieldValue::Bytes(bytes) => bytes_preview(bytes, limits),
        RegistryCheckedFieldValue::OrdinalVectorU32(vector) => {
            ordinal_vector_preview(vector, limits)
        }
        RegistryCheckedFieldValue::RecordVector(vector) => {
            record_vector_preview(vector, path, limits, rows)
        }
    })
}

/// UTF-8 截断协议：以一个转义单元为原子追加，放不下即停并标记截断。
///
/// `truncated` 的判定是「有输入字符未被消费」——预算按转义后的输出域计，
/// 转义扩张导致的丢字符同样会被标记。
fn escape_bounded(text: &str, budget: usize) -> ValueDisplay {
    let total = text.len();
    let mut shown = String::new();
    let mut truncated = false;
    for c in text.chars() {
        let escaped = escape_char(c);
        if shown.len() + escaped.len() > budget {
            truncated = true;
            break;
        }
        shown.push_str(&escaped);
    }
    ValueDisplay::Utf8 {
        text: shown,
        truncated,
        total,
    }
}

/// 不透明 Bytes 的有界预览：截断时给出总长度与 SHA-256 摘要。
#[must_use]
pub fn bytes_preview(bytes: &[u8], limits: &ReportLimits) -> ValueDisplay {
    let total = bytes.len();
    let truncated = total > limits.max_value_display_bytes;
    let shown = &bytes[..total.min(limits.max_value_display_bytes)];
    let digest = if truncated {
        digest_hex(&Sha256Digest::from_bytes(Sha256::digest(bytes).into()))
    } else {
        String::new()
    };
    ValueDisplay::Bytes {
        hex: hex_string(shown, shown.len().saturating_mul(2)),
        total,
        digest,
    }
}

fn ordinal_vector_preview(
    vector: RegistryCheckedOrdinalVectorView<'_>,
    limits: &ReportLimits,
) -> ValueDisplay {
    let total = vector.len();
    let items = (0..total)
        .take(limits.max_vector_items)
        .filter_map(|index| vector.get(index))
        .collect();
    ValueDisplay::OrdinalVector { items, total }
}

fn record_vector_preview(
    vector: RegistryCheckedRecordVectorView<'_>,
    path: &FieldPath,
    limits: &ReportLimits,
    rows: &mut RowBudget<'_>,
) -> ValueDisplay {
    let total = vector.len();
    let take = usize::try_from(total)
        .unwrap_or(usize::MAX)
        .min(limits.max_vector_items);
    let mut shown_rows = Vec::with_capacity(take);
    for (index, row) in vector.rows().enumerate().take(take) {
        if !rows.try_consume() {
            break;
        }
        let nested = path.with_step(NestedStep::RecordVectorItem {
            index: u32::try_from(index).unwrap_or(u32::MAX),
            field_tag: path.field_tag,
            field_name: path.field_name,
        });
        let mut fields = Vec::with_capacity(usize::try_from(row.field_count()).unwrap_or(0));
        for field in row.fields() {
            if let Ok(value) = display_field_value(field, &nested, limits, rows) {
                fields.push((field.tag(), value));
            }
        }
        shown_rows.push(fields);
    }
    ValueDisplay::RecordVector {
        rows: shown_rows,
        total,
    }
}

/// LFSD 已登记成员 Bytes 的有界内嵌行展示。
///
/// 复用 `checked_policy_member_row` 的同一预检路径；预检失败的成员值返回
/// [`FormatError`]，调用方按检查失败语义处理。行预算耗尽时返回省略状态
/// （`fields = None`），不伪装成空行。
///
/// # Errors
///
/// 成员 kind 未登记或 bytes 未通过策略局部 RowV1 预检时返回 [`FormatError`]。
pub fn embedded_row_display(
    kind: u8,
    bytes: &[u8],
    outer_path: &FieldPath,
    limits: &ReportLimits,
    rows: &mut RowBudget<'_>,
    format_limits: FormatLimits,
) -> Result<ValueDisplay, FormatError> {
    let row = checked_policy_member_row(kind, bytes, format_limits)?;
    let member_path = outer_path.with_step(NestedStep::PolicyMember {
        kind,
        field_tag: outer_path.field_tag,
        field_name: outer_path.field_name,
    });
    if !rows.try_consume() {
        return Ok(ValueDisplay::EmbeddedRow { kind, fields: None });
    }
    let mut fields = Vec::with_capacity(usize::try_from(row.field_count()).unwrap_or(0));
    for field in row.fields() {
        let mut leaf = member_path.clone();
        leaf.field_tag = field.tag();
        leaf.field_name = crate::walk::field_name(row_schema_of(kind), field.tag()).unwrap_or("");
        let value = display_field_value(field, &leaf, limits, rows)?;
        fields.push((field.tag(), value));
    }
    Ok(ValueDisplay::EmbeddedRow {
        kind,
        fields: Some(fields),
    })
}

fn row_schema_of(kind: u8) -> &'static laneflow_static_contract::PortableRowSchema {
    laneflow_static_contract::policy_local_value_schema(kind).unwrap_or(&EMPTY_MEMBER_SCHEMA)
}

static EMPTY_MEMBER_SCHEMA: laneflow_static_contract::PortableRowSchema =
    laneflow_static_contract::PortableRowSchema {
        fields: &[],
        shape: laneflow_static_contract::PortableRowShape::Uniform,
    };

/// 十六进制串；超过 `max_chars` 时截断并以省略号标记。
#[must_use]
pub fn hex_string(bytes: &[u8], max_chars: usize) -> String {
    let mut out = String::new();
    for byte in bytes {
        if out.len() + 2 > max_chars {
            out.push('…');
            break;
        }
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn escape_char(c: char) -> String {
    match c {
        '\n' => "\\n".to_string(),
        '\r' => "\\r".to_string(),
        '\t' => "\\t".to_string(),
        '\\' => "\\\\".to_string(),
        '"' => "\\\"".to_string(),
        c if (u32::from(c) < 0x20) || (u32::from(c) == 0x7f) => format!("\\x{:02x}", u32::from(c)),
        c => c.to_string(),
    }
}

/// SHA-256 摘要的十六进制小写展示。
#[must_use]
pub fn digest_hex(digest: &Sha256Digest) -> String {
    hex_string(digest.as_bytes(), 64)
}
