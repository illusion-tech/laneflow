//! 一次成功编译原子拥有的输出与资源观测值。

use crate::source_map::ValidatedSourceMapInput;
use crate::{Diagnostic, ValidatedCanonicalLir};

/// 一次成功编译原子拥有的已验证结果。
///
/// LIR 与来源伴随数据不能分别构造或重新配对；后继源映射后端必须从同一个实例同时借用
/// 二者。当前支持子集不产生 warning/note，因此 `diagnostics` 为空，但该成功契约保留
/// 非错误级诊断通道。
pub struct CompilationOutput {
    pub(in crate::compiler) lir: ValidatedCanonicalLir,
    source_map_input: ValidatedSourceMapInput,
    diagnostics: Box<[Diagnostic]>,
    metrics: CompilationMetrics,
}

/// 一次成功生产编译的只读资源与确定性观测值。
///
/// 这些值来自编译器实际完成的 HIR→MIR→Canonical LIR 管线，不包含前端构造、当前态
/// 投影或证据序列化。字节数是编译器内部资源模型使用的逻辑值，不等同于操作系统进程
/// 工作集，也不是静态镜像或后继可移植制品的文件大小。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompilationMetrics {
    lir_record_count: u64,
    output_logical_bytes: u64,
    compiler_controlled_peak_bytes: u64,
    semantic_fingerprint: [u8; 32],
}

impl CompilationMetrics {
    pub(super) const fn from_pipeline(
        lir_record_count: u64,
        output_logical_bytes: u64,
        compiler_controlled_peak_bytes: u64,
        semantic_fingerprint: [u8; 32],
    ) -> Self {
        Self {
            lir_record_count,
            output_logical_bytes,
            compiler_controlled_peak_bytes,
            semantic_fingerprint,
        }
    }

    /// 返回 Canonical LIR 的实体、关系与出现项逻辑记录总数。
    #[must_use]
    pub const fn lir_record_count(self) -> u64 {
        self.lir_record_count
    }

    /// 返回目标布局中立的 Canonical LIR 逻辑输出字节数。
    #[must_use]
    pub const fn output_logical_bytes(self) -> u64 {
        self.output_logical_bytes
    }

    /// 返回本次编译资源模型计算的编译器控制峰值字节数。
    ///
    /// 该值覆盖同一阶段同时存续的来源、IR、暂存区和输出容量，但不包含标准库、系统
    /// 分配器元数据或进程内其他组件的内存。
    #[must_use]
    pub const fn compiler_controlled_peak_bytes(self) -> u64 {
        self.compiler_controlled_peak_bytes
    }

    /// 返回当前编译器版本对完整 Canonical LIR 语义计算的确定性指纹。
    ///
    /// 该指纹用于同版本重复编译和性能证据核对；它不是制品完整性摘要、路网修订 ID
    /// 或跨格式版本兼容承诺，调用方不得用它替代后继版本化制品描述符。
    #[must_use]
    pub const fn semantic_fingerprint(self) -> [u8; 32] {
        self.semantic_fingerprint
    }
}

impl CompilationOutput {
    pub(super) fn from_success(
        lir: ValidatedCanonicalLir,
        source_map_input: ValidatedSourceMapInput,
        diagnostics: Box<[Diagnostic]>,
        metrics: CompilationMetrics,
    ) -> Self {
        Self {
            lir,
            source_map_input,
            diagnostics,
            metrics,
        }
    }

    /// 借用所有静态语义后端的唯一输入。
    #[must_use]
    pub const fn lir(&self) -> &ValidatedCanonicalLir {
        &self.lir
    }

    /// 借用仅供源映射/诊断后端使用的来源伴随数据。
    #[must_use]
    pub const fn source_map_input(&self) -> &ValidatedSourceMapInput {
        &self.source_map_input
    }

    /// 返回成功路径保留的非错误级诊断。
    #[must_use]
    pub const fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// 返回本次成功编译的资源与确定性观测值。
    ///
    /// 调用者可以在停表后读取该值并形成基线；读取不会遍历或复制 LIR。
    #[must_use]
    pub const fn metrics(&self) -> CompilationMetrics {
        self.metrics
    }
}
