//! Typed AST 到高层中间表示（HIR）的符号解析阶段。
//!
//! 输入 [`CompilationUnit`] 已闭合模块导入图并冻结依赖优先顺序。本阶段据此建立连续
//! 模块表和车道图边符号表，把 `(module namespace, stable key)` 引用解析为阶段私有
//! `u32` 键，并保留来源位置供后续诊断/源映射使用。声明先全部登记、再统一解析引用，
//! 因此前向引用和自环合法。
//!
//! HIR 表顺序是规范顺序：模块沿用编译单元顺序，模块内声明按稳定键排序，导入和连接
//! 也使用已显式规范化的序列。`HashMap` 仅作查找，绝不能通过迭代哈希表决定诊断或
//! 后续布局。所有键、区间和类型均为 crate 私有，不能跨阶段或进入持久制品。

use std::collections::HashMap;
use std::sync::Arc;

use laneflow_static_contract::EntityKind;

use crate::arena::{ArenaKey, ArenaKeyOverflow, TableRange, TypedArena};
use crate::declaration::{LaneEdgeDeclaration, SyntheticDeclaration};
use crate::diagnostic::DiagnosticCollector;
use crate::{CompilationUnit, CompileLimitDimension, Diagnostic, DiagnosticBundle, SourceSpan};

/// 区分 HIR 模块表键的零尺寸阶段标记。
pub(crate) enum HirModuleTag {}
/// 区分 HIR 车道图边表键的零尺寸阶段标记。
pub(crate) enum HirLaneEdgeTag {}

/// 仅在当前 `HirUnit` 模块表内有效的致密键。
pub(crate) type HirModuleKey = ArenaKey<HirModuleTag>;
/// 仅在当前 `HirUnit` 车道图边表内有效的致密键。
pub(crate) type HirLaneEdgeKey = ArenaKey<HirLaneEdgeTag>;

/// 已解析为 HIR 模块键的显式导入边。
pub(crate) struct HirImport {
    /// 被导入模块；目标在规范模块顺序中位于当前模块之前。
    pub(crate) target: HirModuleKey,
    /// 原始导入声明位置。
    pub(crate) source_span: SourceSpan,
}

/// HIR 模块记录及其在平坦导入表中的连续区间。
pub(crate) struct HirModule {
    /// 声明身份与跨模块解析使用的稳定命名空间。
    pub(crate) authoring_namespace_id: Arc<str>,
    /// 与机器路径无关的来源文档键。
    pub(crate) source_document_key: Arc<str>,
    /// 此模块在 `HirUnit::imports` 中的半开区间。
    pub(crate) imports: TableRange<HirImport>,
    /// 模块声明位置。
    pub(crate) source_span: SourceSpan,
}

/// 已解析为 HIR 车道图边键的下游引用。
pub(crate) struct HirLaneEdgeReference {
    /// 当前 `HirUnit::lane_edges` 中的目标键。
    pub(crate) target: HirLaneEdgeKey,
    /// 原始引用位置。
    pub(crate) source_span: SourceSpan,
}

/// 完成模块归属和下游符号解析的车道图边 HIR 记录。
pub(crate) struct HirLaneEdge {
    /// 拥有此声明的 HIR 模块。
    pub(crate) module: HirModuleKey,
    /// 模块内稳定键；不是 HIR 致密下标。
    pub(crate) stable_key: Arc<str>,
    /// 交通权威长度，单位为米并保留来源 `f64` 精度。
    pub(crate) length_meters: f64,
    /// 基础道路限速，单位为米每秒并保留来源 `f64` 精度。
    pub(crate) speed_limit_meters_per_second: f64,
    /// 此边在 `HirUnit::lane_edge_references` 中的连续下游引用区间。
    pub(crate) successors: TableRange<HirLaneEdgeReference>,
    /// 原始声明位置。
    pub(crate) source_span: SourceSpan,
}

