use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::diagnostic::DiagnosticCollector;
use crate::source::external_token_violation;
use crate::{
    CompileLimitDimension, CompileLimits, Diagnostic, DiagnosticBundle, SourceModuleHeader,
    SourceSpan,
};

const SOURCE_RECORD_MAGIC: [u8; 8] = *b"LFSOURCE";

/// 首版合成领域专用语言来源记录编码版本。
pub const SYNTHETIC_FRONTEND_VERSION: u32 = 1;

/// 官方来源模块使用的来源语言。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u16)]
#[non_exhaustive]
pub enum SourceLanguage {
    SyntheticDsl = 1,
}

impl SourceLanguage {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SyntheticDsl => "synthetic-dsl",
        }
    }
}

/// 由官方前端派生、调用方无法独立构造的来源模块描述符。
pub struct SourceModuleDescriptor {
    authoring_namespace_id: Arc<str>,
    source_language: SourceLanguage,
    source_content_digest: [u8; 32],
    source_record_byte_len: u32,
    frontend_version: u32,
    frontend_options_digest: [u8; 32],
    generator_build_id: Arc<str>,
    parameters_and_inputs_digest: [u8; 32],
    random_seed: Option<u64>,
    provenance: Arc<str>,
    source_document_key: Arc<str>,
    imports: Box<[Arc<str>]>,
    declaration_span: SourceSpan,
}

impl SourceModuleDescriptor {
    #[must_use]
    pub fn authoring_namespace_id(&self) -> &str {
        &self.authoring_namespace_id
    }

    #[must_use]
    pub const fn source_language(&self) -> SourceLanguage {
        self.source_language
    }

    #[must_use]
    pub const fn source_content_digest(&self) -> &[u8; 32] {
        &self.source_content_digest
    }

    #[must_use]
    pub const fn source_record_byte_len(&self) -> u32 {
        self.source_record_byte_len
    }

    #[must_use]
    pub const fn frontend_version(&self) -> u32 {
        self.frontend_version
    }

    #[must_use]
    pub const fn frontend_options_digest(&self) -> &[u8; 32] {
        &self.frontend_options_digest
    }

    #[must_use]
    pub fn generator_build_id(&self) -> &str {
        &self.generator_build_id
    }

    #[must_use]
    pub const fn parameters_and_inputs_digest(&self) -> &[u8; 32] {
        &self.parameters_and_inputs_digest
    }

    #[must_use]
    pub const fn random_seed(&self) -> Option<u64> {
        self.random_seed
    }

    #[must_use]
    pub fn provenance(&self) -> &str {
        &self.provenance
    }

    #[must_use]
    pub fn source_document_key(&self) -> &str {
        &self.source_document_key
    }

    pub fn imports(&self) -> impl ExactSizeIterator<Item = &str> {
        self.imports.iter().map(AsRef::as_ref)
    }
}

struct ImportRecord {
    namespace: Arc<str>,
    span: SourceSpan,
}

/// 官方合成领域专用语言来源模块的受检构建器。
pub struct SyntheticModuleBuilder {
    header: SourceModuleHeader,
    limits: CompileLimits,
    imports: Vec<ImportRecord>,
    import_index: HashMap<Arc<str>, usize>,
    string_item_count: u64,
    string_bytes: u64,
    controlled_string_bytes: u64,
    source_record_byte_len: u64,
}

impl SyntheticModuleBuilder {
    /// 建立一个只允许官方合成领域构造的来源模块构建器。
    pub fn new(
        header: SourceModuleHeader,
        limits: &CompileLimits,
    ) -> Result<Self, DiagnosticBundle> {
        let string_bytes = header_resident_string_bytes(&header);
        let controlled_string_bytes = header_controlled_string_bytes(&header);
        let string_item_count = 2;
        let base_source_bytes = encoded_source_record_len(&header, &[]).unwrap_or(u64::MAX);
        let mut diagnostics =
            DiagnosticCollector::new(limits.value(CompileLimitDimension::DiagnosticCount));
        push_limit_if_exceeded(
            &mut diagnostics,
            limits,
            CompileLimitDimension::StringItemCount,
            string_item_count,
            Some(header.declaration_span.clone()),
            Some(header.authoring_namespace_id.as_ref().into()),
        );
        let controlled_live_bytes = controlled_string_bytes.saturating_add(base_source_bytes);
        push_limit_if_exceeded(
            &mut diagnostics,
            limits,
            CompileLimitDimension::CompilerControlledLiveBytes,
            controlled_live_bytes,
            Some(header.declaration_span.clone()),
            Some(header.authoring_namespace_id.as_ref().into()),
        );
        push_limit_if_exceeded(
            &mut diagnostics,
            limits,
            CompileLimitDimension::TotalStringBytes,
            string_bytes,
            Some(header.declaration_span.clone()),
            Some(header.authoring_namespace_id.as_ref().into()),
        );
        push_limit_if_exceeded(
            &mut diagnostics,
            limits,
            CompileLimitDimension::SourceBytesPerModule,
            base_source_bytes,
            Some(header.declaration_span.clone()),
            Some(header.authoring_namespace_id.as_ref().into()),
        );
        if !diagnostics.is_empty() {
            return Err(diagnostics.finish());
        }

        Ok(Self {
            header,
            limits: limits.clone(),
            imports: Vec::new(),
            import_index: HashMap::new(),
            string_item_count,
            string_bytes,
            controlled_string_bytes,
            source_record_byte_len: base_source_bytes,
        })
    }

