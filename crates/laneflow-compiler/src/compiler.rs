//! 官方来源编译到原子已验证输出的公共入口。
//!
//! [`Compiler::compile`] 是唯一能够构造 [`ValidatedCanonicalLir`]、
//! [`ValidatedSourceMapInput`] 和 [`CompilationOutput`] 的路径。当前实现是干净单工作线程
//! 确定性预言机：每个阶段成功后才提交下一阶段，任一错误只返回
//! [`DiagnosticBundle`]；来源伴随数据在 AST/HIR/MIR 释放前冻结。

use laneflow_static_contract::{FieldTag, LaneEdgeId, LaneEdgeOrdinal};

use crate::hir::build_hir;
use crate::lir::{LirIdentityField, LirLaneEdge, LirUnit, freeze_lir};
use crate::mir::lower_to_mir;
use crate::source_map::{ValidatedSourceMapInput, freeze_source_map};
use crate::{CompilationUnit, Diagnostic, DiagnosticBundle};

/// 已完成 #292 当前支持子集全部静态语义验证的 Canonical LIR。
///
/// 字段保持私有，调用方只能按规范稳定顺序读取有类型表、身份字段和关系区间。不存在从
/// 裸表、未验证 MIR 或自报状态构造本类型的入口。
pub struct ValidatedCanonicalLir {
    inner: LirUnit,
}

impl ValidatedCanonicalLir {
    /// 按完整 Identity v1 前像规范顺序遍历全部车道图边。
    pub fn lane_edges(&self) -> impl ExactSizeIterator<Item = CanonicalLaneEdgeView<'_>> {
        self.inner
            .lane_edges
            .iter()
            .map(|edge| CanonicalLaneEdgeView {
                lir: &self.inner,
                edge,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取车道图边。
    ///
    /// 序号来自其他编译结果时可能命中错误实体；跨编译关联必须先使用 `LaneEdgeId`。
    #[must_use]
    pub fn lane_edge(&self, ordinal: LaneEdgeOrdinal) -> Option<CanonicalLaneEdgeView<'_>> {
        self.inner
            .lane_edges
            .get(ordinal.index())
            .map(|edge| CanonicalLaneEdgeView {
                lir: &self.inner,
                edge,
            })
    }
}

/// Canonical LIR 中一条 `LaneEdge` 记录的借用视图。
#[derive(Clone, Copy)]
pub struct CanonicalLaneEdgeView<'a> {
    lir: &'a LirUnit,
    edge: &'a LirLaneEdge,
}

impl CanonicalLaneEdgeView<'_> {
    /// 返回当前表中的有类型逻辑序号。
    #[must_use]
    pub const fn ordinal(&self) -> LaneEdgeOrdinal {
        self.edge.ordinal
    }

    /// 返回由完整 Identity v1 前像派生的稳定标识。
    #[must_use]
    pub const fn stable_id(&self) -> LaneEdgeId {
        self.edge.stable_id
    }

    /// 按 Identity v1 登记顺序遍历完整规范身份字段。
    pub fn identity_fields(&self) -> impl ExactSizeIterator<Item = CanonicalIdentityFieldView<'_>> {
        self.lir.identity_fields[self.edge.identity_fields.as_usize_range()]
            .iter()
            .map(|field| CanonicalIdentityFieldView {
                identity_field_bytes: &self.lir.identity_field_bytes,
                field,
            })
    }

    /// 返回交通权威长度，单位为米。
    #[must_use]
    pub const fn length_meters(&self) -> f64 {
        self.edge.length_meters
    }

    /// 返回基础道路限速，单位为米每秒。
    #[must_use]
    pub const fn speed_limit_meters_per_second(&self) -> f64 {
        self.edge.speed_limit_meters_per_second
    }

    /// 返回按领域顺序冻结的下游边有类型序号。
    #[must_use]
    pub fn successors(&self) -> &[LaneEdgeOrdinal] {
        &self.lir.lane_edge_successors[self.edge.successors.as_usize_range()]
    }
}

/// Canonical LIR 共享身份字段池中的一项借用视图。
#[derive(Clone, Copy)]
pub struct CanonicalIdentityFieldView<'a> {
    identity_field_bytes: &'a [u8],
    field: &'a LirIdentityField,
}

impl CanonicalIdentityFieldView<'_> {
    /// 返回 Identity v1 登记字段标签。
    #[must_use]
    pub const fn tag(&self) -> FieldTag {
        self.field.tag
    }

    /// 返回字段的完整规范值字节，不包含标签和长度前缀。
    #[must_use]
    pub fn value_bytes(&self) -> &[u8] {
        &self.identity_field_bytes[self.field.value_bytes.as_usize_range()]
    }
}