/// HIR 阶段成功后一次性冻结的连续只读表集合。
///
/// 构造完成时所有引用均已解析，所有 `TableRange` 都落在对应平坦表内。字段中的键只对
/// 本实例有效。`controlled_live_bytes` 仅统计成功返回后由 HIR 自身持有的阶段字节；
/// 资源预检使用的峰值还包含输入、查找表和暂存区。
pub(crate) struct HirUnit {
    pub(crate) modules: Box<[HirModule]>,
    pub(crate) imports: Box<[HirImport]>,
    pub(crate) lane_edges: Box<[HirLaneEdge]>,
    pub(crate) lane_edge_references: Box<[HirLaneEdgeReference]>,
    pub(crate) hir_record_count: u64,
    pub(crate) controlled_live_bytes: u64,
}

/// 按 HIR 模块隔离的车道图边查找索引；不提供规范遍历能力。
struct LaneEdgeSymbolTable {
    by_module: Vec<HashMap<Arc<str>, HirLaneEdgeKey>>,
}

impl LaneEdgeSymbolTable {
    fn new(module_declaration_counts: impl IntoIterator<Item = usize>) -> Self {
        Self {
            by_module: module_declaration_counts
                .into_iter()
                .map(HashMap::with_capacity)
                .collect(),
        }
    }

    fn insert(&mut self, module: HirModuleKey, stable_key: Arc<str>, key: HirLaneEdgeKey) {
        let previous = self.by_module[module.index()].insert(stable_key, key);
        debug_assert!(
            previous.is_none(),
            "Typed AST rejected duplicate declarations"
        );
    }

    fn get(&self, module: HirModuleKey, stable_key: &str) -> Option<HirLaneEdgeKey> {
        self.by_module[module.index()].get(stable_key).copied()
    }
}

#[derive(Clone, Copy)]
/// 把规范 HIR 键映回 Typed AST 物理位置的阶段暂存记录。
///
/// HIR 键不能冒充来源模块/声明下标；显式保存两者可在声明排序后仍准确读取来源记录。
struct CanonicalLaneEdgeSource {
    source_module_index: u32,
    declaration_index: u32,
    hir_key: HirLaneEdgeKey,
}