    /// 声明显式模块导入；网络或文件系统发现不属于该操作。
    #[track_caller]
    pub fn add_import(&mut self, namespace: &str) -> Result<&mut Self, DiagnosticBundle> {
        let span = SourceSpan::at_caller(
            Arc::clone(&self.header.source_document_key),
            std::panic::Location::caller(),
        );
        let single_string_limit = self.limits.value(CompileLimitDimension::SingleStringBytes);
        if let Some(violation) = external_token_violation(namespace, single_string_limit) {
            return Err(DiagnosticBundle::single(
                Diagnostic::invalid_import_namespace(violation, span),
            ));
        }
        if namespace == self.header.authoring_namespace_id.as_ref() {
            return Err(DiagnosticBundle::single(Diagnostic::import_cycle(
                &[namespace],
                Box::new([span]),
            )));
        }
        if let Some(existing_index) = self.import_index.get(namespace).copied() {
            return Err(DiagnosticBundle::single(Diagnostic::duplicate_import(
                namespace,
                span,
                self.imports[existing_index].span.clone(),
            )));
        }

        let observed_imports = u64::try_from(self.imports.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        if let Some(diagnostic) = limit_diagnostic(
            &self.limits,
            CompileLimitDimension::ImportEdgeCount,
            observed_imports,
            Some(span.clone()),
            Some(namespace.into()),
        ) {
            return Err(DiagnosticBundle::single(diagnostic));
        }
        let namespace_bytes = u64::try_from(namespace.len()).unwrap_or(u64::MAX);
        let observed_string_items = self.string_item_count.saturating_add(1);
        let observed_string_bytes = self.string_bytes.saturating_add(namespace_bytes);
        let observed_controlled_string_bytes =
            self.controlled_string_bytes.saturating_add(namespace_bytes);
        let observed_source_bytes = self
            .source_record_byte_len
            .checked_add(4 + 16)
            .and_then(|length| length.checked_add(namespace_bytes))
            .unwrap_or(u64::MAX);
        for (dimension, observed) in [
            (
                CompileLimitDimension::StringItemCount,
                observed_string_items,
            ),
            (
                CompileLimitDimension::TotalStringBytes,
                observed_string_bytes,
            ),
            (
                CompileLimitDimension::SourceBytesPerModule,
                observed_source_bytes,
            ),
            (
                CompileLimitDimension::CompilerControlledLiveBytes,
                observed_controlled_string_bytes.saturating_add(observed_source_bytes),
            ),
        ] {
            if let Some(diagnostic) = limit_diagnostic(
                &self.limits,
                dimension,
                observed,
                Some(span.clone()),
                Some(namespace.into()),
            ) {
                return Err(DiagnosticBundle::single(diagnostic));
            }
        }

        let namespace: Arc<str> = namespace.into();
        self.imports.push(ImportRecord {
            namespace: Arc::clone(&namespace),
            span,
        });
        self.import_index.insert(namespace, self.imports.len() - 1);
        self.string_item_count = observed_string_items;
        self.string_bytes = observed_string_bytes;
        self.controlled_string_bytes = observed_controlled_string_bytes;
        self.source_record_byte_len = observed_source_bytes;

        Ok(self)
    }

    /// 原子派生来源记录、SHA-256 内容摘要与不可配错的模块描述符。
    pub fn finish(self) -> Result<SyntheticModule, DiagnosticBundle> {
        let source_record = encode_source_record(
            &self.header,
            &self.imports,
            self.limits
                .value(CompileLimitDimension::SourceBytesPerModule),
        )?;
        let source_record_byte_len = u32::try_from(source_record.len()).map_err(|_| {
            DiagnosticBundle::single(Diagnostic::compile_limit_exceeded(
                CompileLimitDimension::SourceBytesPerModule,
                self.limits
                    .value(CompileLimitDimension::SourceBytesPerModule),
                u64::try_from(source_record.len()).unwrap_or(u64::MAX),
            ))
        })?;
        let source_content_digest: [u8; 32] = Sha256::digest(&source_record).into();

        let mut canonical_imports: Vec<_> = self
            .imports
            .iter()
            .map(|record| Arc::clone(&record.namespace))
            .collect();
        canonical_imports.sort_unstable();

        let descriptor = SourceModuleDescriptor {
            authoring_namespace_id: self.header.authoring_namespace_id,
            source_language: SourceLanguage::SyntheticDsl,
            source_content_digest,
            source_record_byte_len,
            frontend_version: SYNTHETIC_FRONTEND_VERSION,
            frontend_options_digest: self.header.frontend_options_digest,
            generator_build_id: self.header.generator_build_id,
            parameters_and_inputs_digest: self.header.parameters_and_inputs_digest,
            random_seed: self.header.random_seed,
            provenance: self.header.provenance,
            source_document_key: self.header.source_document_key,
            imports: canonical_imports.into_boxed_slice(),
            declaration_span: self.header.declaration_span,
        };

        Ok(SyntheticModule {
            descriptor,
            source_record: source_record.into_boxed_slice(),
            imports: self.imports.into_boxed_slice(),
            string_item_count: self.string_item_count,
            string_bytes: self.string_bytes,
            controlled_live_bytes: self
                .controlled_string_bytes
                .saturating_add(self.source_record_byte_len),
        })
    }
}

/// 官方合成来源与其派生描述符的不可分封装。
pub struct SyntheticModule {
    descriptor: SourceModuleDescriptor,
    source_record: Box<[u8]>,
    imports: Box<[ImportRecord]>,
    string_item_count: u64,
    string_bytes: u64,
    controlled_live_bytes: u64,
}

impl SyntheticModule {
    #[must_use]
    pub const fn descriptor(&self) -> &SourceModuleDescriptor {
        &self.descriptor
    }

