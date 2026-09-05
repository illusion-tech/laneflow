//! 格式入口只导出三个具体函数；不模拟通用的可见性或类型数据流分析。

use syn::{FnArg, GenericArgument, Item, PathArguments, ReturnType, Type, Visibility};

use super::{BTreeSet, Imports, SourceModule, ident_name, path_is_ident, resolve_path};

const SIGNATURES: [&str; 3] = [
    "pub(super) fn verify_semantic_diff(binding: Option<&crate::admin::cutover::SemanticDiffOriginBinding>, bytes: &[u8], base: laneflow_static_network::CanonicalNetworkOrigin, target: laneflow_static_network::CanonicalNetworkOrigin) -> Result<(), crate::admin::cutover::CutoverDescriptorError> {}",
    "pub(super) fn encode_lfrs(snapshot: &crate::admin::snapshot::CapturedSnapshot) -> Vec<u8> {}",
    "pub(super) fn restore_lfrs(bytes: &[u8], revision: std::sync::Arc<laneflow_static_network::SharedNetworkRevision>, source: crate::facade::source::CommittedNetworkSource, config: crate::kernel::config::WorldConfig, limits: crate::admin::snapshot_restore::SnapshotRestoreLimits) -> Result<crate::admin::snapshot_restore::RestoredSnapshot, crate::admin::snapshot_restore::SnapshotRestoreError> {}",
];

pub(super) fn check(
    module: &SourceModule,
    imports: &Imports,
    externals: &BTreeSet<String>,
) -> Result<(), String> {
    let expected: Vec<syn::ItemFn> = SIGNATURES
        .iter()
        .map(|text| syn::parse_str(text).expect("固定格式入口签名"))
        .collect();
    let empty = SourceModule {
        name: Vec::new(),
        file: module.file.clone(),
        items: Vec::new(),
    };
    let mut found = BTreeSet::new();
    for item in &module.items {
        if let Item::Impl(implementation) = item {
            if implementation.trait_.is_some()
                || implementation.items.iter().any(|item| match item {
                    syn::ImplItem::Fn(item) => {
                        !super::test_only(&item.attrs) && !private(&item.vis)
                    }
                    syn::ImplItem::Const(item) => {
                        !super::test_only(&item.attrs) && !private(&item.vis)
                    }
                    syn::ImplItem::Type(item) => {
                        !super::test_only(&item.attrs) && !private(&item.vis)
                    }
                    syn::ImplItem::Macro(item) => !super::test_only(&item.attrs),
                    _ => true,
                })
            {
                return Err("格式入口不允许额外 trait 实现或可见关联项".into());
            }
            continue;
        }
        let visible = match item {
            Item::Fn(item) => !private(&item.vis),
            Item::Use(item) => !private(&item.vis),
            Item::Struct(item) => !private(&item.vis),
            Item::Enum(item) => !private(&item.vis),
            Item::Type(item) => !private(&item.vis),
            Item::Trait(item) => !private(&item.vis),
            Item::TraitAlias(item) => !private(&item.vis),
            Item::Const(item) => !private(&item.vis),
            Item::Static(item) => !private(&item.vis),
            Item::Union(item) => !private(&item.vis),
            Item::ExternCrate(item) => !private(&item.vis),
            Item::Macro(item) => item
                .attrs
                .iter()
                .any(|attr| path_is_ident(attr.path(), "macro_export")),
            // 子模块可见性在模块枚举时检查；foreign/verbatim 不属于这份有限接口。
            _ => return Err("格式入口包含未支持的声明形式".into()),
        };
        if !visible {
            continue;
        }
        let Item::Fn(function) = item else {
            return Err("格式入口只允许三个具体函数，不允许额外可见声明".into());
        };
        let Some(contract) = expected
            .iter()
            .find(|item| ident_name(&item.sig.ident) == ident_name(&function.sig.ident))
        else {
            return Err(format!("格式入口出现未约定函数 {}", function.sig.ident));
        };
        if !matches!(&function.vis, Visibility::Restricted(vis) if vis.in_token.is_none() && path_is_ident(&vis.path, "super"))
            || !found.insert(ident_name(&function.sig.ident))
            || signature(&function.sig, module, imports, externals)?
                != signature(&contract.sig, &empty, &Imports::new(), externals)?
        {
            return Err(format!(
                "格式入口函数 {} 不符合具体接口合同",
                function.sig.ident
            ));
        }
    }
    if found.len() != SIGNATURES.len() {
        return Err("格式入口缺少约定的三个具体函数".into());
    }
    Ok(())
}

