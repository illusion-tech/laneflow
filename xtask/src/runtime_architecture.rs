//! Runtime 模块边界与 Runtime / Spatial / Adapter 的生产依赖方向。
//!
//! 按 Rust 模块树解析语法，保守覆盖所有非 test 配置。解析失败、缺失模块、动态
//! include 或无法解析的通配导入均报错；不能用一次成功 grep 代替失败关闭的检查。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use proc_macro2::{TokenStream, TokenTree};
use syn::parse::Parser;
use syn::visit::{self, Visit};
use syn::{Attribute, Item, Meta, Token, UseTree};

type ModulePath = Vec<String>;
type Imports = BTreeMap<ModulePath, ModulePath>;

const RUNTIME: &str = "laneflow-runtime";
const SPATIAL: &str = "laneflow-spatial";
const ADAPTER: &str = "laneflow-bevy";
const FORMAT: &str = "laneflow-format";
const WIRE: &str = "laneflow-runtime-snapshot-wire";

pub(crate) fn run() -> Result<(), String> {
    let root = std::env::current_dir().map_err(|error| error.to_string())?;
    let output = Command::new("cargo")
        .args(["metadata", "--locked", "--no-deps", "--format-version", "1"])
        .current_dir(&root)
        .output()
        .map_err(|error| format!("无法读取架构依赖图: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "架构依赖图读取失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("架构依赖图不是有效 JSON: {error}"))?;
    let packages = metadata["packages"]
        .as_array()
        .ok_or("架构依赖图缺少 packages")?;
    let mut graph = BTreeMap::new();
    let mut format_names = BTreeSet::from([
        "laneflow_format".to_string(),
        "laneflow_runtime_snapshot_wire".to_string(),
    ]);
    for package in packages {
        let name = package["name"].as_str().ok_or("package 缺少 name")?;
        let dependencies = package["dependencies"]
            .as_array()
            .ok_or("package 缺少 dependencies")?;
        let mut edges = Vec::new();
        for dependency in dependencies {
            // cargo metadata 包含 optional / target-specific 声明；逐条检查生产和构建边。
            if dependency["kind"].as_str() == Some("dev") {
                continue;
            }
            let target = dependency["name"].as_str().ok_or("依赖缺少 name")?;
            edges.push(target.to_string());
            if name == RUNTIME && matches!(target, FORMAT | WIRE) {
                format_names.insert(
                    dependency["rename"]
                        .as_str()
                        .unwrap_or(target)
                        .replace('-', "_"),
                );
            }
        }
        graph.insert(name.to_string(), edges);
    }
    check_dependency_graph(&graph)?;
    check_sources(&root.join("crates/laneflow-runtime/src"), &format_names)?;
    println!("Runtime 架构检查通过：格式入口、kernel 与生产依赖方向闭合。");
    Ok(())
}

fn check_dependency_graph(graph: &BTreeMap<String, Vec<String>>) -> Result<(), String> {
    for required in [RUNTIME, SPATIAL, ADAPTER] {
        if !graph.contains_key(required) {
            return Err(format!("架构依赖图缺少必需包 {required}"));
        }
    }
    for source in [RUNTIME, SPATIAL] {
        let mut pending = vec![(source.to_string(), vec![source.to_string()])];
        let mut seen = BTreeSet::new();
        while let Some((node, chain)) = pending.pop() {
            if !seen.insert(node.clone()) {
                continue;
            }
            for target in graph.get(&node).into_iter().flatten() {
                let forbidden = target == ADAPTER
                    || target == "laneflow-compiler"
                    || target == "bevy"
                    || target.starts_with("bevy_")
                    || (source == RUNTIME && target == SPATIAL)
                    || (source == SPATIAL && target == RUNTIME);
                let mut next = chain.clone();
                next.push(target.clone());
                if forbidden {
                    return Err(format!("生产依赖方向违规: {}", next.join(" -> ")));
                }
                pending.push((target.clone(), next));
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Truth {
    Yes,
    No,
    Unknown,
}

fn non_test_cfg(meta: &Meta) -> Truth {
    match meta {
        Meta::Path(path) if path.is_ident("test") => Truth::No,
        Meta::List(list) => {
            let Ok(parts) = syn::punctuated::Punctuated::<Meta, Token![,]>::parse_terminated
                .parse2(list.tokens.clone())
            else {
                return Truth::Unknown;
            };
            let values: Vec<_> = parts.iter().map(non_test_cfg).collect();
            if list.path.is_ident("not") && values.len() == 1 {
                match values[0] {
                    Truth::Yes => Truth::No,
                    Truth::No => Truth::Yes,
                    Truth::Unknown => Truth::Unknown,
                }
            } else if list.path.is_ident("all") {
                if values.iter().any(|value| matches!(value, Truth::No)) {
                    Truth::No
                } else if values.iter().all(|value| matches!(value, Truth::Yes)) {
                    Truth::Yes
                } else {
                    Truth::Unknown
                }
            } else if list.path.is_ident("any") {
                if values.iter().any(|value| matches!(value, Truth::Yes)) {
                    Truth::Yes
                } else if values.iter().all(|value| matches!(value, Truth::No)) {
                    Truth::No
                } else {
                    Truth::Unknown
                }
            } else {
                Truth::Unknown
            }
        }
        _ => Truth::Unknown,
    }
}

fn test_only(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && attribute
                .parse_args::<Meta>()
                .is_ok_and(|meta| matches!(non_test_cfg(&meta), Truth::No))
    })
}

fn attributes(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(item) => &item.attrs,
        Item::Enum(item) => &item.attrs,
        Item::ExternCrate(item) => &item.attrs,
        Item::Fn(item) => &item.attrs,
        Item::ForeignMod(item) => &item.attrs,
        Item::Impl(item) => &item.attrs,
        Item::Macro(item) => &item.attrs,
        Item::Mod(item) => &item.attrs,
        Item::Static(item) => &item.attrs,
        Item::Struct(item) => &item.attrs,
        Item::Trait(item) => &item.attrs,
        Item::TraitAlias(item) => &item.attrs,
        Item::Type(item) => &item.attrs,
        Item::Union(item) => &item.attrs,
        Item::Use(item) => &item.attrs,
        _ => &[],
    }
}

fn use_paths(
    tree: &UseTree,
    prefix: &mut ModulePath,
    output: &mut Vec<(String, ModulePath)>,
) -> Result<(), String> {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            use_paths(&path.tree, prefix, output)?;
            prefix.pop();
        }
        UseTree::Name(name) => {
            let mut target = prefix.clone();
            if name.ident != "self" {
                target.push(name.ident.to_string());
            }
            let alias = target.last().ok_or("空 use 路径")?.clone();
            output.push((alias, target));
        }
        UseTree::Rename(rename) => {
            let mut target = prefix.clone();
            if rename.ident != "self" {
                target.push(rename.ident.to_string());
            }
            output.push((rename.rename.to_string(), target));
        }
        UseTree::Group(group) => {
            for item in &group.items {
                use_paths(item, prefix, output)?;
            }
        }
        UseTree::Glob(_) => {
            return Err("生产模块的通配导入无法证明边界；请使用显式名称".to_string());
        }
    }
    Ok(())
}