    fn source_record(&self) -> &[u8] {
        &self.source_record
    }
}

/// 只接受 #292 官方来源模块的编译单元构建器。
pub struct CompilationUnitBuilder {
    limits: CompileLimits,
    modules: Vec<SyntheticModule>,
    module_index: HashMap<Arc<str>, usize>,
    source_bytes_total: u64,
    import_edge_count: u64,
    string_item_count: u64,
    string_bytes: u64,
    controlled_live_bytes: u64,
}

impl CompilationUnitBuilder {
    #[must_use]
    pub fn new(limits: CompileLimits) -> Self {
        Self {
            limits,
            modules: Vec::new(),
            module_index: HashMap::new(),
            source_bytes_total: 0,
            import_edge_count: 0,
            string_item_count: 0,
            string_bytes: 0,
            controlled_live_bytes: 0,
        }
    }

    pub fn add_synthetic_module(
        &mut self,
        module: SyntheticModule,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let namespace = module.descriptor.authoring_namespace_id.as_ref();
        if let Some(existing_index) = self.module_index.get(namespace).copied() {
            return Err(DiagnosticBundle::single(
                Diagnostic::duplicate_module_namespace(
                    namespace,
                    module.descriptor.declaration_span.clone(),
                    self.modules[existing_index]
                        .descriptor
                        .declaration_span
                        .clone(),
                ),
            ));
        }

        let next_module_count = u64::try_from(self.modules.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let next_source_bytes = self
            .source_bytes_total
            .saturating_add(u64::try_from(module.source_record().len()).unwrap_or(u64::MAX));
        let next_import_edges = self
            .import_edge_count
            .saturating_add(u64::try_from(module.imports.len()).unwrap_or(u64::MAX));
        let next_string_items = self
            .string_item_count
            .saturating_add(module.string_item_count);
        let next_string_bytes = self.string_bytes.saturating_add(module.string_bytes);
        let next_controlled_live_bytes = self
            .controlled_live_bytes
            .saturating_add(module.controlled_live_bytes);
        for (dimension, observed) in [
            (CompileLimitDimension::ModuleCount, next_module_count),
            (CompileLimitDimension::SourceBytesTotal, next_source_bytes),
            (CompileLimitDimension::ImportEdgeCount, next_import_edges),
            (CompileLimitDimension::StringItemCount, next_string_items),
            (CompileLimitDimension::TotalStringBytes, next_string_bytes),
            (
                CompileLimitDimension::CompilerControlledLiveBytes,
                next_controlled_live_bytes,
            ),
        ] {
            if let Some(diagnostic) = limit_diagnostic(
                &self.limits,
                dimension,
                observed,
                Some(module.descriptor.declaration_span.clone()),
                Some(namespace.into()),
            ) {
                return Err(DiagnosticBundle::single(diagnostic));
            }
        }

        let namespace = Arc::clone(&module.descriptor.authoring_namespace_id);
        self.modules.push(module);
        self.module_index.insert(namespace, self.modules.len() - 1);
        self.source_bytes_total = next_source_bytes;
        self.import_edge_count = next_import_edges;
        self.string_item_count = next_string_items;
        self.string_bytes = next_string_bytes;
        self.controlled_live_bytes = next_controlled_live_bytes;
        Ok(self)
    }

    pub fn build(self) -> Result<CompilationUnit, DiagnosticBundle> {
        let mut diagnostics =
            DiagnosticCollector::new(self.limits.value(CompileLimitDimension::DiagnosticCount));
        let mut canonical_indices: Vec<_> = (0..self.modules.len()).collect();
        canonical_indices.sort_unstable_by(|left, right| {
            self.modules[*left]
                .descriptor
                .authoring_namespace_id
                .cmp(&self.modules[*right].descriptor.authoring_namespace_id)
        });
        for (order, module_index) in canonical_indices.iter().copied().enumerate() {
            let module = &self.modules[module_index];
            for import in &module.imports {
                if !self.module_index.contains_key(import.namespace.as_ref()) {
                    let mut diagnostic =
                        Diagnostic::unknown_import(&import.namespace, import.span.clone());
                    diagnostic.set_canonical_module_order(u32::try_from(order).unwrap_or(u32::MAX));
                    diagnostics.push(diagnostic);
                }
            }
        }
        if !diagnostics.is_empty() {
            return Err(diagnostics.finish());
        }

        let order = match canonical_topological_order(&self.modules, &self.module_index) {
            Ok(order) => order,
            Err(cycles) => {
                let mut canonical_order_by_index = vec![0_u32; self.modules.len()];
                for (order, module_index) in canonical_indices.iter().copied().enumerate() {
                    canonical_order_by_index[module_index] =
                        u32::try_from(order).unwrap_or(u32::MAX);
                }
                for cycle in cycles {
                    let namespaces: Vec<_> = cycle
                        .iter()
                        .map(|index| {
                            self.modules[*index]
                                .descriptor
                                .authoring_namespace_id
                                .as_ref()
                        })
                        .collect();
                    let spans: Vec<_> = cycle
                        .iter()
                        .enumerate()
                        .filter_map(|(position, module_index)| {
                            let next_index = cycle[(position + 1) % cycle.len()];
                            let next_namespace = self.modules[next_index]
                                .descriptor
                                .authoring_namespace_id
                                .as_ref();
                            self.modules[*module_index]
                                .imports
                                .iter()
                                .find(|import| import.namespace.as_ref() == next_namespace)
                                .map(|import| import.span.clone())
                        })
                        .collect();
                    let mut diagnostic =
                        Diagnostic::import_cycle(&namespaces, spans.into_boxed_slice());
                    if let Some(first_index) = cycle.first().copied() {
                        diagnostic
                            .set_canonical_module_order(canonical_order_by_index[first_index]);
                    }
                    diagnostics.push(diagnostic);
                }
                return Err(diagnostics.finish());
            }
        };

        let mut canonical_rank = vec![0_usize; order.len()];
        for (rank, index) in order.into_iter().enumerate() {
            canonical_rank[index] = rank;
        }
        let mut modules: Vec<_> = self.modules.into_iter().enumerate().collect();
        modules.sort_unstable_by_key(|(original_index, _)| canonical_rank[*original_index]);
        let modules = modules
            .into_iter()
            .map(|(_, module)| module)
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ok(CompilationUnit {
            limits: self.limits,
            modules,
        })
    }
}

/// 规范模块顺序已冻结的原子编译输入。
pub struct CompilationUnit {
    // 后继编译遍使用同一份不可变配置；来源图切片建立时暂不读取该字段。
    #[allow(dead_code)]
    pub(crate) limits: CompileLimits,
    pub(crate) modules: Box<[SyntheticModule]>,
}

impl CompilationUnit {
    #[must_use]
    pub const fn module_count(&self) -> usize {
        self.modules.len()
    }