fn private(vis: &Visibility) -> bool {
    matches!(vis, Visibility::Inherited)
}

fn signature(
    sig: &syn::Signature,
    module: &SourceModule,
    imports: &Imports,
    externals: &BTreeSet<String>,
) -> Result<Vec<String>, String> {
    if sig.asyncness.is_some()
        || sig.unsafety.is_some()
        || sig.constness.is_some()
        || sig.abi.is_some()
        || sig.variadic.is_some()
        || !sig.generics.params.is_empty()
        || sig.generics.where_clause.is_some()
    {
        return Err(format!(
            "格式入口函数 {} 不能扩展为泛型或其他调用形式",
            sig.ident
        ));
    }
    let mut shapes = Vec::new();
    for input in &sig.inputs {
        let FnArg::Typed(input) = input else {
            return Err("格式入口不能具有方法接收者".into());
        };
        shapes.push(type_shape(&input.ty, module, imports, externals)?);
    }
    let ReturnType::Type(_, output) = &sig.output else {
        return Err("格式入口必须显式声明返回类型".into());
    };
    shapes.push(type_shape(output, module, imports, externals)?);
    Ok(shapes)
}

fn type_shape(
    ty: &Type,
    module: &SourceModule,
    imports: &Imports,
    externals: &BTreeSet<String>,
) -> Result<String, String> {
    match ty {
        Type::Reference(reference)
            if reference.mutability.is_none() && reference.lifetime.is_none() =>
        {
            Ok(format!(
                "&{}",
                type_shape(&reference.elem, module, imports, externals)?
            ))
        }
        Type::Slice(slice) => Ok(format!(
            "[{}]",
            type_shape(&slice.elem, module, imports, externals)?
        )),
        Type::Tuple(tuple) if tuple.elems.is_empty() => Ok("()".into()),
        Type::Path(path) if path.qself.is_none() => {
            let names: Vec<_> = path
                .path
                .segments
                .iter()
                .map(|segment| ident_name(&segment.ident))
                .collect();
            let mut resolved = resolve_path(&module.name, imports, externals, &names);
            if names.len() == 1 && !bound_here(module, imports, &names[0]) {
                let prelude = match names[0].as_str() {
                    "Option" => Some("std::option::Option"),
                    "Result" => Some("std::result::Result"),
                    "Vec" => Some("std::vec::Vec"),
                    "u8" => Some("std::primitive::u8"),
                    _ => None,
                };
                if let Some(prelude) = prelude {
                    resolved = prelude.split("::").map(str::to_owned).collect();
                }
            }
            let mut shape = resolved.join("::");
            for (index, segment) in path.path.segments.iter().enumerate() {
                match &segment.arguments {
                    PathArguments::None => {}
                    PathArguments::AngleBracketed(arguments) if index + 1 == names.len() => {
                        let mut values = Vec::new();
                        for arg in &arguments.args {
                            let GenericArgument::Type(ty) = arg else {
                                return Err("格式接口只接受约定的具体类型参数".into());
                            };
                            values.push(type_shape(ty, module, imports, externals)?);
                        }
                        shape.push_str(&format!("<{}>", values.join(",")));
                    }
                    _ => return Err("格式接口不支持该类型路径形式".into()),
                }
            }
            Ok(shape)
        }
        _ => Err("格式接口仅允许约定的具体类型，不允许 trait 或推断类型出口".into()),
    }
}

fn bound_here(module: &SourceModule, imports: &Imports, name: &str) -> bool {
    let mut key = module.name.clone();
    key.push(name.into());
    imports.contains_key(&key)
        || module.items.iter().any(|item| match item {
            Item::Struct(item) => ident_name(&item.ident) == name,
            Item::Enum(item) => ident_name(&item.ident) == name,
            Item::Type(item) => ident_name(&item.ident) == name,
            Item::Trait(item) => ident_name(&item.ident) == name,
            Item::Union(item) => ident_name(&item.ident) == name,
            _ => false,
        })
}