/// 一次成功编译原子拥有的已验证结果。
///
/// LIR 与来源伴随数据不能分别构造或重新配对；后继源映射后端必须从同一个实例同时借用
/// 二者。当前支持子集不产生 warning/note，因此 `diagnostics` 为空，但该成功契约保留
/// 非错误级诊断通道。
pub struct CompilationOutput {
    lir: ValidatedCanonicalLir,
    source_map_input: ValidatedSourceMapInput,
    diagnostics: Box<[Diagnostic]>,
}

impl CompilationOutput {
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
}

/// 可复用的 LaneFlow 静态路网生产编译器。
///
/// 当前干净单线程预言机没有跨编译语义状态；因此失败后无需清理缓存，也不可能让上次
/// 编译污染下一次结果。后继若加入容量复用，仍必须维持这一可观察契约。
pub struct Compiler {
    _private: (),
}

// #292 G1 只冻结显式 `Compiler::new()`，没有授权额外的公共 `Default` 构造契约。
#[allow(clippy::new_without_default)]
impl Compiler {
    /// 建立一个没有隐式输入、线程配置或无限资源模式的编译器实例。
    #[must_use]
    pub const fn new() -> Self {
        Self { _private: () }
    }

    /// 消费一个受检编译单元并运行完整的当前生产编译管线。
    ///
    /// # Errors
    ///
    /// 任一阶段出现错误级语义诊断或超过 `CompilationUnit` 携带的资源配置档时，返回
    /// 规范排序的 [`DiagnosticBundle`]。错误路径不会返回部分 LIR 或部分源映射输入；
    /// 同一个 `Compiler` 可以立即用于下一次编译。
    pub fn compile(
        &mut self,
        unit: CompilationUnit,
    ) -> Result<CompilationOutput, DiagnosticBundle> {
        let hir = build_hir(&unit)?;
        let mir = lower_to_mir(&unit, &hir)?;
        // MIR 已拥有后继阶段所需的完整语义与来源位置；尽早释放 HIR，避免把阶段共存
        // 时间延长到 LIR/source-map 冻结并破坏资源峰值模型。
        drop(hir);
        let frozen_lir = freeze_lir(&unit, &mir)?;
        let source_map_input = freeze_source_map(unit, &mir, &frozen_lir)?;
        drop(mir);
        let crate::lir::LirFreezeOutput { lir, .. } = frozen_lir;
        Ok(CompilationOutput {
            lir: ValidatedCanonicalLir { inner: lir },
            source_map_input,
            diagnostics: Box::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CompilationUnitBuilder, CompileLimitDimension, CompileLimits, DiagnosticPayload,
        LaneEdgeInput, LaneEdgeReference, SourceModuleDescriptor, SourceModuleHeader,
        SourceModuleHeaderInput, SourceRelationRole, SyntheticModule, SyntheticModuleBuilder,
    };

    fn module(
        namespace: &str,
        document: &str,
        imports: &[&str],
        edges: &[(&str, f64, &[LaneEdgeReference<'_>])],
    ) -> SyntheticModule {
        let limits = CompileLimits::p100_initial_v1();
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: namespace,
                source_document_key: document,
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x11; 32],
                frontend_options_digest: [0x22; 32],
                random_seed: Some(42),
                provenance: "repository:laneflow",
            },
            &limits,
        )
        .unwrap();
        let mut builder = SyntheticModuleBuilder::new(header, &limits).unwrap();
        for import in imports {
            builder.add_import(import).unwrap();
        }
        for (key, length_meters, successors) in edges {
            builder
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: key,
                    length_meters: *length_meters,
                    speed_limit_meters_per_second: 13.75,
                    successors,
                })
                .unwrap();
        }
        builder.finish().unwrap()
    }

    fn unit(modules: impl IntoIterator<Item = SyntheticModule>) -> CompilationUnit {
        let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
        for module in modules {
            builder.add_synthetic_module(module).unwrap();
        }
        builder.build().unwrap()
    }

    fn edge_key(edge: CanonicalLaneEdgeView<'_>) -> String {
        edge.identity_fields()
            .find(|field| field.tag() == FieldTag::LaneEdgeKey)
            .map(|field| String::from_utf8(field.value_bytes().to_vec()).unwrap())
            .unwrap()
    }