fn absolute(module: &[String], path: &[String]) -> ModulePath {
    let mut prefix = module.to_vec();
    let mut offset = 0;
    match path.first().map(String::as_str) {
        Some("crate") => {
            prefix.clear();
            offset = 1;
        }
        Some("self") => {
            offset = 1;
        }
        Some("super") => {
            while path.get(offset).is_some_and(|part| part == "super") {
                prefix.pop();
                offset += 1;
            }
        }
        _ => {}
    }
    prefix.extend_from_slice(&path[offset..]);
    prefix
}

struct SourceModule {
    name: ModulePath,
    file: PathBuf,
    items: Vec<Item>,
}

fn load_module(
    root: &Path,
    file: &Path,
    name: ModulePath,
    modules: &mut Vec<SourceModule>,
) -> Result<(), String> {
    let canonical = file
        .canonicalize()
        .map_err(|error| format!("模块 {} 无法读取: {error}", file.display()))?;
    if !canonical.starts_with(root) {
        return Err(format!("模块逃逸 Runtime src: {}", file.display()));
    }
    let source = fs::read_to_string(&canonical).map_err(|error| error.to_string())?;
    let parsed = syn::parse_file(&source)
        .map_err(|error| format!("模块 {} 解析失败: {error}", file.display()))?;
    let directory = if matches!(
        file.file_name().and_then(|name| name.to_str()),
        Some("lib.rs" | "mod.rs")
    ) {
        file.parent().ok_or("模块缺少目录")?.to_path_buf()
    } else {
        file.with_extension("")
    };
    load_items(root, file, &directory, name, parsed.items, modules)
}