    pub fn module_descriptors(&self) -> impl ExactSizeIterator<Item = &SourceModuleDescriptor> {
        self.modules.iter().map(|module| &module.descriptor)
    }
}

fn header_resident_string_bytes(header: &SourceModuleHeader) -> u64 {
    [
        header.authoring_namespace_id.len(),
        header.source_document_key.len(),
    ]
    .into_iter()
    .try_fold(0_u64, |total, value| {
        total.checked_add(u64::try_from(value).ok()?)
    })
    .unwrap_or(u64::MAX)
}

fn header_controlled_string_bytes(header: &SourceModuleHeader) -> u64 {
    [
        header.authoring_namespace_id.len(),
        header.source_document_key.len(),
        header.generator_build_id.len(),
        header.provenance.len(),
    ]
    .into_iter()
    .try_fold(0_u64, |total, value| {
        total.checked_add(u64::try_from(value).ok()?)
    })
    .unwrap_or(u64::MAX)
}

fn limit_diagnostic(
    limits: &CompileLimits,
    dimension: CompileLimitDimension,
    observed: u64,
    primary_span: Option<SourceSpan>,
    stable_key: Option<Box<str>>,
) -> Option<Diagnostic> {
    let limit = limits.value(dimension);
    (observed > limit).then(|| {
        Diagnostic::compile_limit_exceeded_at(dimension, limit, observed, primary_span, stable_key)
    })
}

fn push_limit_if_exceeded(
    diagnostics: &mut DiagnosticCollector,
    limits: &CompileLimits,
    dimension: CompileLimitDimension,
    observed: u64,
    primary_span: Option<SourceSpan>,
    stable_key: Option<Box<str>>,
) {
    if let Some(diagnostic) =
        limit_diagnostic(limits, dimension, observed, primary_span, stable_key)
    {
        diagnostics.push(diagnostic);
    }
}

fn encoded_source_record_len(header: &SourceModuleHeader, imports: &[ImportRecord]) -> Option<u64> {
    let mut length = u64::try_from(SOURCE_RECORD_MAGIC.len()).ok()?;
    length = length.checked_add(4 + 2)?;
    for value in [
        header.authoring_namespace_id.as_ref(),
        header.source_document_key.as_ref(),
        header.generator_build_id.as_ref(),
        header.provenance.as_ref(),
    ] {
        length = length.checked_add(4)?;
        length = length.checked_add(u64::try_from(value.len()).ok()?)?;
    }
    length = length.checked_add(32 + 32 + 1 + 8 + 16 + 4)?;
    for import in imports {
        length = length.checked_add(4)?;
        length = length.checked_add(u64::try_from(import.namespace.len()).ok()?)?;
        length = length.checked_add(16)?;
    }
    length.checked_add(4)
}

fn encode_source_record(
    header: &SourceModuleHeader,
    imports: &[ImportRecord],
    source_bytes_per_module_limit: u64,
) -> Result<Vec<u8>, DiagnosticBundle> {
    let expected_len = encoded_source_record_len(header, imports).unwrap_or(u64::MAX);
    let limit = source_bytes_per_module_limit.min(u64::from(u32::MAX));
    if expected_len > limit {
        return Err(DiagnosticBundle::single(
            Diagnostic::compile_limit_exceeded(
                CompileLimitDimension::SourceBytesPerModule,
                limit,
                expected_len,
            ),
        ));
    }
    let capacity = usize::try_from(expected_len).map_err(|_| {
        DiagnosticBundle::single(Diagnostic::compile_limit_exceeded(
            CompileLimitDimension::SourceBytesPerModule,
            limit,
            expected_len,
        ))
    })?;
    let mut bytes = Vec::with_capacity(capacity);
    bytes.extend_from_slice(&SOURCE_RECORD_MAGIC);
    bytes.extend_from_slice(&SYNTHETIC_FRONTEND_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(SourceLanguage::SyntheticDsl as u16).to_le_bytes());
    put_bytes(&mut bytes, &header.authoring_namespace_id);
    put_bytes(&mut bytes, &header.source_document_key);
    put_bytes(&mut bytes, &header.generator_build_id);
    bytes.extend_from_slice(&header.parameters_and_inputs_digest);
    bytes.extend_from_slice(&header.frontend_options_digest);
    bytes.push(u8::from(header.random_seed.is_some()));
    bytes.extend_from_slice(&header.random_seed.unwrap_or(0).to_le_bytes());
    put_bytes(&mut bytes, &header.provenance);
    put_span(&mut bytes, &header.declaration_span);
    bytes.extend_from_slice(
        &u32::try_from(imports.len())
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    for import in imports {
        put_bytes(&mut bytes, &import.namespace);
        put_span(&mut bytes, &import.span);
    }
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    debug_assert_eq!(bytes.len(), capacity);
    Ok(bytes)
}

fn put_bytes(output: &mut Vec<u8>, value: &str) {
    output.extend_from_slice(&u32::try_from(value.len()).unwrap_or(u32::MAX).to_le_bytes());
    output.extend_from_slice(value.as_bytes());
}

fn put_span(output: &mut Vec<u8>, span: &SourceSpan) {
    output.extend_from_slice(&span.start().line().to_le_bytes());
    output.extend_from_slice(&span.start().column().to_le_bytes());
    output.extend_from_slice(&span.end().line().to_le_bytes());
    output.extend_from_slice(&span.end().column().to_le_bytes());
}

fn canonical_topological_order(
    modules: &[SyntheticModule],
    module_index: &HashMap<Arc<str>, usize>,
) -> Result<Vec<usize>, Vec<Vec<usize>>> {
    let mut indegree = vec![0_usize; modules.len()];
    let mut dependents = vec![Vec::new(); modules.len()];
    for (index, module) in modules.iter().enumerate() {
        indegree[index] = module.imports.len();
        for import in &module.imports {
            let dependency_index = module_index[import.namespace.as_ref()];
            dependents[dependency_index].push(index);
        }
    }
    for entries in &mut dependents {
        entries.sort_unstable_by(|left, right| {
            modules[*left]
                .descriptor
                .authoring_namespace_id
                .cmp(&modules[*right].descriptor.authoring_namespace_id)
        });
    }

    let mut ready = BTreeSet::new();
    for (index, degree) in indegree.iter().copied().enumerate() {
        if degree == 0 {
            ready.insert((
                Arc::clone(&modules[index].descriptor.authoring_namespace_id),
                index,
            ));
        }
    }
    let mut order = Vec::with_capacity(modules.len());
    while let Some((_, index)) = ready.pop_first() {
        order.push(index);
        for dependent in dependents[index].iter().copied() {
            indegree[dependent] -= 1;
            if indegree[dependent] == 0 {
                ready.insert((
                    Arc::clone(&modules[dependent].descriptor.authoring_namespace_id),
                    dependent,
                ));
            }
        }
    }

    if order.len() == modules.len() {
        Ok(order)
    } else {
        Err(find_canonical_cycles(modules, module_index))
    }
}

fn sorted_dependencies(
    index: usize,
    modules: &[SyntheticModule],
    module_index: &HashMap<Arc<str>, usize>,
) -> Vec<usize> {
    let mut dependencies: Vec<_> = modules[index]
        .imports
        .iter()
        .map(|import| module_index[import.namespace.as_ref()])
        .collect();
    dependencies.sort_unstable_by(|left, right| {
        modules[*left]
            .descriptor
            .authoring_namespace_id
            .cmp(&modules[*right].descriptor.authoring_namespace_id)
    });
    dependencies
}

fn find_canonical_cycles(
    modules: &[SyntheticModule],
    module_index: &HashMap<Arc<str>, usize>,
) -> Vec<Vec<usize>> {
    struct Tarjan<'a> {
        modules: &'a [SyntheticModule],
        module_index: &'a HashMap<Arc<str>, usize>,
        next_index: usize,
        indices: Vec<Option<usize>>,
        low_links: Vec<usize>,
        stack: Vec<usize>,
        on_stack: Vec<bool>,
        components: Vec<Vec<usize>>,
    }

    impl Tarjan<'_> {
        fn visit(&mut self, index: usize) {
            let discovery_index = self.next_index;
            self.next_index += 1;
            self.indices[index] = Some(discovery_index);
            self.low_links[index] = discovery_index;
            self.stack.push(index);
            self.on_stack[index] = true;

            for dependency in sorted_dependencies(index, self.modules, self.module_index) {
                if self.indices[dependency].is_none() {
                    self.visit(dependency);
                    self.low_links[index] = self.low_links[index].min(self.low_links[dependency]);
                } else if self.on_stack[dependency]
                    && let Some(dependency_index) = self.indices[dependency]
                {
                    self.low_links[index] = self.low_links[index].min(dependency_index);
                }
            }

            if self.low_links[index] != discovery_index {
                return;
            }

            let mut component = Vec::new();
            while let Some(member) = self.stack.pop() {
                self.on_stack[member] = false;
                component.push(member);
                if member == index {
                    break;
                }
            }
            component.sort_unstable_by(|left, right| {
                self.modules[*left]
                    .descriptor
                    .authoring_namespace_id
                    .cmp(&self.modules[*right].descriptor.authoring_namespace_id)
            });
            self.components.push(component);
        }
    }

