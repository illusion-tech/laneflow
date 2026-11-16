//! 会话配置与展示策略：资源限额的唯一构造点、筛选谓词的唯一实现。
//!
//! 分层契约：
//! - [`SessionConfig`] 是资源策略（读入期限界、格式调用方预算、报告限额），
//!   其中格式预算只能经 [`ToolFormatBudget::tighten`] 构造——类型即「只能收紧」
//!   的证明，不存在校验后被丢弃的通道。
//! - [`DisplayPolicy`] 是展示策略（筛选 + 报告限额）。渲染与收集侧的全部
//!   可展示类别都必须经过它的谓词；wire 事实（前导、全对象总数）不属于
//!   可筛范畴，代码上没有任何应用筛选的路径。
//! - 比较与检查结论永远不受本层影响：筛选只决定「哪些条目进入展示缓冲」。

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
    pub const HARD_DEFAULT: Self = Self {
        max_input_bytes: FormatLimitConfig::HARD.max_object_bytes,
        max_object_bytes: FormatLimitConfig::HARD.max_object_bytes,
    };

    /// hex 外层的文本上限：对象上限的两倍加 4 KiB 余量（允许空白）。
    #[must_use]
    pub const fn hex_text_limit(self) -> u64 {
        self.max_object_bytes.saturating_mul(2).saturating_add(4096)
    }

    /// 与已收紧的对象预算对齐（读入限额跟随对象预算）。
    #[must_use]
    pub const fn for_object_budget(max_object_bytes: u64) -> Self {
        Self {
            max_input_bytes: max_object_bytes,
            max_object_bytes,
        }
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
    /// 单值展示的最大字节长度（UTF-8 转义后 / Bytes 预览）。
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

/// 资源策略：一次构造、全程传递，杜绝逐层手工重传限额造成的旁路。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionConfig {
    /// 输入外层读入与解码限额。
    pub input: InputLimits,
    /// 格式调用方预算（只能经收紧构造）。
    pub format: FormatLimits,
}

impl SessionConfig {
    /// 默认会话：读入与格式预算都取 HARD 默认。
    #[must_use]
    pub fn hardened() -> Self {
        Self {
            input: InputLimits::HARD_DEFAULT,
            format: FormatLimits::HARD,
        }
    }

    /// 按只能收紧的规则构造会话；读入限额跟随收紧后的对象预算。
    ///
    /// # Errors
    ///
    /// 预算请求高于 HARD 默认或为零时返回 [`BudgetTightenError`]。
    pub fn tighten(
        max_object_bytes: Option<u64>,
        max_chunks_per_section: Option<u32>,
    ) -> Result<Self, BudgetTightenError> {
        let format = ToolFormatBudget::tighten(max_object_bytes, max_chunks_per_section)?;
        let budget = max_object_bytes.unwrap_or(ToolFormatBudget::HARD_DEFAULT.max_object_bytes);
        Ok(Self {
            input: InputLimits::for_object_budget(budget),
            format,
        })
    }
}

/// 行范围：零基全表逻辑行，左闭右开。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RowRange {
    /// 起始行（含）。
    pub start: u32,
    /// 结束行（不含）。
    pub end: u32,
}

/// 节选择器。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SectionSelector {
    /// 零基节序号。
    Ordinal(u32),
    /// 节登记名（区分大小写）。
    Name(String),
}

/// 表选择器。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TableSelector {
    /// 表登记 kind。
    Kind(u16),
    /// 表登记名（区分大小写）。
    Name(String),
}

/// 检视/差异共用的展示筛选。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InspectFilters {
    /// 按零基序号或登记名筛选节；`None` 为全部。
    pub section: Option<SectionSelector>,
    /// 按 kind 数值或登记名筛选表；`None` 为全部。
    pub table: Option<TableSelector>,
    /// 行范围（作用于每张表的零基逻辑行）。
    pub rows: Option<RowRange>,
}

impl InspectFilters {
    /// 是否存在任一有效筛选维度。
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.section.is_some() || self.table.is_some() || self.rows.is_some()
    }

    /// 节谓词：本层唯一的节筛选实现，渲染与收集侧共用。
    #[must_use]
    pub fn selects_section(&self, ordinal: u32, name: &str) -> bool {
        match &self.section {
            None => true,
            Some(SectionSelector::Ordinal(expected)) => *expected == ordinal,
            Some(SectionSelector::Name(expected)) => *expected == name,
        }
    }

    /// 表谓词：本层唯一的表筛选实现。
    #[must_use]
    pub fn selects_table(&self, kind: u16, name: &str) -> bool {
        match &self.table {
            None => true,
            Some(TableSelector::Kind(expected)) => *expected == kind,
            Some(TableSelector::Name(expected)) => *expected == name,
        }
    }

    /// 逻辑行谓词：行范围筛选。
    #[must_use]
    pub fn selects_row(&self, logical_row: u32) -> bool {
        match &self.rows {
            None => true,
            Some(range) => logical_row >= range.start && logical_row < range.end,
        }
    }
}

/// 展示策略：筛选与报告限额的唯一挂载点。
///
/// 渲染器的全部可展示类别（字段、容器、行数、表汇总）都必须经过本策略的
/// 谓词；收集侧的展示缓冲同样以本策略过滤。wire 事实（前导、全对象总数、
/// 逐表首差、精确计数）不属于可筛范畴。
#[derive(Default, Clone, Debug, Eq, PartialEq)]
pub struct DisplayPolicy {
    /// 筛选。
    pub filters: InspectFilters,
    /// 报告限额。
    pub limits: ReportLimits,
}

impl DisplayPolicy {
    /// 字段路径谓词：节 + 表 + 行三维修一。
    #[must_use]
    pub fn selects_field_path(
        &self,
        section_ordinal: u32,
        section_name: &str,
        table_kind: u16,
        table_name: &str,
        logical_row: u32,
    ) -> bool {
        self.filters.selects_section(section_ordinal, section_name)
            && self.filters.selects_table(table_kind, table_name)
            && self.filters.selects_row(logical_row)
    }
}