fn load_items(
    root: &Path,
    file: &Path,
    directory: &Path,
    name: ModulePath,
    items: Vec<Item>,
    modules: &mut Vec<SourceModule>,
) -> Result<(), String> {
    let mut ordinary = Vec::new();
    for item in items {
        if test_only(attributes(&item)) {
            continue;
        }
        if let Item::Mod(module) = item {
            // 动态 path 会让物理目录与逻辑内核不一致；边界中的模块使用固定文件。
            if module
                .attrs
                .iter()
                .any(|attr| attr.path().is_ident("path") || attr.path().is_ident("cfg_attr"))
            {
                return Err(format!(
                    "模块 {} 使用条件或替代路径，无法证明架构边界",
                    module.ident
                ));
            }
            let mut child = name.clone();
            child.push(module.ident.to_string());
            let child_directory = directory.join(module.ident.to_string());
            if let Some((_, items)) = module.content {
                load_items(root, file, &child_directory, child, items, modules)?;
            } else {
                let flat = child_directory.with_extension("rs");
                let nested = child_directory.join("mod.rs");
                let path = match (flat.is_file(), nested.is_file()) {
                    (true, false) => flat,
                    (false, true) => nested,
                    _ => return Err(format!("模块 {} 的文件缺失或不唯一", child.join("::"))),
                };
                load_module(root, &path, child, modules)?;
            }
        } else {
            ordinary.push(item);
        }
    }
    modules.push(SourceModule {
        name,
        file: file.to_path_buf(),
        items: ordinary,
    });
    Ok(())
}

fn check_sources(root: &Path, format_names: &BTreeSet<String>) -> Result<(), String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("Runtime src 缺失: {error}"))?;
    let mut modules = Vec::new();
    load_module(&root, &root.join("lib.rs"), Vec::new(), &mut modules)?;
    for boundary in ["kernel", "admin", "facade"] {
        if !modules.iter().any(|module| module.name == [boundary]) {
            return Err(format!("缺少 Runtime 必需模块 {boundary}"));
        }
    }
    let admission_file = root.join("admin/format_admission.rs");
    if !modules
        .iter()
        .any(|module| module.name == ["admin", "format_admission"] && module.file == admission_file)
    {
        return Err("缺少唯一格式入口 admin/format_admission.rs".to_string());
    }
    let mut imports = Imports::new();
    for module in &modules {
        for item in &module.items {
            if let Item::Use(item) = item {
                let mut paths = Vec::new();
                use_paths(&item.tree, &mut Vec::new(), &mut paths)?;
                for (alias, target) in paths {
                    let mut key = module.name.clone();
                    key.push(alias);
                    let target = if format_names.contains(&target[0])
                        || matches!(target[0].as_str(), "std" | "core" | "alloc")
                    {
                        target
                    } else {
                        absolute(&module.name, &target)
                    };
                    imports.insert(key, target);
                }
            }
        }
    }
    for module in &modules {
        let mut check = SourceCheck {
            module,
            imports: &imports,
            format_names,
            admission_file: &admission_file,
            exported_signature: false,
            error: None,
        };
        for item in &module.items {
            check.visit_item(item);
        }
        if let Some(error) = check.error {
            return Err(format!("{}: {error}", module.file.display()));
        }
    }
    Ok(())
}

struct SourceCheck<'a> {
    module: &'a SourceModule,
    imports: &'a Imports,
    format_names: &'a BTreeSet<String>,
    admission_file: &'a Path,
    exported_signature: bool,
    error: Option<String>,
}