    #[test]
    fn compiler_atomically_returns_lir_source_map_and_success_diagnostics() {
        let successors = [LaneEdgeReference::imported("city/base", "edge-b")];
        let input = unit([
            module(
                "city/app",
                "app.document",
                &["city/base"],
                &[("edge-a", 10.0, &successors)],
            ),
            module("city/base", "base.document", &[], &[("edge-b", 20.0, &[])]),
        ]);
        let output = Compiler::new().compile(input).unwrap();

        assert!(output.diagnostics().is_empty());
        let edges = output.lir().lane_edges().collect::<Vec<_>>();
        assert_eq!(edges.len(), 2);
        assert_eq!(edge_key(edges[0]), "edge-a");
        assert_eq!(edges[0].ordinal().raw(), 0);
        assert_eq!(edges[0].successors(), [LaneEdgeOrdinal::from_raw(1)]);
        assert_eq!(edges[0].length_meters(), 10.0);
        assert_eq!(edges[0].speed_limit_meters_per_second(), 13.75);
        assert_eq!(
            output
                .lir()
                .lane_edge(edges[1].ordinal())
                .unwrap()
                .stable_id(),
            edges[1].stable_id()
        );

        let modules = output
            .source_map_input()
            .source_modules()
            .map(SourceModuleDescriptor::authoring_namespace_id)
            .collect::<Vec<_>>();
        assert_eq!(modules, ["city/base", "city/app"]);
        let documents = output
            .source_map_input()
            .source_documents()
            .map(|document| document.source_document_key())
            .collect::<Vec<_>>();
        assert_eq!(documents, ["base.document", "app.document"]);

        let entity_sources = output
            .source_map_input()
            .lane_edge_sources()
            .collect::<Vec<_>>();
        assert_eq!(entity_sources.len(), 2);
        for (edge, source) in edges.iter().zip(entity_sources) {
            assert_eq!(source.ordinal(), edge.ordinal());
            assert_eq!(source.stable_id(), edge.stable_id());
            assert!(source.contributing_sources().next().is_none());
        }
        assert_eq!(
            output
                .source_map_input()
                .lane_edge_successor_sources()
                .map(|source| (
                    source.owner_ordinal().raw(),
                    source.role(),
                    source.local_index(),
                    source.primary_source().source_document_key().to_owned(),
                ))
                .collect::<Vec<_>>(),
            [(
                0,
                SourceRelationRole::LaneEdgeSuccessor,
                0,
                "app.document".to_owned(),
            )]
        );
    }

    #[test]
    fn source_changes_do_not_change_lir_semantic_digest() {
        let left = unit([module(
            "city/a",
            "left.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]);
        let right = unit([module(
            "city/a",
            "right.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]);
        let mut compiler = Compiler::new();
        let left = compiler.compile(left).unwrap();
        let right = compiler.compile(right).unwrap();

        assert_eq!(
            left.lir.inner.semantic_digest,
            right.lir.inner.semantic_digest
        );
        assert_ne!(
            left.source_map_input()
                .lane_edge_sources()
                .next()
                .unwrap()
                .primary_source()
                .source_document_key(),
            right
                .source_map_input()
                .lane_edge_sources()
                .next()
                .unwrap()
                .primary_source()
                .source_document_key()
        );
    }

    #[test]
    fn thirty_two_failures_do_not_pollute_reused_compiler() {
        let missing = [LaneEdgeReference::local("missing")];
        let mut compiler = Compiler::new();
        for index in 0..32 {
            let failed = unit([module(
                &format!("failed/{index}"),
                &format!("failed-{index}.document"),
                &[],
                &[("edge-a", 10.0, &missing)],
            )]);
            let diagnostics = match compiler.compile(failed) {
                Ok(_) => panic!("expected failed compilation"),
                Err(diagnostics) => diagnostics,
            };
            assert!(matches!(
                diagnostics.diagnostics()[0].payload(),
                DiagnosticPayload::UnknownReferenceTarget { .. }
            ));
        }

        let recovered = unit([module(
            "city/a",
            "city-a.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]);
        let fresh = unit([module(
            "city/a",
            "city-a.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]);
        assert_eq!(
            compiler
                .compile(recovered)
                .unwrap()
                .lir
                .inner
                .semantic_digest,
            Compiler::new()
                .compile(fresh)
                .unwrap()
                .lir
                .inner
                .semantic_digest
        );
    }

    #[test]
    fn source_map_output_limit_fails_after_lir_without_exposing_partial_output() {
        let probe = unit([module(
            "city/a",
            "city-a.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]);
        let hir = build_hir(&probe).unwrap();
        let mir = lower_to_mir(&probe, &hir).unwrap();
        let lir_output_bytes = freeze_lir(&probe, &mir).unwrap().lir.output_bytes;

        let mut constrained = unit([module(
            "city/a",
            "city-a.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]);
        constrained.limits = CompileLimits::p100_initial_v1().with_test_lir_limits(
            u32::MAX,
            u32::MAX,
            u32::try_from(lir_output_bytes).unwrap(),
            u32::MAX,
        );
        let mut compiler = Compiler::new();
        let diagnostics = match compiler.compile(constrained) {
            Ok(_) => panic!("expected source-map output limit failure"),
            Err(diagnostics) => diagnostics,
        };
        assert!(diagnostics.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::OutputBytes,
                limit,
                observed,
            } if *limit == lir_output_bytes && observed > limit
        )));

        let recovered = unit([module(
            "city/recovered",
            "recovered.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]);
        assert!(compiler.compile(recovered).is_ok());
    }
}