    let mut canonical_indices: Vec<_> = (0..modules.len()).collect();
    canonical_indices.sort_unstable_by(|left, right| {
        modules[*left]
            .descriptor
            .authoring_namespace_id
            .cmp(&modules[*right].descriptor.authoring_namespace_id)
    });
    let mut tarjan = Tarjan {
        modules,
        module_index,
        next_index: 0,
        indices: vec![None; modules.len()],
        low_links: vec![0; modules.len()],
        stack: Vec::new(),
        on_stack: vec![false; modules.len()],
        components: Vec::new(),
    };
    for index in canonical_indices {
        if tarjan.indices[index].is_none() {
            tarjan.visit(index);
        }
    }

    let mut cycles: Vec<_> = tarjan
        .components
        .into_iter()
        .filter(|component| {
            component.len() > 1
                || component.first().is_some_and(|index| {
                    modules[*index]
                        .imports
                        .iter()
                        .any(|import| module_index[import.namespace.as_ref()] == *index)
                })
        })
        .map(|component| canonical_cycle_for_component(&component, modules, module_index))
        .collect();
    cycles.sort_unstable_by(|left, right| {
        let left_namespace = left
            .first()
            .map(|index| &modules[*index].descriptor.authoring_namespace_id);
        let right_namespace = right
            .first()
            .map(|index| &modules[*index].descriptor.authoring_namespace_id);
        left_namespace.cmp(&right_namespace)
    });
    cycles
}

fn canonical_cycle_for_component(
    component: &[usize],
    modules: &[SyntheticModule],
    module_index: &HashMap<Arc<str>, usize>,
) -> Vec<usize> {
    fn find_path(
        index: usize,
        target: usize,
        allowed: &[bool],
        modules: &[SyntheticModule],
        module_index: &HashMap<Arc<str>, usize>,
        visited: &mut [bool],
    ) -> Option<Vec<usize>> {
        if index == target {
            return Some(vec![target]);
        }
        if visited[index] {
            return None;
        }
        visited[index] = true;
        for dependency in sorted_dependencies(index, modules, module_index) {
            if allowed[dependency]
                && let Some(mut suffix) =
                    find_path(dependency, target, allowed, modules, module_index, visited)
            {
                let mut path = Vec::with_capacity(suffix.len() + 1);
                path.push(index);
                path.append(&mut suffix);
                return Some(path);
            }
        }
        None
    }

    let Some(start) = component.first().copied() else {
        return Vec::new();
    };
    let mut allowed = vec![false; modules.len()];
    for index in component.iter().copied() {
        allowed[index] = true;
    }
    for dependency in sorted_dependencies(start, modules, module_index) {
        if !allowed[dependency] {
            continue;
        }
        if dependency == start {
            return vec![start];
        }
        let mut visited = vec![false; modules.len()];
        if let Some(mut path) = find_path(
            dependency,
            start,
            &allowed,
            modules,
            module_index,
            &mut visited,
        ) {
            path.pop();
            let mut cycle = Vec::with_capacity(path.len() + 1);
            cycle.push(start);
            cycle.append(&mut path);
            return cycle;
        }
    }
    component.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DiagnosticCode, DiagnosticPayload, SourceModuleHeaderInput};

