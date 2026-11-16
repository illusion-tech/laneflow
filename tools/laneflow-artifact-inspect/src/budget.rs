//! 工具资源限制：输入外层限额、格式调用方预算的只能收紧、报告独立设限。

use laneflow_format::{FormatLimitConfig, FormatLimits};

/// 工具对格式调用方预算（`max_object_bytes` / `max_chunks_per_section`）的
/// 显式收紧规则。
///
/// `FormatLimits::try_new` 本身不保证这两个调用方预算维度不超过 HARD 默认值；
/// 本类型负责拒绝任何高于 HARD 默认值的请求，使工具侧只允许收紧。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolFormatBudget {
    /// 单个对象 exact bytes 的调用方预算上限。
    pub max_object_bytes: u64,
    /// 每个 section 允许的最大物理 chunk 数。
    pub max_chunks_per_section: u32,
}

impl ToolFormatBudget {
    /// 工具默认预算，等于 `FormatLimitConfig::HARD` 的两个调用方维度。
    pub const HARD_DEFAULT: Self = Self {
        max_object_bytes: FormatLimitConfig::HARD.max_object_bytes,
        max_chunks_per_section: FormatLimitConfig::HARD.max_chunks_per_section,
    };

    /// 按只能收紧的规则构造 [`FormatLimits`]。
    ///
    /// # Errors
    ///
    /// 任一请求值高于 [`Self::HARD_DEFAULT`] 对应维度时返回
    /// [`BudgetTightenError`]；不做静默 clamp。
    pub fn tighten(
        max_object_bytes: Option<u64>,
        max_chunks_per_section: Option<u32>,
    ) -> Result<FormatLimits, BudgetTightenError> {
        let base = Self::HARD_DEFAULT;
        let max_object_bytes = max_object_bytes.unwrap_or(base.max_object_bytes);
        let max_chunks_per_section = max_chunks_per_section.unwrap_or(base.max_chunks_per_section);
        if max_object_bytes > base.max_object_bytes {
            return Err(BudgetTightenError::ObjectBytesNotTightened {
                requested: max_object_bytes,
                hard_default: base.max_object_bytes,
            });
        }
        if max_chunks_per_section > base.max_chunks_per_section {
            return Err(BudgetTightenError::ChunksNotTightened {
                requested: u64::from(max_chunks_per_section),
                hard_default: u64::from(base.max_chunks_per_section),
            });
        }
        if max_object_bytes == 0 || max_chunks_per_section == 0 {
            return Err(BudgetTightenError::ZeroBudget);
        }
        let mut config = FormatLimitConfig::HARD;
        config.max_object_bytes = max_object_bytes;
        config.max_chunks_per_section = max_chunks_per_section;
        FormatLimits::try_new(config).map_err(|_| BudgetTightenError::ZeroBudget)
    }
}

/// 只能收紧规则被违反时的错误。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetTightenError {
    /// 请求的对象预算高于 HARD 默认值。
    ObjectBytesNotTightened {
        /// 被拒绝的请求值。
        requested: u64,
        /// HARD 默认值。
        hard_default: u64,
    },
    /// 请求的 chunk 预算高于 HARD 默认值。
    ChunksNotTightened {
        /// 被拒绝的请求值。
        requested: u64,
        /// HARD 默认值。
        hard_default: u64,
    },
    /// 请求了零预算。
    ZeroBudget,
}

impl core::fmt::Display for BudgetTightenError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ObjectBytesNotTightened {
                requested,
                hard_default,
            } => write!(
                f,
                "max-object-bytes 只能收紧（请求 {requested} 高于 HARD 默认 {hard_default}）"
            ),
            Self::ChunksNotTightened {
                requested,
                hard_default,
            } => write!(
                f,
                "max-chunks-per-section 只能收紧（请求 {requested} 高于 HARD 默认 {hard_default}）"
            ),
            Self::ZeroBudget => write!(f, "预算必须为正"),
        }
    }
}

impl std::error::Error for BudgetTightenError {}

/// 输入外层读入与解码的限额。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputLimits {
    /// 允许从文件系统读入的最大字节数（raw 为对象字节，hex 为文本字节）。
    pub max_input_bytes: u64,
    /// 解码（hex）或读入（raw）后单个对象的最大字节数。
    pub max_object_bytes: u64,
}

impl InputLimits {
    /// 与 [`ToolFormatBudget::HARD_DEFAULT`] 对齐的默认输入限额。
    ///
    /// hex 文本上限按「每字节两字符加余量」推导，解码后仍受
    /// `max_object_bytes` 约束。
    pub const HARD_DEFAULT: Self = Self {
        max_input_bytes: FormatLimitConfig::HARD.max_object_bytes,
        max_object_bytes: FormatLimitConfig::HARD.max_object_bytes,
    };

    /// hex 外层的文本上限：对象上限的两倍加 4 KiB 余量（允许空白）。
    #[must_use]
    pub const fn hex_text_limit(self) -> u64 {
        self.max_object_bytes.saturating_mul(2).saturating_add(4096)
    }
}

/// 报告展示限额：只影响展示，不改变检查与比较结论。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReportLimits {
    /// 稳定报告正文的最大输出字节数。
    pub max_output_bytes: usize,
    /// 差异条目的最大展示条数。
    pub max_diff_entries: usize,
    /// inspect 展开的最大行数。
    pub max_expanded_rows: usize,
    /// 单值展示的最大字节长度（UTF-8 / Bytes 预览）。
    pub max_value_display_bytes: usize,
    /// 向量展示的最大元素个数。
    pub max_vector_items: usize,
}

impl ReportLimits {
    /// 工具默认展示限额。
    pub const DEFAULT: Self = Self {
        max_output_bytes: 65_536,
        max_diff_entries: 200,
        max_expanded_rows: 64,
        max_value_display_bytes: 128,
        max_vector_items: 16,
    };
}

impl Default for ReportLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}