impl SourceCheck<'_> {
    fn path(&mut self, original: ModulePath) {
        if original.is_empty() {
            return;
        }
        let mut path = if self.format_names.contains(&original[0]) {
            original.clone()
        } else {
            absolute(&self.module.name, &original)
        };
        let mut seen = BTreeSet::new();
        while seen.insert(path.clone()) {
            let replacement = (1..=path.len()).rev().find_map(|length| {
                self.imports.get(&path[..length]).map(|target| {
                    let mut next = target.clone();
                    next.extend_from_slice(&path[length..]);
                    next
                })
            });
            match replacement {
                Some(next) if next != path => path = next,
                _ => break,
            }
        }
        let admission = self.module.name == ["admin", "format_admission"]
            && self.module.file == self.admission_file;
        let raw_format = path
            .first()
            .is_some_and(|name| self.format_names.contains(name));
        let kernel = self
            .module
            .name
            .first()
            .is_some_and(|name| name == "kernel");
        let admin_operation = path.first().is_some_and(|name| name == "admin")
            && !matches!(
                path.get(1).map(String::as_str),
                Some("migration_journal" | "state")
            );
        if (raw_format && (!admission || self.exported_signature)) || (kernel && admin_operation) {
            self.error.get_or_insert_with(|| {
                format!(
                    "禁止依赖 {}（解析为 {}）",
                    original.join("::"),
                    path.join("::")
                )
            });
        }
    }

    fn tokens(&mut self, tokens: TokenStream) {
        let tokens: Vec<_> = tokens.into_iter().collect();
        for (index, token) in tokens.iter().enumerate() {
            match token {
                TokenTree::Ident(ident) => {
                    if ident == "include"
                        && matches!(tokens.get(index + 1), Some(TokenTree::Punct(punct)) if punct.as_char() == '!')
                    {
                        self.error
                            .get_or_insert_with(|| "生产宏不得动态 include Rust 源码".to_string());
                    }
                    let mut path = vec![ident.to_string()];
                    let mut next = index + 1;
                    while let [
                        TokenTree::Punct(first),
                        TokenTree::Punct(second),
                        TokenTree::Ident(segment),
                    ] = tokens.get(next..next + 3).unwrap_or(&[])
                    {
                        if first.as_char() != ':' || second.as_char() != ':' {
                            break;
                        }
                        path.push(segment.to_string());
                        next += 3;
                    }
                    self.path(path);
                }
                TokenTree::Group(group) => self.tokens(group.stream()),
                _ => {}
            }
        }
    }
}

