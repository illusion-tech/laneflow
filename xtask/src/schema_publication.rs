//! JSON Schema publication contract、site 构建与对应测试。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const SCHEMA_PUBLICATION_CATALOG_PATH: &str = "schemas/publication.json";
const SCHEMA_PUBLICATION_README_PATH: &str = "schemas/README.md";
const JSON_SCHEMA_2020_12_URI: &str = "https://json-schema.org/draft/2020-12/schema";
const LEGACY_RAW_SCHEMA_BASE_URL: &str =
    "https://raw.githubusercontent.com/illusion-tech/laneflow/main/schemas/";

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SchemaPublicationCatalog {
    contract_version: u64,
    retention_policy: String,
    pages_base_url: String,
    families: Vec<SchemaFamily>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SchemaFamily {
    family: String,
    schema_file_stem: String,
    current_format_version: String,
    published_schemas: Vec<PublishedSchema>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PublishedSchema {
    format_version: String,
    path: String,
    canonical_url: String,
    source_revision: String,
    source_blob_oid: String,
}

struct PublishedSchemaDocument {
    file_name: String,
    bytes: Vec<u8>,
}

pub(crate) fn check_schema_publication_contract() -> Result<(), String> {
    let (catalog, documents) = validated_schema_publication()?;
    println!(
        "已校验 {} 个 schema family、{} 个 public schema，retention={}",
        catalog.families.len(),
        documents.len(),
        catalog.retention_policy
    );
    Ok(())
}

pub(crate) fn build_schema_publication(output_directory: &str) -> Result<(), String> {
    let (catalog, documents) = validated_schema_publication()?;
    let output_root = safe_publication_output_path(output_directory)?;
    let schema_output = output_root.join("schema");
    validate_existing_publication_output(&output_root, &documents)?;
    fs::create_dir_all(&schema_output).map_err(|error| {
        format!(
            "无法创建 schema publication 输出目录 `{}`: {error}",
            schema_output.display()
        )
    })?;

    fs::write(output_root.join(".nojekyll"), b"")
        .map_err(|error| format!("无法写入 schema publication `.nojekyll`: {error}"))?;
    for document in &documents {
        let destination = schema_output.join(&document.file_name);
        fs::write(&destination, &document.bytes).map_err(|error| {
            format!(
                "无法写入 published schema `{}`: {error}",
                destination.display()
            )
        })?;
    }

    let index = serde_json::json!({
        "contractVersion": catalog.contract_version,
        "retentionPolicy": catalog.retention_policy,
        "families": catalog.families.iter().map(|family| {
            serde_json::json!({
                "family": family.family,
                "currentFormatVersion": family.current_format_version,
                "schemas": family.published_schemas.iter().map(|schema| {
                    let file_name = Path::new(&schema.path)
                        .file_name()
                        .and_then(|value| value.to_str())
                        .expect("validated schema path must have a UTF-8 file name");
                    serde_json::json!({
                        "formatVersion": schema.format_version,
                        "fileName": file_name,
                        "canonicalUrl": schema.canonical_url,
                        "pagesUrl": format!("{}{file_name}", catalog.pages_base_url),
                        "sourceRevision": schema.source_revision,
                        "sourceBlobOid": schema.source_blob_oid,
                    })
                }).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
    });
    let mut index_bytes = serde_json::to_vec_pretty(&index)
        .map_err(|error| format!("无法序列化 schema publication index: {error}"))?;
    index_bytes.push(b'\n');
    fs::write(schema_output.join("index.json"), index_bytes)
        .map_err(|error| format!("无法写入 schema publication index.json: {error}"))?;

    let family_items = catalog
        .families
        .iter()
        .map(|family| {
            let published_current = family
                .published_schemas
                .iter()
                .find(|schema| schema.format_version == family.current_format_version)
                .and_then(|schema| Path::new(&schema.path).file_name())
                .and_then(|value| value.to_str());
            published_current.map_or_else(
                || {
                    format!(
                        "<li>{}: source current {} (publication pending)</li>",
                        family.family, family.current_format_version
                    )
                },
                |file_name| {
                    format!(
                        "<li>{}: <a href=\"schema/{file_name}\">{}</a></li>",
                        family.family, family.current_format_version
                    )
                },
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let root_index = format!(
        "<!doctype html>\n<meta charset=\"utf-8\">\n<title>LaneFlow JSON Schema</title>\n<h1>LaneFlow JSON Schema</h1>\n<ul>\n{family_items}\n</ul>\n<p><a href=\"schema/index.json\">Publication catalog</a></p>\n"
    );
    fs::write(output_root.join("index.html"), root_index)
        .map_err(|error| format!("无法写入 schema publication index.html: {error}"))?;

    println!(
        "已构建 schema publication site：{}（{} 个版本）",
        output_root.display(),
        documents.len()
    );
    Ok(())
}

fn validated_schema_publication()
-> Result<(SchemaPublicationCatalog, Vec<PublishedSchemaDocument>), String> {
    let catalog_bytes = fs::read(SCHEMA_PUBLICATION_CATALOG_PATH).map_err(|error| {
        format!("无法读取 schema publication catalog `{SCHEMA_PUBLICATION_CATALOG_PATH}`: {error}")
    })?;
    let catalog: SchemaPublicationCatalog = serde_json::from_slice(&catalog_bytes).map_err(|error| {
        format!(
            "schema publication catalog `{SCHEMA_PUBLICATION_CATALOG_PATH}` 不是预期 JSON: {error}"
        )
    })?;
    validate_schema_publication_catalog(&catalog)
}

fn validate_schema_publication_catalog(
    catalog: &SchemaPublicationCatalog,
) -> Result<(SchemaPublicationCatalog, Vec<PublishedSchemaDocument>), String> {
    if catalog.contract_version != 2 {
        return Err(format!(
            "schema publication contractVersion 必须为 2，实际为 {}",
            catalog.contract_version
        ));
    }
    if catalog.retention_policy != "immutable-permanent" {
        return Err(format!(
            "schema publication retentionPolicy 必须为 `immutable-permanent`，实际为 `{}`",
            catalog.retention_policy
        ));
    }
    if catalog.pages_base_url != "https://illusion-tech.github.io/laneflow/schema/" {
        return Err(format!(
            "schema publication pagesBaseUrl 不符合 organisation-owned HTTPS path：{}",
            catalog.pages_base_url
        ));
    }
    if catalog.families.is_empty() {
        return Err("schema publication catalog 至少需要一个 schema family".to_string());
    }

    let mut family_names = BTreeSet::new();
    let mut file_stems = BTreeSet::new();
    let mut current_paths = BTreeSet::new();
    let mut published_paths = BTreeSet::new();
    let mut canonical_urls = BTreeSet::new();
    let mut documents = Vec::new();
    for family in &catalog.families {
        if family.family.is_empty()
            || !family
                .family
                .chars()
                .all(|character| character.is_ascii_alphanumeric())
        {
            return Err(format!(
                "schema family 必须是非空 ASCII alphanumeric token：`{}`",
                family.family
            ));
        }
        if !family_names.insert(family.family.as_str()) {
            return Err(format!(
                "schema publication catalog 重复 family `{}`",
                family.family
            ));
        }
        if family.schema_file_stem.is_empty()
            || !family.schema_file_stem.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
        {
            return Err(format!(
                "schemaFileStem 必须是非空安全文件名 token：`{}`",
                family.schema_file_stem
            ));
        }
        if !file_stems.insert(family.schema_file_stem.as_str()) {
            return Err(format!(
                "schema publication catalog 重复 schemaFileStem `{}`",
                family.schema_file_stem
            ));
        }

        let current_version = parse_format_version(&family.current_format_version)?;
        let current_file_name = format!(
            "{}-v{}.schema.json",
            family.schema_file_stem, family.current_format_version
        );
        let current_path = format!("schemas/{current_file_name}");
        if !current_paths.insert(current_path.clone()) {
            return Err(format!(
                "schema publication catalog 的 current source path 重复：`{current_path}`"
            ));
        }
        let current_bytes = fs::read(&current_path).map_err(|error| {
            format!(
                "无法读取 family `{}` 的 current source schema `{current_path}`: {error}",
                family.family
            )
        })?;
        let current_canonical_url = format!("{}{current_file_name}", catalog.pages_base_url);
        validate_schema_document(
            &current_path,
            &current_canonical_url,
            &family.current_format_version,
            &current_bytes,
        )?;

        let mut versions = BTreeSet::new();
        let mut previous_version = None;
        for schema in &family.published_schemas {
            let version = parse_format_version(&schema.format_version)?;
            if previous_version.is_some_and(|previous| version <= previous) {
                return Err(format!(
                    "family `{}` 的 publishedSchemas 必须按 formatVersion 严格递增，`{}` 顺序错误",
                    family.family, schema.format_version
                ));
            }
            previous_version = Some(version);
            if version > current_version {
                return Err(format!(
                    "family `{}` 的 published version `{}` 不得高于 current source `{}`",
                    family.family, schema.format_version, family.current_format_version
                ));
            }
            if !versions.insert(schema.format_version.as_str()) {
                return Err(format!(
                    "family `{}` 重复 published formatVersion `{}`",
                    family.family, schema.format_version
                ));
            }

            let expected_file_name = format!(
                "{}-v{}.schema.json",
                family.schema_file_stem, schema.format_version
            );
            let expected_path = format!("schemas/{expected_file_name}");
            if schema.path != expected_path {
                return Err(format!(
                    "family `{}` schema `{}` 的 path 应为 `{expected_path}`，实际为 `{}`",
                    family.family, schema.format_version, schema.path
                ));
            }
            if !published_paths.insert(schema.path.as_str()) {
                return Err(format!(
                    "schema publication catalog 重复 published path `{}`",
                    schema.path
                ));
            }
            if !schema.canonical_url.starts_with("https://") || schema.canonical_url.contains('#') {
                return Err(format!(
                    "schema `{}` canonicalUrl 必须是无 fragment 的 HTTPS absolute URI：{}",
                    schema.format_version, schema.canonical_url
                ));
            }
            if !canonical_urls.insert(schema.canonical_url.as_str()) {
                return Err(format!(
                    "schema publication catalog 重复 canonicalUrl `{}`",
                    schema.canonical_url
                ));
            }
            let pages_url = format!("{}{expected_file_name}", catalog.pages_base_url);
            let legacy_raw_url = format!("{LEGACY_RAW_SCHEMA_BASE_URL}{expected_file_name}");
            let expected_canonical_url =
                if family.schema_file_stem == "laneflow-data" && version < (0, 4) {
                    &legacy_raw_url
                } else {
                    &pages_url
                };
            if &schema.canonical_url != expected_canonical_url {
                return Err(format!(
                    "family `{}` schema `{}` canonicalUrl 应为 `{expected_canonical_url}`，实际为 `{}`",
                    family.family, schema.format_version, schema.canonical_url,
                ));
            }
            if !valid_git_object_id(&schema.source_revision) {
                return Err(format!(
                    "schema `{}` sourceRevision 必须是完整 40 位 Git OID：{}",
                    schema.format_version, schema.source_revision
                ));
            }
            if !valid_git_object_id(&schema.source_blob_oid) {
                return Err(format!(
                    "schema `{}` sourceBlobOid 必须是完整 40 位 Git OID：{}",
                    schema.format_version, schema.source_blob_oid
                ));
            }

            let working_bytes = fs::read(&schema.path).map_err(|error| {
                format!(
                    "无法读取 published schema `{}`: {error}；已发布版本必须保留在工作树",
                    schema.path
                )
            })?;
            let source_spec = format!("{}:{}", schema.source_revision, schema.path);
            let source_bytes = git_bytes(&["show", &source_spec])?;
            if working_bytes != source_bytes {
                return Err(format!(
                    "published schema `{}` 与 immutable source `{source_spec}` 不一致；不得原地修改已发布版本，请提升 formatVersion",
                    schema.path
                ));
            }
            let actual_blob_oid = git(["rev-parse", source_spec.as_str()])?;
            if actual_blob_oid.trim() != schema.source_blob_oid {
                return Err(format!(
                    "schema `{}` sourceBlobOid 不匹配：catalog={}，Git={}；更新 provenance 前先复核 source revision",
                    schema.format_version,
                    schema.source_blob_oid,
                    actual_blob_oid.trim()
                ));
            }
            validate_schema_document(
                &schema.path,
                &schema.canonical_url,
                &schema.format_version,
                &working_bytes,
            )?;
            documents.push(PublishedSchemaDocument {
                file_name: expected_file_name,
                bytes: working_bytes,
            });
        }
    }

    validate_catalog_covers_schema_directory(catalog)?;
    validate_schema_publication_readme(catalog)?;
    validate_runtime_has_no_schema_network_dependency(catalog)?;

    Ok((catalog.clone(), documents))
}

fn validate_schema_document(
    path: &str,
    canonical_url: &str,
    format_version: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let document: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("schema `{path}` 不是合法 JSON: {error}"))?;
    if document.get("$schema").and_then(serde_json::Value::as_str) != Some(JSON_SCHEMA_2020_12_URI)
    {
        return Err(format!(
            "schema `{path}` 的 `$schema` 必须为 `{JSON_SCHEMA_2020_12_URI}`"
        ));
    }
    if document.get("$id").and_then(serde_json::Value::as_str) != Some(canonical_url) {
        return Err(format!(
            "schema `{path}` 的 `$id` 必须与 catalog canonical URL 完全一致：{canonical_url}"
        ));
    }
    if document
        .pointer("/properties/formatVersion/const")
        .and_then(serde_json::Value::as_str)
        != Some(format_version)
    {
        return Err(format!(
            "schema `{path}` 的 properties.formatVersion.const 必须为 `{format_version}`"
        ));
    }
    Ok(())
}

fn parse_format_version(value: &str) -> Result<(u64, u64), String> {
    let Some((major, minor)) = value.split_once('.') else {
        return Err(format!(
            "formatVersion `{value}` 必须使用 `<major>.<minor>` 数字格式"
        ));
    };
    if minor.contains('.') || major.is_empty() || minor.is_empty() {
        return Err(format!(
            "formatVersion `{value}` 必须使用 `<major>.<minor>` 数字格式"
        ));
    }
    let major = major
        .parse::<u64>()
        .map_err(|_| format!("formatVersion `{value}` major 不是整数"))?;
    let minor = minor
        .parse::<u64>()
        .map_err(|_| format!("formatVersion `{value}` minor 不是整数"))?;
    Ok((major, minor))
}

fn valid_git_object_id(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn validate_catalog_covers_schema_directory(
    catalog: &SchemaPublicationCatalog,
) -> Result<(), String> {
    let mut actual_paths = fs::read_dir("schemas")
        .map_err(|error| format!("无法读取 schemas 目录: {error}"))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let file_name = path.file_name()?.to_str()?;
            file_name
                .ends_with(".schema.json")
                .then(|| path.to_string_lossy().replace('\\', "/"))
        })
        .collect::<Vec<_>>();
    actual_paths.sort();
    let mut catalog_paths = catalog
        .families
        .iter()
        .flat_map(|family| {
            let current = format!(
                "schemas/{}-v{}.schema.json",
                family.schema_file_stem, family.current_format_version
            );
            std::iter::once(current).chain(
                family
                    .published_schemas
                    .iter()
                    .map(|schema| schema.path.clone()),
            )
        })
        .collect::<Vec<_>>();
    catalog_paths.sort();
    catalog_paths.dedup();
    if actual_paths != catalog_paths {
        return Err(format!(
            "schemas 目录与 publication catalog 不一致：目录={actual_paths:?}，catalog={catalog_paths:?}；每个 current source 或 published schema 都必须登记"
        ));
    }
    Ok(())
}

fn validate_schema_publication_readme(catalog: &SchemaPublicationCatalog) -> Result<(), String> {
    let readme = fs::read_to_string(SCHEMA_PUBLICATION_README_PATH)
        .map_err(|error| format!("无法读取 `{SCHEMA_PUBLICATION_README_PATH}`: {error}"))?;
    let current_families = catalog
        .families
        .iter()
        .map(|family| format!("{}={}", family.family, family.current_format_version))
        .collect::<Vec<_>>()
        .join(";");
    for marker in [
        "<!-- schema-publication-contract: public-retrieval -->".to_string(),
        format!("<!-- schema-publication-catalog: {SCHEMA_PUBLICATION_CATALOG_PATH} -->"),
        format!("<!-- schema-source-current: {current_families} -->"),
    ] {
        if !readme.contains(&marker) {
            return Err(format!(
                "`{SCHEMA_PUBLICATION_README_PATH}` 缺少机器可校验标记 `{marker}`"
            ));
        }
    }
    Ok(())
}

fn validate_runtime_has_no_schema_network_dependency(
    catalog: &SchemaPublicationCatalog,
) -> Result<(), String> {
    let canonical_urls = catalog
        .families
        .iter()
        .flat_map(|family| {
            let current = format!(
                "{}{}-v{}.schema.json",
                catalog.pages_base_url, family.schema_file_stem, family.current_format_version
            );
            std::iter::once(current).chain(
                family
                    .published_schemas
                    .iter()
                    .map(|schema| schema.canonical_url.clone()),
            )
        })
        .collect::<BTreeSet<_>>();
    let mut source_paths = Vec::new();
    for root in ["crates/laneflow-core/src", "crates/laneflow-data/src"] {
        collect_rust_sources(Path::new(root), &mut source_paths)?;
    }
    for path in source_paths {
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("无法读取 runtime source `{}`: {error}", path.display()))?;
        for canonical_url in &canonical_urls {
            if source.contains(canonical_url) {
                return Err(format!(
                    "runtime source `{}` 硬编码了 canonical schema URL `{}`；public publication 不得成为 Core/Data 网络依赖",
                    path.display(),
                    canonical_url
                ));
            }
        }
    }
    Ok(())
}

fn collect_rust_sources(root: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(root)
        .map_err(|error| format!("无法读取 runtime source 目录 `{}`: {error}", root.display()))?
    {
        let entry = entry.map_err(|error| {
            format!("无法枚举 runtime source 目录 `{}`: {error}", root.display())
        })?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("无法读取 `{}` 文件类型: {error}", entry.path().display()))?;
        if file_type.is_dir() {
            collect_rust_sources(&entry.path(), paths)?;
        } else if file_type.is_file() && entry.path().extension().is_some_and(|value| value == "rs")
        {
            paths.push(entry.path());
        }
    }
    Ok(())
}

fn git_bytes(args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .map_err(|error| format!("无法运行 git: {error}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(format!(
            "git 命令失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn git<const N: usize>(args: [&str; N]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .map_err(|err| format!("无法运行 git: {err}"))?;

    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|err| format!("git 输出不是 UTF-8: {err}"))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("git 命令失败: {}", stderr.trim()))
    }
}

fn safe_publication_output_path(value: &str) -> Result<PathBuf, String> {
    if value.trim().is_empty() {
        return Err("schema publication output directory 不能为空".to_string());
    }
    let path = PathBuf::from(value);
    if path == Path::new(".") || path.parent().is_none() {
        return Err(format!(
            "schema publication output directory 必须是专用子目录，不能使用 `{value}`"
        ));
    }
    Ok(path)
}

fn validate_existing_publication_output(
    output_root: &Path,
    documents: &[PublishedSchemaDocument],
) -> Result<(), String> {
    if !output_root.exists() {
        return Ok(());
    }
    if !output_root.is_dir() {
        return Err(format!(
            "schema publication output `{}` 已存在且不是目录",
            output_root.display()
        ));
    }
    let allowed_root = BTreeSet::from([".nojekyll", "index.html", "schema"]);
    for entry in fs::read_dir(output_root).map_err(|error| {
        format!(
            "无法读取 schema publication output `{}`: {error}",
            output_root.display()
        )
    })? {
        let entry = entry.map_err(|error| format!("无法枚举 publication output: {error}"))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !allowed_root.contains(name.as_str()) {
            return Err(format!(
                "schema publication output `{}` 含未知条目 `{name}`；请使用专用空目录",
                output_root.display()
            ));
        }
    }
    let schema_output = output_root.join("schema");
    if schema_output.exists() {
        let mut allowed_schema = documents
            .iter()
            .map(|document| document.file_name.as_str())
            .collect::<BTreeSet<_>>();
        allowed_schema.insert("index.json");
        for entry in fs::read_dir(&schema_output).map_err(|error| {
            format!(
                "无法读取 schema publication output `{}`: {error}",
                schema_output.display()
            )
        })? {
            let entry = entry.map_err(|error| format!("无法枚举 schema output: {error}"))?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !allowed_schema.contains(name.as_str()) {
                return Err(format!(
                    "schema publication output `{}` 含未知条目 `{name}`；请使用专用空目录",
                    schema_output.display()
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn published_schema(canonical_url: &str) -> PublishedSchema {
        PublishedSchema {
            format_version: "0.5".to_string(),
            path: "schemas/laneflow-data-v0.5.schema.json".to_string(),
            canonical_url: canonical_url.to_string(),
            source_revision: "aff544e0545239007003a08a41ccfde280e5d20f".to_string(),
            source_blob_oid: "d77383ecc9f6b2fe07320ca74613cbf3106efa01".to_string(),
        }
    }

    #[test]
    fn parses_numeric_format_versions_for_ordering() {
        assert_eq!(parse_format_version("0.5"), Ok((0, 5)));
        assert_eq!(parse_format_version("1.0"), Ok((1, 0)));
        assert!(parse_format_version("0.5.1").is_err());
        assert!(parse_format_version("v0.5").is_err());
    }

    #[test]
    fn validates_full_git_object_ids() {
        assert!(valid_git_object_id(
            "aff544e0545239007003a08a41ccfde280e5d20f"
        ));
        assert!(!valid_git_object_id("aff544e"));
        assert!(!valid_git_object_id(
            "zff544e0545239007003a08a41ccfde280e5d20f"
        ));
    }

    #[test]
    fn validates_published_schema_identity_and_version() {
        let canonical_url =
            "https://illusion-tech.github.io/laneflow/schema/laneflow-data-v0.5.schema.json";
        let schema = published_schema(canonical_url);
        let document = format!(
            r#"{{
              "$schema": "{JSON_SCHEMA_2020_12_URI}",
              "$id": "{canonical_url}",
              "properties": {{"formatVersion": {{"const": "0.5"}}}}
            }}"#
        );

        assert!(
            validate_schema_document(
                &schema.path,
                &schema.canonical_url,
                &schema.format_version,
                document.as_bytes()
            )
            .is_ok()
        );

        let mismatched = document.replace(canonical_url, "https://example.invalid/schema.json");
        let error = validate_schema_document(
            &schema.path,
            &schema.canonical_url,
            &schema.format_version,
            mismatched.as_bytes(),
        )
        .expect_err("catalog and schema $id must match");
        assert!(error.contains("canonical URL"));
    }
}