    fn header(namespace: &str, document: &str) -> SourceModuleHeader {
        SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: namespace,
                source_document_key: document,
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

    fn module(namespace: &str, imports: &[&str]) -> SyntheticModule {
        let limits = CompileLimits::p100_initial_v1();
        let mut builder =
            SyntheticModuleBuilder::new(header(namespace, namespace), &limits).unwrap();
        for import in imports {
            builder.add_import(import).unwrap();
        }
        builder.finish().unwrap()
    }

    fn expect_diagnostics<T>(result: Result<T, DiagnosticBundle>) -> DiagnosticBundle {
        match result {
            Ok(_) => panic!("expected structured diagnostics"),
            Err(bundle) => bundle,
        }
    }

    #[test]
    fn descriptor_digest_is_derived_from_exact_source_record() {
        let module = module("city/a", &["city/base"]);
        assert_eq!(
            module.descriptor().source_language(),
            SourceLanguage::SyntheticDsl
        );
        assert_eq!(module.descriptor().frontend_version(), 1);
        assert_eq!(
            usize::try_from(module.descriptor().source_record_byte_len()).unwrap(),
            module.source_record.len()
        );
        let digest: [u8; 32] = Sha256::digest(&module.source_record).into();
        assert_eq!(module.descriptor().source_content_digest(), &digest);
        assert_eq!(
            module.descriptor().imports().collect::<Vec<_>>(),
            ["city/base"]
        );
    }

    #[test]
    fn empty_module_source_record_has_a_position_independent_known_vector() {
        let source_document_key = Arc::from("source.known-vector");
        let fixed_header = SourceModuleHeader {
            authoring_namespace_id: Arc::from("city/known-vector"),
            source_document_key: Arc::clone(&source_document_key),
            generator_build_id: Arc::from("git:0123456789abcdef"),
            parameters_and_inputs_digest: [0x11; 32],
            frontend_options_digest: [0x22; 32],
            random_seed: Some(42),
            provenance: Arc::from("repository:laneflow"),
            declaration_span: SourceSpan::point(source_document_key, 7, 11),
        };
        let module = SyntheticModuleBuilder::new(fixed_header, &CompileLimits::p100_initial_v1())
            .unwrap()
            .finish()
            .unwrap();

        assert_eq!(module.descriptor.source_record_byte_len, 202);
        assert_eq!(
            module.descriptor.source_content_digest,
            [
                0x30, 0x24, 0xae, 0x9e, 0xb4, 0xa2, 0xcd, 0x16, 0x59, 0x3b, 0xd9, 0x7a, 0xf3, 0xbe,
                0x54, 0xcb, 0x06, 0x61, 0x8d, 0xce, 0x2e, 0x24, 0x3a, 0xc7, 0xc1, 0xb2, 0x3a, 0x12,
                0xef, 0x02, 0xa3, 0x4a,
            ]
        );
    }

    #[test]
    fn duplicate_and_self_imports_fail_with_source_context() {
        let limits = CompileLimits::p100_initial_v1();
        let mut builder =
            SyntheticModuleBuilder::new(header("city/a", "source.a"), &limits).unwrap();
        builder.add_import("city/base").unwrap();
        let duplicate = expect_diagnostics(builder.add_import("city/base"));
        assert_eq!(
            duplicate.diagnostics()[0].code(),
            DiagnosticCode::DuplicateImport
        );
        assert!(duplicate.diagnostics()[0].primary_span().is_some());
        assert_eq!(duplicate.diagnostics()[0].related_spans().len(), 1);

        let self_import = expect_diagnostics(builder.add_import("city/a"));
        assert_eq!(
            self_import.diagnostics()[0].code(),
            DiagnosticCode::ImportCycle
        );
    }

    #[test]
    fn import_limit_failure_does_not_mutate_the_module() {
        let limits = CompileLimits::p100_initial_v1();
        let mut builder =
            SyntheticModuleBuilder::new(header("city/a", "source.a"), &limits).unwrap();
        let import_limit = limits.value(CompileLimitDimension::ImportEdgeCount);
        for index in 0..import_limit {
            builder
                .add_import(&format!("city/import/{index:04}"))
                .unwrap();
        }

        let failure = expect_diagnostics(builder.add_import("city/import/overflow"));
        assert!(matches!(
            failure.diagnostics()[0].payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::ImportEdgeCount,
                limit,
                observed,
            } if *limit == import_limit && *observed == import_limit + 1
        ));
        let module = builder.finish().unwrap();
        assert_eq!(
            u64::try_from(module.descriptor().imports().len()).unwrap(),
            import_limit
        );
    }

