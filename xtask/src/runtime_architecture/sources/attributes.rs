//! 生产源码与有限接口共用的属性判定；未知配置保守纳入。

use syn::parse::Parser;
use syn::{Attribute, Item, Meta, Token};

use super::path_is_ident;

#[derive(Clone, Copy)]
enum Truth {
    Yes,
    No,
    Unknown,
}

fn non_test_cfg(meta: &Meta) -> Truth {
    match meta {
        Meta::Path(path) if path_is_ident(path, "test") => Truth::No,
        Meta::List(list) => {
            let Ok(parts) = syn::punctuated::Punctuated::<Meta, Token![,]>::parse_terminated
                .parse2(list.tokens.clone())
            else {
                return Truth::Unknown;
            };
            let values: Vec<_> = parts.iter().map(non_test_cfg).collect();
            if path_is_ident(&list.path, "not") && values.len() == 1 {
                match values[0] {
                    Truth::Yes => Truth::No,
                    Truth::No => Truth::Yes,
                    Truth::Unknown => Truth::Unknown,
                }
            } else if path_is_ident(&list.path, "all") {
                if values.iter().any(|value| matches!(value, Truth::No)) {
                    Truth::No
                } else if values.iter().all(|value| matches!(value, Truth::Yes)) {
                    Truth::Yes
                } else {
                    Truth::Unknown
                }
            } else if path_is_ident(&list.path, "any") {
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

pub(super) fn test_only(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        path_is_ident(attribute.path(), "cfg")
            && attribute
                .parse_args::<Meta>()
                .is_ok_and(|meta| matches!(non_test_cfg(&meta), Truth::No))
    })
}

pub(super) fn of_item(item: &Item) -> &[Attribute] {
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

pub(super) fn of_impl_item(item: &syn::ImplItem) -> &[Attribute] {
    match item {
        syn::ImplItem::Const(item) => &item.attrs,
        syn::ImplItem::Fn(item) => &item.attrs,
        syn::ImplItem::Type(item) => &item.attrs,
        syn::ImplItem::Macro(item) => &item.attrs,
        _ => &[],
    }
}

pub(super) fn of_trait_item(item: &syn::TraitItem) -> &[Attribute] {
    match item {
        syn::TraitItem::Const(item) => &item.attrs,
        syn::TraitItem::Fn(item) => &item.attrs,
        syn::TraitItem::Type(item) => &item.attrs,
        syn::TraitItem::Macro(item) => &item.attrs,
        _ => &[],
    }
}

pub(super) fn of_foreign_item(item: &syn::ForeignItem) -> &[Attribute] {
    match item {
        syn::ForeignItem::Fn(item) => &item.attrs,
        syn::ForeignItem::Static(item) => &item.attrs,
        syn::ForeignItem::Type(item) => &item.attrs,
        syn::ForeignItem::Macro(item) => &item.attrs,
        _ => &[],
    }
}

pub(super) fn exports_macro(attributes: &[Attribute]) -> Result<bool, String> {
    for attribute in attributes {
        if may_export_macro(&attribute.meta)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn may_export_macro(meta: &Meta) -> Result<bool, String> {
    if path_is_ident(meta.path(), "macro_export") {
        return Ok(true);
    }
    if !path_is_ident(meta.path(), "cfg_attr") {
        return Ok(false);
    }
    let Meta::List(list) = meta else {
        return Err("格式入口宏的 cfg_attr 无法解析".into());
    };
    let parts = syn::punctuated::Punctuated::<Meta, Token![,]>::parse_terminated
        .parse2(list.tokens.clone())
        .map_err(|error| format!("格式入口宏的 cfg_attr 无法解析: {error}"))?;
    let mut parts = parts.iter();
    let condition = parts.next().ok_or("格式入口宏的 cfg_attr 缺少条件")?;
    if matches!(non_test_cfg(condition), Truth::No) {
        return Ok(false);
    }
    for attribute in parts {
        if may_export_macro(attribute)? {
            return Ok(true);
        }
    }
    Ok(false)
}