/// 建立模块/符号表并解析编译单元中的全部车道图边引用。
///
/// # Errors
///
/// 当 HIR 记录数、阶段暂存区、编译器控制存续字节或 `u32` 表边界超过所选配置档，
/// 或任一目标稳定键不存在时，返回规范有序诊断。失败不会返回部分 HIR。
pub(crate) fn build_hir(unit: &CompilationUnit) -> Result<HirUnit, DiagnosticBundle> {
    // 在任何与记录数成正比的阶段分配前，同时预检持久表、lookup 预算和阶段最大暂存区。
    // scratch 取互斥工作集的最大值而非总和，live peak 则包含输入与当时存续的全部集合。
    let module_count = u64::try_from(unit.modules.len()).unwrap_or(u64::MAX);
    let hir_record_count = module_count
        .saturating_add(unit.import_edge_count)
        .saturating_add(unit.symbol_count)
        .saturating_add(unit.identity_field_occurrence_count)
        .saturating_add(unit.reference_count)
        .saturating_add(unit.relation_occurrence_count);
    let canonical_source_scratch =
        requested_bytes::<CanonicalLaneEdgeSource>(unit.declaration_count)
            .saturating_add(requested_bytes::<usize>(unit.declaration_count));
    let import_sort_scratch = requested_bytes::<(&str, &SourceSpan)>(unit.import_edge_count);
    let stage_scratch_bytes = canonical_source_scratch.max(import_sort_scratch);
    let hir_persistent_bytes = requested_bytes::<HirModule>(module_count)
        .saturating_add(requested_bytes::<HirImport>(unit.import_edge_count))
        .saturating_add(requested_bytes::<HirLaneEdge>(unit.declaration_count))
        .saturating_add(requested_bytes::<HirLaneEdgeReference>(
            unit.reference_count,
        ));
    let hir_lookup_bytes = requested_hash_table_bytes::<Arc<str>, HirModuleKey>(module_count)
        .saturating_add(requested_bytes::<HashMap<Arc<str>, HirLaneEdgeKey>>(
            module_count,
        ))
        .saturating_add(requested_hash_table_bytes::<Arc<str>, HirLaneEdgeKey>(
            unit.symbol_count,
        ));
    let controlled_live_bytes = unit
        .controlled_live_bytes
        .saturating_add(hir_persistent_bytes)
        .saturating_add(hir_lookup_bytes)
        .saturating_add(stage_scratch_bytes);

    let primary_span = unit
        .modules
        .first()
        .map(|module| module.descriptor().declaration_span().clone());
    let stable_key = unit
        .modules
        .first()
        .map(|module| module.descriptor().authoring_namespace_id().into());
    let mut limit_diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    for (dimension, observed) in [
        (CompileLimitDimension::HirRecordCount, hir_record_count),
        (
            CompileLimitDimension::StageScratchBytes,
            stage_scratch_bytes,
        ),
        (
            CompileLimitDimension::CompilerControlledLiveBytes,
            controlled_live_bytes,
        ),
    ] {
        if observed > unit.limits.value(dimension) {
            limit_diagnostics.push(Diagnostic::compile_limit_exceeded_at(
                dimension,
                unit.limits.value(dimension),
                observed,
                primary_span.clone(),
                stable_key.clone(),
            ));
        }
    }
    if !limit_diagnostics.is_empty() {
        return Err(limit_diagnostics.finish());
    }

    let module_capacity = unit.modules.len();
    let import_capacity = count_to_usize(unit.import_edge_count, &unit.limits)?;
    let declaration_capacity = count_to_usize(unit.declaration_count, &unit.limits)?;
    let reference_capacity = count_to_usize(unit.reference_count, &unit.limits)?;
    // 第一阶段冻结模块键。CompilationUnit 已按依赖优先排序，因此 raw key 顺序可直接
    // 作为后续规范模块轴；module_lookup 只用于解析，不参与任何输出遍历。
    let mut modules = TypedArena::<HirModuleTag, HirModule>::with_capacity(module_capacity);
    let mut module_lookup = HashMap::with_capacity(module_capacity);
    for source_module in &unit.modules {
        let key = modules
            .push(HirModule {
                authoring_namespace_id: source_module.descriptor().authoring_namespace_arc(),
                source_document_key: source_module.descriptor().source_document_key_arc(),
                imports: TableRange::empty(),
                source_span: source_module.descriptor().declaration_span().clone(),
            })
            .map_err(|overflow| arena_overflow(overflow, &unit.limits, primary_span.clone()))?;
        module_lookup.insert(source_module.descriptor().authoring_namespace_arc(), key);
    }

    // 每个模块的导入单独按目标命名空间排序后追加到一个平坦表，TableRange 保留模块
    // 边界并避免每模块 Vec 的额外分配。
    let mut imports = Vec::with_capacity(import_capacity);
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key =
            HirModuleKey::from_raw(u32::try_from(module_index).map_err(|_| {
                arena_overflow(ArenaKeyOverflow, &unit.limits, primary_span.clone())
            })?);
        let start = imports.len();
        let mut canonical_imports: Vec<_> = source_module.import_records().collect();
        canonical_imports.sort_unstable_by(|left, right| left.0.cmp(right.0));
        for (target_namespace, source_span) in canonical_imports {
            let target = module_lookup[target_namespace];
            imports.push(HirImport {
                target,
                source_span: source_span.clone(),
            });
        }
        modules.get_mut(module_key).imports =
            TableRange::try_from_usize(start, imports.len().saturating_sub(start))
                .map_err(|overflow| arena_overflow(overflow, &unit.limits, primary_span.clone()))?;
    }

    // 先按 `(canonical module order, stable key)` 为全部声明分配键并建立完整符号表。
    // 这一步必须先于连接解析，才能让前向引用、自环和跨模块引用具有相同语义。
    let mut lane_edges =
        TypedArena::<HirLaneEdgeTag, HirLaneEdge>::with_capacity(declaration_capacity);
    let mut symbols =
        LaneEdgeSymbolTable::new(unit.modules.iter().map(|module| module.declarations.len()));
    let mut canonical_sources = Vec::with_capacity(declaration_capacity);
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key =
            HirModuleKey::from_raw(u32::try_from(module_index).map_err(|_| {
                arena_overflow(ArenaKeyOverflow, &unit.limits, primary_span.clone())
            })?);
        let mut declaration_indices: Vec<usize> = (0..source_module.declarations.len()).collect();
        declaration_indices.sort_unstable_by(|left, right| {
            declaration(&source_module.declarations[*left])
                .header
                .stable_key
                .cmp(
                    &declaration(&source_module.declarations[*right])
                        .header
                        .stable_key,
                )
        });
        for declaration_index in declaration_indices {
            let source = declaration(&source_module.declarations[declaration_index]);
            let key = lane_edges
                .push(HirLaneEdge {
                    module: module_key,
                    stable_key: Arc::clone(&source.header.stable_key),
                    length_meters: source.length.value(),
                    speed_limit_meters_per_second: source.speed_limit.value(),
                    successors: TableRange::empty(),
                    source_span: source.header.span.clone(),
                })
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?;
            symbols.insert(module_key, Arc::clone(&source.header.stable_key), key);
            canonical_sources.push(CanonicalLaneEdgeSource {
                source_module_index: u32::try_from(module_index).map_err(|_| {
                    arena_overflow(
                        ArenaKeyOverflow,
                        &unit.limits,
                        Some(source.header.span.clone()),
                    )
                })?,
                declaration_index: u32::try_from(declaration_index).map_err(|_| {
                    arena_overflow(
                        ArenaKeyOverflow,
                        &unit.limits,
                        Some(source.header.span.clone()),
                    )
                })?,
                hir_key: key,
            });
        }
    }

    // 第二遍只解析已经规范化的引用序列。未知目标继续收集到有界诊断集合中；该边的
    // 临时区间不会在失败时泄漏，因为整个 HirUnit 仅在零错误后提交。
    let mut references = Vec::with_capacity(reference_capacity);
    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    for source_location in canonical_sources {
        let module_index = usize::try_from(source_location.source_module_index)
            .expect("u32 module index must fit usize on supported targets");
        let source_module = &unit.modules[module_index];
        let source = declaration(
            &source_module.declarations[usize::try_from(source_location.declaration_index)
                .expect("u32 declaration index must fit usize on supported targets")],
        );
        let start = references.len();
        for successor in &source.successors {
            let target_module = module_lookup[successor.module_namespace.as_ref()];
            let Some(target) = symbols.get(target_module, &successor.declaration_key) else {
                let mut diagnostic = Diagnostic::unknown_reference_target(
                    EntityKind::LaneEdge,
                    &source.header.stable_key,
                    &successor.module_namespace,
                    &successor.declaration_key,
                    successor.span.clone(),
                    source.header.span.clone(),
                );
                diagnostic.set_canonical_module_order(source_location.source_module_index);
                diagnostics.push(diagnostic);
                continue;
            };
            references.push(HirLaneEdgeReference {
                target,
                source_span: successor.span.clone(),
            });
        }
        lane_edges.get_mut(source_location.hir_key).successors =
            TableRange::try_from_usize(start, references.len().saturating_sub(start)).map_err(
                |overflow| arena_overflow(overflow, &unit.limits, Some(source.header.span.clone())),
            )?;
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    debug_assert_eq!(modules.len(), module_capacity);
    debug_assert_eq!(lane_edges.len(), declaration_capacity);
    debug_assert_eq!(references.len(), reference_capacity);
    Ok(HirUnit {
        modules: modules.into_boxed_slice(),
        imports: imports.into_boxed_slice(),
        lane_edges: lane_edges.into_boxed_slice(),
        lane_edge_references: references.into_boxed_slice(),
        hir_record_count,
        controlled_live_bytes: hir_persistent_bytes,
    })
}