    #[test]
    fn source_record_encoder_fails_before_over_limit_allocation() {
        let header = header("city/a", "source.a");
        let expected_len = encoded_source_record_len(&header, &[]).unwrap();
        let failure = expect_diagnostics(encode_source_record(&header, &[], expected_len - 1));
        assert!(matches!(
            failure.diagnostics()[0].payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::SourceBytesPerModule,
                limit,
                observed,
            } if *limit == expected_len - 1 && *observed == expected_len
        ));
    }

    #[test]
    fn compilation_unit_rejects_unknown_imports_and_duplicate_namespaces() {
        let limits = CompileLimits::p100_initial_v1();
        let mut builder = CompilationUnitBuilder::new(limits.clone());
        builder
            .add_synthetic_module(module("city/a", &["city/missing"]))
            .unwrap();
        let unknown = expect_diagnostics(builder.build());
        assert!(matches!(
            unknown.diagnostics()[0].payload(),
            DiagnosticPayload::UnknownImport { namespace } if namespace.as_ref() == "city/missing"
        ));

        let mut builder = CompilationUnitBuilder::new(limits);
        builder.add_synthetic_module(module("city/a", &[])).unwrap();
        let duplicate = expect_diagnostics(builder.add_synthetic_module(module("city/a", &[])));
        assert_eq!(
            duplicate.diagnostics()[0].code(),
            DiagnosticCode::DuplicateModuleNamespace
        );
        assert_eq!(duplicate.diagnostics()[0].related_spans().len(), 1);
    }

    #[test]
    fn compilation_unit_uses_dependency_first_canonical_order() {
        let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
        builder
            .add_synthetic_module(module("city/z", &["city/a"]))
            .unwrap();
        builder.add_synthetic_module(module("city/b", &[])).unwrap();
        builder.add_synthetic_module(module("city/a", &[])).unwrap();
        let unit = builder.build().unwrap();
        assert_eq!(
            unit.module_descriptors()
                .map(SourceModuleDescriptor::authoring_namespace_id)
                .collect::<Vec<_>>(),
            ["city/a", "city/b", "city/z"]
        );
    }

    #[test]
    fn compilation_unit_reports_canonical_cycle() {
        let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
        builder
            .add_synthetic_module(module("city/c", &["city/a"]))
            .unwrap();
        builder
            .add_synthetic_module(module("city/a", &["city/b"]))
            .unwrap();
        builder
            .add_synthetic_module(module("city/b", &["city/c"]))
            .unwrap();
        let bundle = expect_diagnostics(builder.build());
        assert!(matches!(
            bundle.diagnostics()[0].payload(),
            DiagnosticPayload::ImportCycle { namespaces }
                if namespaces.iter().map(AsRef::as_ref).collect::<Vec<&str>>()
                    == ["city/a", "city/b", "city/c"]
        ));
    }

    #[test]
    fn compilation_unit_reports_every_disjoint_cycle_in_canonical_order() {
        let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
        for module in [
            module("city/d", &["city/c"]),
            module("city/a", &["city/b"]),
            module("city/c", &["city/d"]),
            module("city/b", &["city/a"]),
        ] {
            builder.add_synthetic_module(module).unwrap();
        }

        let bundle = expect_diagnostics(builder.build());
        assert_eq!(bundle.diagnostics().len(), 2);
        let cycles: Vec<_> = bundle
            .diagnostics()
            .iter()
            .map(|diagnostic| match diagnostic.payload() {
                DiagnosticPayload::ImportCycle { namespaces } => {
                    namespaces.iter().map(AsRef::as_ref).collect::<Vec<&str>>()
                }
                other => panic!("expected import cycle, got {other:?}"),
            })
            .collect();
        assert_eq!(cycles, [["city/a", "city/b"], ["city/c", "city/d"]]);
        assert!(
            bundle
                .diagnostics()
                .iter()
                .all(|diagnostic| diagnostic.primary_span().is_some())
        );
    }
}
