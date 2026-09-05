//! 显式模块清单与路径规则；不解析方法调用或展开宏。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::{TokenStream, TokenTree};
use syn::ext::IdentExt;
use syn::visit::{self, Visit};
use syn::{Item, UseTree};

use super::SourceInputs;
mod admission;
mod attributes;
use attributes::test_only;

type ModulePath = Vec<String>;
type Imports = BTreeMap<ModulePath, ModulePath>;

fn ident_name(ident: &syn::Ident) -> String {
    ident.unraw().to_string()
}

fn path_is_ident(path: &syn::Path, expected: &str) -> bool {
    path.get_ident()
        .is_some_and(|ident| ident_name(ident) == expected)
}

fn use_paths(
    tree: &UseTree,
    prefix: &mut ModulePath,
    output: &mut Vec<(String, ModulePath)>,
) -> Result<(), String> {
    match tree {
        UseTree::Path(path) => {
            prefix.push(ident_name(&path.ident));
            use_paths(&path.tree, prefix, output)?;
            prefix.pop();
        }
        UseTree::Name(name) => {
            let mut target = prefix.clone();
            if ident_name(&name.ident) != "self" {
                target.push(ident_name(&name.ident));
            }
            let alias = target.last().ok_or("空 use 路径")?.clone();
            output.push((alias, target));
        }
        UseTree::Rename(rename) => {
            let mut target = prefix.clone();
            if ident_name(&rename.ident) != "self" {
                target.push(ident_name(&rename.ident));
            }
            output.push((ident_name(&rename.rename), target));
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
    let directory = if name.is_empty()
        || matches!(
            file.file_name().and_then(|name| name.to_str()),
            Some("lib.rs" | "mod.rs")
        ) {
        file.parent().ok_or("模块缺少目录")?.to_path_buf()
    } else {
        file.with_extension("")
    };
    load_items(root, &canonical, &directory, name, parsed.items, modules)
}

fn load_items(
    root: &Path,
    file: &Path,
    directory: &Path,
    name: ModulePath,
    items: Vec<Item>,
    modules: &mut Vec<SourceModule>,
) -> Result<(), String> {
    if let Some(boundary) = name.first() {
        if !file.starts_with(root.join(boundary)) {
            return Err(format!("模块 {} 未位于 {boundary}/ 目录", file.display()));
        }
    }
    let mut ordinary = Vec::new();
    for item in items {
        if test_only(attributes::of_item(&item)) {
            continue;
        }
        if let Item::Mod(module) = item {
            if name.is_empty()
                && !matches!(
                    ident_name(&module.ident).as_str(),
                    "kernel" | "admin" | "facade"
                )
            {
                return Err(format!("不允许额外生产根模块 {}", module.ident));
            }
            if name == ["admin", "format_admission"]
                && !matches!(module.vis, syn::Visibility::Inherited)
            {
                return Err("格式入口不允许额外可见模块".into());
            }
            // 动态 path 会让物理目录与逻辑内核不一致；边界中的模块使用固定文件。
            if module.attrs.iter().any(|attr| {
                path_is_ident(attr.path(), "path") || path_is_ident(attr.path(), "cfg_attr")
            }) {
                return Err(format!(
                    "模块 {} 使用条件或替代路径，无法证明架构边界",
                    module.ident
                ));
            }
            let mut child = name.clone();
            child.push(ident_name(&module.ident));
            let child_directory = directory.join(ident_name(&module.ident));
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
            if name.is_empty()
                && !matches!(&item, Item::Use(item) if !matches!(item.vis, syn::Visibility::Inherited))
            {
                return Err("Runtime 库入口只允许显式模块声明与公开 re-export".into());
            }
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

pub(super) fn check(inputs: &SourceInputs) -> Result<(), String> {
    let entry = inputs
        .entry
        .canonicalize()
        .map_err(|error| format!("Runtime 库入口缺失: {error}"))?;
    let package_root = inputs
        .package_root
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !entry.starts_with(&package_root) {
        return Err("Runtime 库入口逃逸包目录".into());
    }
    let root = entry
        .parent()
        .ok_or("Runtime 库入口缺少目录")?
        .to_path_buf();
    let mut modules = Vec::new();
    load_module(&root, &entry, Vec::new(), &mut modules)?;
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
                    let target = if inputs.externals.contains(&target[0]) {
                        target
                    } else {
                        absolute(&module.name, &target)
                    };
                    match imports.entry(key) {
                        std::collections::btree_map::Entry::Vacant(entry) => {
                            entry.insert(target);
                        }
                        std::collections::btree_map::Entry::Occupied(entry) => {
                            if entry.get() != &target {
                                return Err(format!(
                                    "生产导入存在别名歧义 {}：{} / {}",
                                    entry.key().join("::"),
                                    entry.get().join("::"),
                                    target.join("::")
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    for module in &modules {
        if module.name == ["admin", "format_admission"] {
            admission::check(module, &imports, &inputs.externals)?;
        }
        let mut check = SourceCheck {
            module,
            imports: &imports,
            format_names: &inputs.formats,
            externals: &inputs.externals,
            admission_file: &admission_file,
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
    externals: &'a BTreeSet<String>,
    error: Option<String>,
}

fn resolve_path(
    module: &[String],
    imports: &Imports,
    externals: &BTreeSet<String>,
    original: &[String],
) -> ModulePath {
    let mut path = if externals.contains(&original[0]) {
        original.to_vec()
    } else {
        absolute(module, original)
    };
    let mut seen = BTreeSet::new();
    while seen.insert(path.clone()) {
        let replacement = (1..=path.len()).rev().find_map(|length| {
            imports.get(&path[..length]).map(|target| {
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
    path
}

impl SourceCheck<'_> {
    fn reject_include(&mut self, original: &[String]) {
        let resolved = resolve_path(&self.module.name, self.imports, self.externals, original);
        // 保留 include 宏名，不解析宏命名空间；导入时也拒绝以覆盖块内别名。
        if original.last().is_some_and(|name| name == "include")
            || resolved.last().is_some_and(|name| name == "include")
        {
            self.error
                .get_or_insert_with(|| "生产源码不得调用、导入或转导出 include 宏".into());
        }
    }

    fn path(&mut self, original: ModulePath) {
        if original.is_empty() {
            return;
        }
        let path = resolve_path(&self.module.name, self.imports, self.externals, &original);
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
        if (raw_format && !admission) || (kernel && admin_operation) {
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
                    let mut path = vec![ident_name(ident)];
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
                        path.push(ident_name(segment));
                        next += 3;
                    }
                    // 宏体中的显式导入也可能给 include 改名，不只检查后接 ! 的路径。
                    self.reject_include(&path);
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
        if !test_only(attributes::of_item(item)) {
            // 模块层级的声明已由清单加载；这里出现的是函数块内的局部模块。
            if matches!(item, Item::Mod(_)) {
                self.error
                    .get_or_insert_with(|| "生产模块只支持模块层级的显式 mod 声明".into());
                return;
            }
            if matches!(item, Item::Verbatim(_)) {
                self.error
                    .get_or_insert_with(|| "无法解析的生产 Rust item".to_string());
            }
            visit::visit_item(self, item);
        }
    }
    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        if !test_only(attributes::of_impl_item(item)) {
            visit::visit_impl_item(self, item);
        }
    }
    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        if !test_only(attributes::of_trait_item(item)) {
            visit::visit_trait_item(self, item);
        }
    }
    fn visit_foreign_item(&mut self, item: &'ast syn::ForeignItem) {
        if !test_only(attributes::of_foreign_item(item)) {
            visit::visit_foreign_item(self, item);
        }
    }
    fn visit_field(&mut self, field: &'ast syn::Field) {
        if !test_only(&field.attrs) {
            visit::visit_field(self, field);
        }
    }
    fn visit_variant(&mut self, variant: &'ast syn::Variant) {
        if !test_only(&variant.attrs) {
            visit::visit_variant(self, variant);
        }
    }
    fn visit_local(&mut self, local: &'ast syn::Local) {
        if !test_only(&local.attrs) {
            visit::visit_local(self, local);
        }
    }
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        let mut paths = Vec::new();
        match use_paths(&item.tree, &mut Vec::new(), &mut paths) {
            Ok(()) => {
                for (_, path) in paths {
                    self.reject_include(&path);
                    self.path(path);
                }
            }
            Err(error) => {
                self.error.get_or_insert(error);
            }
        }
    }
    fn visit_item_extern_crate(&mut self, item: &'ast syn::ItemExternCrate) {
        if ident_name(&item.ident) == "self" {
            self.error.get_or_insert_with(|| {
                "生产源码不支持 extern crate self 别名；请使用显式 crate:: 路径".into()
            });
        } else {
            self.path(vec![ident_name(&item.ident)]);
        }
    }
    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.path(
            path.segments
                .iter()
                .map(|segment| ident_name(&segment.ident))
                .collect(),
        );
        visit::visit_path(self, path);
    }
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        let path: Vec<_> = mac
            .path
            .segments
            .iter()
            .map(|segment| ident_name(&segment.ident))
            .collect();
        self.reject_include(&path);
        self.visit_path(&mac.path);
        self.tokens(mac.tokens.clone());
    }
}