fn declaration(declaration: &SyntheticDeclaration) -> &LaneEdgeDeclaration {
    match declaration {
        SyntheticDeclaration::LaneEdge(declaration) => declaration,
    }
}

fn requested_bytes<T>(count: u64) -> u64 {
    count.saturating_mul(u64::try_from(size_of::<T>()).unwrap_or(u64::MAX))
}

fn requested_hash_table_bytes<K, V>(entry_count: u64) -> u64 {
    if entry_count == 0 {
        return 0;
    }
    // 标准库不公开桶分配布局。预检为每个请求项预留八个桶，并额外计入每桶控制字节
    // 与一组尾部控制区，覆盖小表最小桶数和负载因子取整，而不依赖哈希表迭代顺序。
    // 真实生产基准仍须另报实际容量和进程内存，不能用本预算冒充测量。
    let bucket_bytes = u64::try_from(size_of::<(K, V)>())
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    entry_count
        .saturating_mul(8)
        .saturating_mul(bucket_bytes)
        .saturating_add(16)
}

fn count_to_usize(count: u64, limits: &crate::CompileLimits) -> Result<usize, DiagnosticBundle> {
    usize::try_from(count).map_err(|_| arena_overflow(ArenaKeyOverflow, limits, None))
}

fn arena_overflow(
    _: ArenaKeyOverflow,
    limits: &crate::CompileLimits,
    primary_span: Option<SourceSpan>,
) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::compile_limit_exceeded_at(
        CompileLimitDimension::HirRecordCount,
        limits.value(CompileLimitDimension::HirRecordCount),
        u64::from(u32::MAX) + 1,
        primary_span,
        None,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CompilationUnitBuilder, CompileLimits, DiagnosticCode, DiagnosticPayload, LaneEdgeInput,
        LaneEdgeReference, SourceModuleHeader, SourceModuleHeaderInput, SyntheticModule,
        SyntheticModuleBuilder,
    };

    fn header(namespace: &str) -> SourceModuleHeader {
        SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: namespace,
                source_document_key: namespace,
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x11; 32],
                frontend_options_digest: [0x22; 32],
                random_seed: Some(42),
                provenance: "repository:laneflow",
            },
            &CompileLimits::p100_initial_v1(),
        )
        .unwrap()
    }

    fn module(
        namespace: &str,
        imports: &[&str],
        edges: &[(&str, &[LaneEdgeReference<'_>])],
    ) -> SyntheticModule {
        let limits = CompileLimits::p100_initial_v1();
        let mut builder = SyntheticModuleBuilder::new(header(namespace), &limits).unwrap();
        for import in imports {
            builder.add_import(import).unwrap();
        }
        for (key, successors) in edges {
            builder
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: key,
                    length_meters: 12.5,
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

    #[test]
    fn hir_resolves_local_and_imported_lane_edge_references_to_typed_keys() {
        let base = module("city/base", &[], &[("edge-b", &[])]);
        let app_successors = [
            LaneEdgeReference::imported("city/base", "edge-b"),
            LaneEdgeReference::local("edge-c"),
        ];
        let app = module(
            "city/app",
            &["city/base"],
            &[("edge-c", &[]), ("edge-a", &app_successors)],
        );
        let unit = unit([app, base]);
        let hir = build_hir(&unit).unwrap();

        assert_eq!(hir.modules.len(), 2);
        assert_eq!(hir.modules[0].authoring_namespace_id.as_ref(), "city/base");
        assert_eq!(hir.modules[1].authoring_namespace_id.as_ref(), "city/app");
        assert_eq!(hir.imports.len(), 1);
        assert_eq!(hir.imports[0].target.raw(), 0);
        assert_eq!(hir.imports[0].source_span.source_document_key(), "city/app");
        assert_eq!(hir.modules[1].imports.start(), 0);
        assert_eq!(hir.modules[1].imports.len(), 1);
        assert_eq!(hir.lane_edges.len(), 3);
        assert_eq!(
            hir.lane_edges
                .iter()
                .map(|edge| edge.stable_key.as_ref())
                .collect::<Vec<_>>(),
            ["edge-b", "edge-a", "edge-c"]
        );
        let edge_a = &hir.lane_edges[1];
        let targets = hir.lane_edge_references[edge_a.successors.as_usize_range()]
            .iter()
            .map(|reference| reference.target.raw())
            .collect::<Vec<_>>();
        assert_eq!(targets, [2, 0]);
        assert!(hir.modules[0].imports.is_empty());
        assert_eq!(hir.hir_record_count, 16);
    }

    #[test]
    fn hir_reports_every_unknown_target_in_canonical_module_order() {
        let z_successors = [LaneEdgeReference::local("missing-z")];
        let a_successors = [LaneEdgeReference::local("missing-a")];
        let unit = unit([
            module("city/z", &[], &[("edge-z", &z_successors)]),
            module("city/a", &[], &[("edge-a", &a_successors)]),
        ]);
        let diagnostics = match build_hir(&unit) {
            Ok(_) => panic!("unknown targets must reject HIR construction"),
            Err(diagnostics) => diagnostics,
        };

        assert_eq!(diagnostics.diagnostics().len(), 2);
        assert_eq!(
            diagnostics
                .diagnostics()
                .iter()
                .map(|diagnostic| diagnostic.stable_key().unwrap())
                .collect::<Vec<_>>(),
            ["edge-a", "edge-z"]
        );
        assert!(diagnostics.diagnostics().iter().all(|diagnostic| {
            diagnostic.code() == DiagnosticCode::UnknownReferenceTarget
                && diagnostic.primary_span().is_some()
                && diagnostic.related_spans().len() == 1
        }));
    }

    #[test]
    fn hir_symbol_and_reference_order_ignore_declaration_insertion_order() {
        let successors = [
            LaneEdgeReference::local("edge-c"),
            LaneEdgeReference::local("edge-b"),
        ];
        let left = unit([module(
            "city/a",
            &[],
            &[("edge-a", &successors), ("edge-b", &[]), ("edge-c", &[])],
        )]);
        let right = unit([module(
            "city/a",
            &[],
            &[("edge-c", &[]), ("edge-a", &successors), ("edge-b", &[])],
        )]);
        let left = build_hir(&left).unwrap();
        let right = build_hir(&right).unwrap();

        let projection = |hir: &HirUnit| {
            hir.lane_edges
                .iter()
                .map(|edge| {
                    (
                        edge.stable_key.to_string(),
                        hir.lane_edge_references[edge.successors.as_usize_range()]
                            .iter()
                            .map(|reference| reference.target.raw())
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(projection(&left), projection(&right));
        assert_eq!(
            projection(&left),
            [
                ("edge-a".into(), vec![1, 2]),
                ("edge-b".into(), vec![]),
                ("edge-c".into(), vec![]),
            ]
        );
    }

    #[test]
    fn hir_checks_record_scratch_and_live_byte_limits_before_stage_allocation() {
        let mut unit = unit([module("city/a", &[], &[("edge-a", &[])])]);
        unit.limits = CompileLimits::p100_initial_v1().with_test_pipeline_limits(
            3,
            u32::MAX,
            u32::MAX,
            u32::MAX,
        );
        let record_failure = match build_hir(&unit) {
            Ok(_) => panic!("HIR record limit must fail closed"),
            Err(diagnostics) => diagnostics,
        };
        assert!(matches!(
            record_failure.diagnostics()[0].payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::HirRecordCount,
                limit: 3,
                observed: 4,
            }
        ));

        unit.limits = CompileLimits::p100_initial_v1().with_test_pipeline_limits(
            u32::MAX,
            u32::MAX,
            0,
            u32::MAX,
        );
        let scratch_failure = match build_hir(&unit) {
            Ok(_) => panic!("HIR scratch limit must fail closed"),
            Err(diagnostics) => diagnostics,
        };
        assert!(
            scratch_failure
                .diagnostics()
                .iter()
                .any(|diagnostic| matches!(
                    diagnostic.payload(),
                    DiagnosticPayload::CompileLimitExceeded {
                        dimension: CompileLimitDimension::StageScratchBytes,
                        limit: 0,
                        observed,
                    } if *observed > 0
                ))
        );

        let source_live_bytes = u32::try_from(unit.controlled_live_bytes).unwrap();
        unit.limits = CompileLimits::p100_initial_v1().with_test_pipeline_limits(
            u32::MAX,
            u32::MAX,
            u32::MAX,
            source_live_bytes,
        );
        let live_failure = match build_hir(&unit) {
            Ok(_) => panic!("HIR live byte limit must fail closed"),
            Err(diagnostics) => diagnostics,
        };
        assert!(live_failure.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::CompilerControlledLiveBytes,
                limit,
                observed,
            } if *limit == u64::from(source_live_bytes) && observed > limit
        )));
    }
}