impl<'ast> Visit<'ast> for SourceCheck<'_> {
    fn visit_item(&mut self, item: &'ast Item) {
        if !test_only(attributes(item)) {
            if matches!(item, Item::Verbatim(_)) {
                self.error
                    .get_or_insert_with(|| "无法解析的生产 Rust item".to_string());
            }
            visit::visit_item(self, item);
        }
    }
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.exported_signature = !matches!(item.vis, syn::Visibility::Inherited);
        self.visit_signature(&item.sig);
        self.exported_signature = false;
        self.visit_block(&item.block);
    }
    fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
        self.exported_signature = !matches!(item.vis, syn::Visibility::Inherited);
        visit::visit_item_type(self, item);
        self.exported_signature = false;
    }
    fn visit_field(&mut self, field: &'ast syn::Field) {
        let previous = self.exported_signature;
        self.exported_signature |= !matches!(field.vis, syn::Visibility::Inherited);
        visit::visit_field(self, field);
        self.exported_signature = previous;
    }
    fn visit_item_enum(&mut self, item: &'ast syn::ItemEnum) {
        self.exported_signature = !matches!(item.vis, syn::Visibility::Inherited);
        visit::visit_item_enum(self, item);
        self.exported_signature = false;
    }
    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        if !test_only(&item.attrs) {
            self.exported_signature = !matches!(item.vis, syn::Visibility::Inherited);
            self.visit_signature(&item.sig);
            self.exported_signature = false;
            self.visit_block(&item.block);
        }
    }
    fn visit_local(&mut self, local: &'ast syn::Local) {
        if !test_only(&local.attrs) {
            visit::visit_local(self, local);
        }
    }
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        self.exported_signature = !matches!(item.vis, syn::Visibility::Inherited);
        let mut paths = Vec::new();
        match use_paths(&item.tree, &mut Vec::new(), &mut paths) {
            Ok(()) => {
                for (_, path) in paths {
                    self.path(path);
                }
            }
            Err(error) => {
                self.error.get_or_insert(error);
            }
        }
        self.exported_signature = false;
    }
    fn visit_item_extern_crate(&mut self, item: &'ast syn::ItemExternCrate) {
        self.path(vec![item.ident.to_string()]);
    }
    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.path(
            path.segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect(),
        );
        visit::visit_path(self, path);
    }
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if mac.path.is_ident("include") {
            self.error
                .get_or_insert_with(|| "生产模块不得动态 include Rust 源码".to_string());
        }
        self.visit_path(&mac.path);
        self.tokens(mac.tokens.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);
    impl Fixture {
        fn new(kernel: &str, admin: &str, facade: &str, exports: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "laneflow-architecture-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&root).unwrap();
            fs::write(
                root.join("lib.rs"),
                format!("mod kernel; mod admin; mod facade; {exports}"),
            )
            .unwrap();
            fs::write(root.join("kernel.rs"), kernel).unwrap();
            fs::write(
                root.join("admin.rs"),
                format!("pub mod format_admission; {admin}"),
            )
            .unwrap();
            fs::create_dir(root.join("admin")).unwrap();
            fs::write(root.join("admin/format_admission.rs"), "").unwrap();
            fs::write(root.join("facade.rs"), facade).unwrap();
            Self(root)
        }
        fn check(&self) -> Result<(), String> {
            check_sources(
                &self.0,
                &BTreeSet::from([
                    "laneflow_format".into(),
                    "laneflow_runtime_snapshot_wire".into(),
                    "renamed_format".into(),
                ]),
            )
        }
        fn admission(&self, source: &str) {
            fs::write(self.0.join("admin/format_admission.rs"), source).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn kernel_rejects_direct_renamed_and_macro_format_access() {
        for code in [
            "use laneflow_format as format;",
            "fn bad() { ::laneflow_runtime_snapshot_wire::read(); }",
            "use renamed_format::Reader;",
            "macro_rules! bad { () => { laneflow_format::read() } }",
        ] {
            assert!(
                Fixture::new(code, "", "", "")
                    .check()
                    .unwrap_err()
                    .contains("禁止依赖")
            );
        }
    }

    #[test]
    fn only_admission_reads_wire_and_kernel_cannot_follow_reexports() {
        let admitted = Fixture::new("", "", "", "");
        admitted.admission("use laneflow_runtime_snapshot_wire::Reader; pub struct Snapshot;");
        assert!(admitted.check().is_ok());
        let fixture = Fixture::new(
            "use crate::FacadeSnapshot;",
            "",
            "pub use crate::admin::format_admission::Snapshot;",
            "pub use facade::Snapshot as FacadeSnapshot;",
        );
        fixture.admission("pub struct Snapshot;");
        assert!(fixture.check().unwrap_err().contains("禁止依赖"));
        assert!(
            Fixture::new("", "use laneflow_runtime_snapshot_wire::Reader;", "", "")
                .check()
                .is_err()
        );
        let macro_fixture = Fixture::new(
            "macro_rules! bad { () => { crate::FacadeSnapshot } }",
            "",
            "",
            "pub use admin::format_admission::Snapshot as FacadeSnapshot;",
        );
        macro_fixture.admission("pub struct Snapshot;");
        assert!(macro_fixture.check().is_err());
    }

    #[test]
    fn test_only_fixtures_are_excluded_but_optional_production_is_checked() {
        let allowed = "#[cfg(test)] mod tests { use laneflow_format::*; } #[cfg(all(test, feature = \"extra\"))] mod extra;";
        assert!(Fixture::new(allowed, "", "", "").check().is_ok());
        for cfg in ["any(test, feature = \"extra\")", "not(test)"] {
            let source = format!("#[cfg({cfg})] use laneflow_format::Reader;");
            assert!(Fixture::new(&source, "", "", "").check().is_err());
        }
    }

    #[test]
    fn unsupported_or_missing_source_fails_closed() {
        for source in [
            "mod missing;",
            "fn broken(",
            "include!(\"hidden.rs\");",
            "use crate::admin::*;",
            "#[path = \"elsewhere.rs\"] mod hidden;",
        ] {
            assert!(Fixture::new(source, "", "", "").check().is_err());
        }
    }

    #[test]
    fn admission_does_not_export_raw_wire_views() {
        let fixture = Fixture::new("", "", "", "");
        fixture.admission(
            "use laneflow_runtime_snapshot_wire as wire; fn private() -> wire::Reader { todo!() }",
        );
        assert!(fixture.check().is_ok());
        for declaration in [
            "pub(crate) fn read() -> wire::Reader { todo!() }",
            "pub(crate) type Reader = wire::Reader;",
            "pub(crate) use wire::Reader;",
            "pub(crate) struct Escape { pub(crate) raw: wire::Reader }",
            "pub(crate) enum Escape { Raw(wire::Reader) }",
        ] {
            fixture.admission(&format!(
                "use laneflow_runtime_snapshot_wire as wire; {declaration}"
            ));
            assert!(fixture.check().is_err());
        }
    }

    #[test]
    fn production_dependency_direction_is_transitive() {
        let mut graph = BTreeMap::from([
            (RUNTIME.into(), vec!["helper".into()]),
            (SPATIAL.into(), vec![]),
            (ADAPTER.into(), vec![RUNTIME.into(), SPATIAL.into()]),
        ]);
        assert!(check_dependency_graph(&graph).is_ok());
        graph.insert("helper".into(), vec![SPATIAL.into()]);
        assert!(
            check_dependency_graph(&graph)
                .unwrap_err()
                .contains("helper -> laneflow-spatial")
        );
        graph.get_mut(RUNTIME).unwrap().clear();
        graph.get_mut(SPATIAL).unwrap().push(ADAPTER.into());
        assert!(check_dependency_graph(&graph).is_err());
    }
}
