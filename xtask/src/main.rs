mod markdown_tables;

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const ALLOWED_TYPES: &[&str] = &[
    "feat", "fix", "docs", "test", "refactor", "perf", "build", "ci", "chore", "revert",
];

const ALLOWED_SLICES: &[&str] = &[
    "docs-only",
    "governance",
    "core-runtime",
    "data-spec",
    "adapter",
    "authoring-tool",
    "example",
    "cross-layer",
];

const REQUIRED_FIELDS: &[&str] = &["Gate", "Slice", "Impact", "Scope", "Validation", "Docs"];
const CURRENT_GATE_VALUES: &[&str] = &["G3 Candidate", "G3 Block"];
const LEGACY_GATE_VALUES: &[&str] = &["G3 Pass", "G3 Waived", "Docs Only"];
// 2026-08-07T00:00:00Z: legacy commit-message syntax migration boundary.
const LEGACY_GATE_CUTOFF_UNIX: u64 = 1_786_060_800;

const DEPENDABOT_AUTHOR_NAME: &str = "dependabot[bot]";
const DEPENDABOT_AUTHOR_EMAIL: &str = "49699333+dependabot[bot]@users.noreply.github.com";
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

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("check-commit-messages") => check_commit_messages(args.get(1).map(String::as_str)),
        Some("check-commit-message-file") => {
            let path = args
                .get(1)
                .ok_or("缺少 commit message 文件路径，例如: cargo +1.96.0 run --locked -p xtask -- check-commit-message-file .git/COMMIT_EDITMSG")?;
            check_commit_message_file(path)
        }
        Some("check-gate-evidence") => check_gate_evidence(&args[1..]),
        Some("format-md-tables") => markdown_tables::run(&args[1..]),
        Some("check-schema-publication-contract") => check_schema_publication_contract(),
        Some("build-schema-publication") => match args.as_slice() {
            [_, output_directory] => build_schema_publication(output_directory),
            _ => Err(
                "用法：cargo +1.96.0 run --locked -p xtask -- build-schema-publication <output-directory>"
                    .to_string(),
            ),
        },
        Some(command) => Err(format!("未知 xtask 命令: {command}")),
        None => Err(
            "缺少 xtask 命令。可用命令: check-commit-messages, check-commit-message-file, check-gate-evidence, format-md-tables, check-schema-publication-contract, build-schema-publication"
                .to_string(),
        ),
    }
}

fn check_schema_publication_contract() -> Result<(), String> {
    let (catalog, documents) = validated_schema_publication()?;
    println!(
        "已校验 {} 个 schema family、{} 个 public schema，retention={}",
        catalog.families.len(),
        documents.len(),
        catalog.retention_policy
    );
    Ok(())
}

fn build_schema_publication(output_directory: &str) -> Result<(), String> {
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

fn check_commit_message_file(path: &str) -> Result<(), String> {
    let message = std::fs::read_to_string(path)
        .map_err(|err| format!("无法读取 commit message 文件 `{path}`: {err}"))?;
    let message = normalize_commit_message_line_endings(&message);
    let message = strip_commit_message_comments(message.as_ref());
    let errors = validate_message("commit-msg", &message);

    if errors.is_empty() {
        println!("已校验 commit message 文件: {path}");
        Ok(())
    } else {
        let mut output = String::from("Commit message 校验失败:");
        for error in errors {
            output.push_str("\n- ");
            output.push_str(&error);
        }
        Err(output)
    }
}

fn normalize_commit_message_line_endings(message: &str) -> Cow<'_, str> {
    if !message.as_bytes().contains(&b'\r') {
        return Cow::Borrowed(message);
    }

    Cow::Owned(message.replace("\r\n", "\n").replace('\r', "\n"))
}

fn strip_commit_message_comments(message: &str) -> String {
    message
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

fn check_commit_messages(explicit_range: Option<&str>) -> Result<(), String> {
    let commit_range = match explicit_range {
        Some(commit_range) => commit_range.to_string(),
        None => commit_range_from_env()?,
    };

    let commits = commit_hashes(&commit_range)?;
    if commits.is_empty() {
        println!("提交范围内没有非 merge commit: {commit_range}");
        return Ok(());
    }

    let mut errors = Vec::new();
    for commit_hash in &commits {
        let message = git(["log", "-1", "--format=%B", commit_hash])?;
        let author_name = git(["log", "-1", "--format=%an", commit_hash])?;
        let author_email = git(["log", "-1", "--format=%ae", commit_hash])?;
        if is_allowed_dependabot_commit(author_name.trim(), author_email.trim(), &message) {
            continue;
        }
        let gate_policy = commit_gate_policy(commit_hash)?;
        errors.extend(validate_message_with_gate_policy(
            commit_hash,
            &message,
            gate_policy,
        ));
    }

    if errors.is_empty() {
        println!(
            "已校验 {} 个 commit message，范围: {}",
            commits.len(),
            commit_range
        );
        Ok(())
    } else {
        let mut output = String::from("Commit message 校验失败:");
        for error in errors {
            output.push_str("\n- ");
            output.push_str(&error);
        }
        Err(output)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommitGatePolicy {
    LocalHook,
    RangeCurrent,
    RangeLegacyCompatible,
}

fn commit_gate_policy(commit_hash: &str) -> Result<CommitGatePolicy, String> {
    let author_timestamp = git(["log", "-1", "--format=%at", commit_hash])?;
    let author_timestamp = author_timestamp.trim().parse::<u64>().map_err(|error| {
        format!(
            "无法解析 commit `{commit_hash}` 的 author timestamp `{}`: {error}",
            author_timestamp.trim()
        )
    })?;
    Ok(commit_gate_policy_for_timestamp(author_timestamp))
}

fn commit_gate_policy_for_timestamp(author_timestamp: u64) -> CommitGatePolicy {
    if author_timestamp < LEGACY_GATE_CUTOFF_UNIX {
        CommitGatePolicy::RangeLegacyCompatible
    } else {
        CommitGatePolicy::RangeCurrent
    }
}

fn is_allowed_dependabot_commit(author_name: &str, author_email: &str, message: &str) -> bool {
    if author_name != DEPENDABOT_AUTHOR_NAME || author_email != DEPENDABOT_AUTHOR_EMAIL {
        return false;
    }

    let message = normalize_commit_message_line_endings(message);
    let title = message.lines().next().unwrap_or_default();
    title.starts_with("build(deps): ")
        && valid_conventional_title(title)
        && !title_has_breaking_bang(title)
}

fn commit_range_from_env() -> Result<String, String> {
    let event_payload = github_event_payload();
    let event_name = env::var("GITHUB_EVENT_NAME").ok();
    let base_ref = env::var("GITHUB_BASE_REF").ok();
    let github_sha = env::var("GITHUB_SHA").ok();

    commit_range_from_event(
        event_name.as_deref(),
        base_ref.as_deref(),
        github_sha.as_deref(),
        event_payload.as_deref(),
    )
}

fn commit_range_from_event(
    event_name: Option<&str>,
    base_ref: Option<&str>,
    github_sha: Option<&str>,
    event_payload: Option<&str>,
) -> Result<String, String> {
    match event_name {
        Some("pull_request") => {
            let base_ref =
                non_empty_value(base_ref).ok_or("pull_request 事件缺少 GITHUB_BASE_REF")?;
            Ok(format!("origin/{base_ref}..HEAD"))
        }
        Some("push") => {
            let event = event_payload.and_then(push_event_payload);
            let after = event
                .as_ref()
                .and_then(|event| event.after.as_deref())
                .filter(|value| is_non_zero_value(value))
                .map(ToOwned::to_owned)
                .or_else(|| {
                    non_empty_value(github_sha)
                        .filter(|value| is_non_zero_value(value))
                        .map(ToOwned::to_owned)
                })
                .ok_or("push 事件缺少可用的 after 或 GITHUB_SHA，无法推导 commit range")?;

            match event.as_ref().and_then(|event| event.before.as_deref()) {
                Some(before) if is_non_zero_value(before) => Ok(format!("{before}..{after}")),
                _ => Ok(format!("{after}^!")),
            }
        }
        Some(event_name) => Err(format!(
            "不支持从 GitHub event `{event_name}` 自动推导 commit range，请显式传入 rev-range，例如: cargo +1.96.0 run --locked -p xtask -- check-commit-messages origin/main..HEAD"
        )),
        None => Err(
            "非 CI 场景必须显式传入 commit rev-range，例如: cargo +1.96.0 run --locked -p xtask -- check-commit-messages origin/main..HEAD"
                .to_string(),
        ),
    }
}

fn github_event_payload() -> Option<String> {
    let path = env::var("GITHUB_EVENT_PATH").ok()?;
    std::fs::read_to_string(path).ok()
}

#[derive(serde::Deserialize)]
struct PushEventPayload {
    before: Option<String>,
    after: Option<String>,
}

fn push_event_payload(payload: &str) -> Option<PushEventPayload> {
    serde_json::from_str(payload).ok()
}

fn is_zero_oid(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|ch| ch == '0')
}

fn is_non_zero_value(value: &str) -> bool {
    !value.trim().is_empty() && !is_zero_oid(value)
}

fn non_empty_value(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

fn commit_hashes(commit_range: &str) -> Result<Vec<String>, String> {
    let output = git(["rev-list", "--no-merges", "--reverse", commit_range])?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GateEvidencePhase {
    G3,
    G4,
}

impl GateEvidencePhase {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "g3" => Ok(Self::G3),
            "g4" => Ok(Self::G4),
            _ => Err(format!(
                "未知 Gate evidence 阶段 `{value}`，应为 `g3` 或 `g4`"
            )),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct GateEvidenceArgs {
    phase: GateEvidencePhase,
    repo: String,
    issue: u64,
    delivery_pr: u64,
    related_prs: Vec<u64>,
}

#[derive(Debug, serde::Deserialize)]
struct GitHubIssue {
    body: String,
    state: String,
    #[serde(rename = "projectItems")]
    project_items: Vec<ProjectItem>,
    comments: Vec<GitHubComment>,
}

#[derive(Debug, serde::Deserialize)]
struct GitHubPullRequest {
    body: String,
    state: String,
    #[serde(rename = "mergedAt")]
    merged_at: Option<String>,
    #[serde(rename = "closingIssuesReferences")]
    closing_issues_references: Vec<IssueReference>,
    #[serde(rename = "projectItems")]
    project_items: Vec<ProjectItem>,
    comments: Vec<GitHubComment>,
}

#[derive(Debug, serde::Deserialize)]
struct ProjectItem {
    title: String,
    status: Option<ProjectStatus>,
}

#[derive(Debug, serde::Deserialize)]
struct ProjectStatus {
    name: String,
}

#[derive(Debug, serde::Deserialize)]
struct IssueReference {
    number: u64,
}

#[derive(Debug, serde::Deserialize)]
struct GitHubComment {
    url: String,
    body: String,
    #[serde(rename = "createdAt")]
    created_at: String,
}

const G3_COMMENT_FIELDS: &[&str] = &[
    "## G3 合并判断",
    "- Checks：",
    "- 审阅：",
    "- 验证：",
    "- 风险：",
    "- 例外：",
    "- 合并方式：",
    "- Gate 断言：",
];

const G4_COMMENT_FIELDS: &[&str] = &[
    "## G4 完成判断",
    "- 合并：",
    "- main CI：",
    "- 验收：",
    "- Project：",
    "- 关系：",
    "- 分支：",
    "- 权限 / bypass：",
    "- Gate 断言：",
];

const GATE_ASSERTION_PREFIX: &str = "- Gate 断言：";

fn check_gate_evidence(args: &[String]) -> Result<(), String> {
    let args = parse_gate_evidence_args(args)?;
    let issue = gh_issue_view(&args.repo, args.issue)?;
    let delivery_pr = gh_pr_view(&args.repo, args.delivery_pr)?;
    let related_prs = args
        .related_prs
        .iter()
        .map(|number| gh_pr_view(&args.repo, *number))
        .collect::<Result<Vec<_>, _>>()?;

    validate_g3_evidence(&args, &issue, &delivery_pr, &related_prs)?;
    if args.phase == GateEvidencePhase::G4 {
        validate_g4_evidence(&args, &issue, &delivery_pr, &related_prs)?;
    }

    println!(
        "已校验 Gate {} 远端证据：Issue #{}，Delivery PR #{}",
        match args.phase {
            GateEvidencePhase::G3 => "G3",
            GateEvidencePhase::G4 => "G4",
        },
        args.issue,
        args.delivery_pr
    );
    Ok(())
}

fn parse_gate_evidence_args(args: &[String]) -> Result<GateEvidenceArgs, String> {
    let phase = args
        .first()
        .ok_or_else(|| "缺少 Gate evidence 阶段，应为 `g3` 或 `g4`".to_string())
        .and_then(|value| GateEvidencePhase::parse(value))?;

    let mut repo = None;
    let mut issue = None;
    let mut delivery_pr = None;
    let mut related_prs = Vec::new();
    let mut index = 1;
    while index < args.len() {
        let flag = &args[index];
        let value = args.get(index + 1).ok_or_else(|| {
            format!(
                "`{flag}` 缺少值。用法：check-gate-evidence <g3|g4> --repo <owner/repo> --issue <number> --delivery-pr <number> [--related-pr <number>]..."
            )
        })?;

        match flag.as_str() {
            "--repo" => {
                if repo.replace(value.clone()).is_some() {
                    return Err("`--repo` 只能指定一次".to_string());
                }
            }
            "--issue" => {
                if issue
                    .replace(parse_issue_number("--issue", value)?)
                    .is_some()
                {
                    return Err("`--issue` 只能指定一次".to_string());
                }
            }
            "--delivery-pr" => {
                if delivery_pr
                    .replace(parse_issue_number("--delivery-pr", value)?)
                    .is_some()
                {
                    return Err("`--delivery-pr` 只能指定一次".to_string());
                }
            }
            "--related-pr" => related_prs.push(parse_issue_number("--related-pr", value)?),
            _ => return Err(format!("未知 check-gate-evidence 参数：{flag}")),
        }
        index += 2;
    }

    let repo = repo.ok_or("缺少 `--repo <owner/repo>`")?;
    if !valid_repository_name(&repo) {
        return Err(format!("`--repo` 格式不正确：{repo}，应为 `owner/repo`"));
    }
    let issue = issue.ok_or("缺少 `--issue <number>`")?;
    let delivery_pr = delivery_pr.ok_or("缺少 `--delivery-pr <number>`")?;
    let all_prs = related_prs
        .iter()
        .copied()
        .chain(std::iter::once(delivery_pr))
        .collect::<BTreeSet<_>>();
    if all_prs.len() != related_prs.len() + 1 {
        return Err("Delivery PR 与 Related PR 不能重复".to_string());
    }
    Ok(GateEvidenceArgs {
        phase,
        repo,
        issue,
        delivery_pr,
        related_prs,
    })
}

fn parse_issue_number(flag: &str, value: &str) -> Result<u64, String> {
    value
        .strip_prefix('#')
        .unwrap_or(value)
        .parse::<u64>()
        .ok()
        .filter(|number| *number > 0)
        .ok_or_else(|| format!("`{flag}` 必须是正整数 Issue / PR 编号：{value}"))
}

fn valid_repository_name(repo: &str) -> bool {
    let Some((owner, name)) = repo.split_once('/') else {
        return false;
    };
    !owner.is_empty() && !name.is_empty() && !name.contains('/')
}

fn gh_issue_view(repo: &str, number: u64) -> Result<GitHubIssue, String> {
    gh_json(&[
        "issue".to_string(),
        "view".to_string(),
        number.to_string(),
        "--repo".to_string(),
        repo.to_string(),
        "--json".to_string(),
        "body,state,projectItems,comments".to_string(),
    ])
}

fn gh_pr_view(repo: &str, number: u64) -> Result<GitHubPullRequest, String> {
    gh_json(&[
        "pr".to_string(),
        "view".to_string(),
        number.to_string(),
        "--repo".to_string(),
        repo.to_string(),
        "--json".to_string(),
        "body,state,mergedAt,closingIssuesReferences,projectItems,comments".to_string(),
    ])
}

fn gh_json<T: serde::de::DeserializeOwned>(args: &[String]) -> Result<T, String> {
    let output = Command::new("gh")
        .args(args)
        .output()
        .map_err(|err| format!("无法运行 gh: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh 命令失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    serde_json::from_slice(&output.stdout).map_err(|err| {
        format!(
            "gh 输出不是预期 JSON: {err}; 原始输出：{}",
            String::from_utf8_lossy(&output.stdout).trim()
        )
    })
}

fn validate_g3_evidence(
    args: &GateEvidenceArgs,
    issue: &GitHubIssue,
    delivery_pr: &GitHubPullRequest,
    related_prs: &[GitHubPullRequest],
) -> Result<(), String> {
    let issue_g3_line = completed_gate_line(&issue.body, "G3")?;
    let delivery_pr_line = metadata_line(&issue.body, "Delivery PR")?;
    if !delivery_pr_line.contains(&format!("#{}", args.delivery_pr)) {
        return Err(format!(
            "Issue 的 `Delivery PR` 字段未记录 Delivery PR #{}",
            args.delivery_pr
        ));
    }
    let related_prs_line = metadata_line(&issue.body, "Related PRs")?;
    let recorded_related_prs = metadata_issue_numbers(related_prs_line);
    let requested_related_prs = args.related_prs.iter().copied().collect::<BTreeSet<_>>();
    if recorded_related_prs != requested_related_prs {
        return Err(format!(
            "Issue 的 `Related PRs` 字段与命令参数不一致：Issue 记录 [{}]；命令传入 [{}]",
            format_issue_numbers(&recorded_related_prs),
            format_issue_numbers(&requested_related_prs)
        ));
    }
    let delivery_permalink = completed_gate_permalink(&delivery_pr.body, "G3")?;
    if !issue_g3_line.contains(&delivery_permalink) {
        return Err("Issue 的 G3 checkbox 未回链 Delivery PR 的 G3 comment permalink".to_string());
    }
    validate_comment(
        delivery_pr,
        &delivery_permalink,
        G3_COMMENT_FIELDS,
        "Delivery PR G3",
        args,
    )?;
    validate_g3_timing(delivery_pr, &delivery_permalink, "Delivery PR")?;
    if !delivery_pr
        .closing_issues_references
        .iter()
        .any(|reference| reference.number == args.issue)
    {
        return Err(format!(
            "Delivery PR #{} 的 closingIssuesReferences 未覆盖 Issue #{}",
            args.delivery_pr, args.issue
        ));
    }

    for (number, related_pr) in args.related_prs.iter().zip(related_prs) {
        let permalink = completed_gate_permalink(&related_pr.body, "G3")?;
        if !issue_g3_line.contains(&permalink) {
            return Err(format!(
                "Issue 的 G3 checkbox 未回链 Related PR #{number} 的 G3 comment permalink"
            ));
        }
        validate_comment(
            related_pr,
            &permalink,
            G3_COMMENT_FIELDS,
            &format!("Related PR #{number} G3"),
            args,
        )?;
        validate_g3_timing(related_pr, &permalink, &format!("Related PR #{number}"))?;
        if related_pr
            .closing_issues_references
            .iter()
            .any(|reference| reference.number == args.issue)
        {
            return Err(format!(
                "Related PR #{number} 不得以 closing keyword 覆盖 Issue #{}",
                args.issue
            ));
        }
        if !related_pr.body.contains(&format!("Refs: #{}", args.issue)) {
            return Err(format!(
                "Related PR #{number} 缺少 `Refs: #{}` 关系记录",
                args.issue
            ));
        }
    }
    Ok(())
}

fn validate_g4_evidence(
    args: &GateEvidenceArgs,
    issue: &GitHubIssue,
    delivery_pr: &GitHubPullRequest,
    related_prs: &[GitHubPullRequest],
) -> Result<(), String> {
    if issue.state != "OPEN" {
        return Err("G4 断言必须在手动关闭 Issue 前运行".to_string());
    }
    let merged_at = delivery_pr
        .merged_at
        .as_deref()
        .ok_or("Delivery PR 尚未合并，不能通过 G4")?;
    if delivery_pr.state != "MERGED" {
        return Err("Delivery PR 状态不是 MERGED，不能通过 G4".to_string());
    }
    let mut latest_merge = merged_at;
    for (number, related_pr) in args.related_prs.iter().zip(related_prs) {
        let related_merged_at = related_pr
            .merged_at
            .as_deref()
            .ok_or_else(|| format!("Related PR #{number} 尚未合并，不能通过 G4"))?;
        if related_pr.state != "MERGED" {
            return Err(format!("Related PR #{number} 状态不是 MERGED，不能通过 G4"));
        }
        if related_merged_at > latest_merge {
            latest_merge = related_merged_at;
        }
    }

    let issue_g4_permalink = completed_gate_permalink(&issue.body, "G4")?;
    let g4_comment = comment_for_permalink(issue, &issue_g4_permalink, "Issue G4")?;
    validate_comment_body(&g4_comment.body, G4_COMMENT_FIELDS, "Issue G4")?;
    validate_gate_assertion(&g4_comment.body, "Issue G4", args, GateEvidencePhase::G4)?;
    if g4_comment.created_at.as_str() < latest_merge {
        return Err("Issue G4 comment 早于最后一个关联 PR 的合并时间".to_string());
    }
    if !delivery_pr.body.contains("G4 回写") || !delivery_pr.body.contains(&issue_g4_permalink) {
        return Err(
            "Delivery PR body 缺少指向 Issue G4 comment 的 `G4 回写` permalink".to_string(),
        );
    }
    for gate in ["G0", "G1", "G2", "G3", "G4"] {
        completed_gate_line(&issue.body, gate)?;
    }
    if !is_laneflow_project_done(&issue.project_items) {
        return Err("Issue 尚未处于 LaneFlow Project 的 Done 状态".to_string());
    }
    if !is_laneflow_project_done(&delivery_pr.project_items) {
        return Err("Delivery PR 尚未处于 LaneFlow Project 的 Done 状态".to_string());
    }
    for (number, related_pr) in args.related_prs.iter().zip(related_prs) {
        if !is_laneflow_project_done(&related_pr.project_items) {
            return Err(format!(
                "Related PR #{number} 尚未处于 LaneFlow Project 的 Done 状态"
            ));
        }
    }
    Ok(())
}

fn is_laneflow_project_done(project_items: &[ProjectItem]) -> bool {
    project_items.iter().any(|item| {
        item.title == "LaneFlow"
            && item
                .status
                .as_ref()
                .is_some_and(|status| status.name == "Done")
    })
}

fn completed_gate_line<'a>(body: &'a str, gate: &str) -> Result<&'a str, String> {
    let prefix = gate_ledger_prefix(gate)?;
    body.lines()
        .find(|line| line.starts_with(prefix))
        .ok_or_else(|| format!("body 缺少已勾选的 `{gate}` Gate Ledger 项"))
}

fn gate_ledger_prefix(gate: &str) -> Result<&'static str, String> {
    match gate {
        "G0" => Ok("- [x] G0 立项已记录："),
        "G1" => Ok("- [x] G1 设计判断已记录："),
        "G2" => Ok("- [x] G2 开工判断已记录："),
        "G3" => Ok("- [x] G3 合并判断已记录："),
        "G4" => Ok("- [x] G4 完成判断已记录："),
        _ => Err(format!("未知 Gate：{gate}")),
    }
}

fn metadata_line<'a>(body: &'a str, field: &str) -> Result<&'a str, String> {
    body.lines()
        .find(|line| line.starts_with(&format!("- {field}：")))
        .ok_or_else(|| format!("body 缺少 `{field}` 元数据字段"))
}

fn metadata_issue_numbers(line: &str) -> BTreeSet<u64> {
    line.split('#')
        .skip(1)
        .filter_map(|tail| {
            let digits = tail
                .chars()
                .take_while(|character| character.is_ascii_digit())
                .collect::<String>();
            digits.parse::<u64>().ok().filter(|number| *number > 0)
        })
        .collect()
}

fn format_issue_numbers(numbers: &BTreeSet<u64>) -> String {
    if numbers.is_empty() {
        "N/A".to_string()
    } else {
        numbers
            .iter()
            .map(|number| format!("#{number}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn completed_gate_permalink(body: &str, gate: &str) -> Result<String, String> {
    let line = completed_gate_line(body, gate)?;
    extract_comment_permalink(line)
        .ok_or_else(|| format!("已勾选的 `{gate}` Gate Ledger 项缺少直接 GitHub comment permalink"))
}

fn extract_comment_permalink(line: &str) -> Option<String> {
    let start = line.find("https://github.com/")?;
    let permalink = line[start..]
        .split(|character: char| character.is_whitespace() || character == ')')
        .next()?;
    permalink
        .contains("#issuecomment-")
        .then(|| permalink.to_string())
}

fn validate_comment(
    pr: &GitHubPullRequest,
    permalink: &str,
    required_fields: &[&str],
    label: &str,
    args: &GateEvidenceArgs,
) -> Result<(), String> {
    let comment = pr
        .comments
        .iter()
        .find(|comment| comment.url == permalink)
        .ok_or_else(|| format!("{label} permalink 未指向该 PR 的 comment"))?;
    validate_comment_body(&comment.body, required_fields, label)?;
    validate_gate_assertion(&comment.body, label, args, GateEvidencePhase::G3)
}

fn comment_for_permalink<'a>(
    issue: &'a GitHubIssue,
    permalink: &str,
    label: &str,
) -> Result<&'a GitHubComment, String> {
    issue
        .comments
        .iter()
        .find(|comment| comment.url == permalink)
        .ok_or_else(|| format!("{label} permalink 未指向该 Issue 的 comment"))
}

fn validate_comment_body(body: &str, required_fields: &[&str], label: &str) -> Result<(), String> {
    let missing_fields = required_fields
        .iter()
        .filter(|field| !body.contains(**field))
        .copied()
        .collect::<Vec<_>>();
    if missing_fields.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{label} comment 缺少必需字段：{}",
            missing_fields.join("、")
        ))
    }
}

fn validate_gate_assertion(
    body: &str,
    label: &str,
    args: &GateEvidenceArgs,
    phase: GateEvidencePhase,
) -> Result<(), String> {
    let assertion_lines = body
        .lines()
        .filter(|line| line.starts_with(GATE_ASSERTION_PREFIX))
        .collect::<Vec<_>>();
    let assertion_line = match assertion_lines.as_slice() {
        [line] => *line,
        [] => return Err(format!("{label} comment 缺少独立的 `Gate 断言` 行")),
        _ => return Err(format!("{label} comment 只能包含一条 `Gate 断言` 行")),
    };

    let value = assertion_line
        .strip_prefix(GATE_ASSERTION_PREFIX)
        .expect("filtered assertion line must have the prefix")
        .trim();
    let Some(command_and_result) = value.strip_prefix('`') else {
        return Err(format!(
            "{label} comment 的 `Gate 断言` 必须先用反引号记录规范命令"
        ));
    };
    let Some((actual_command, result)) = command_and_result.split_once('`') else {
        return Err(format!("{label} comment 的 `Gate 断言` 命令缺少闭合反引号"));
    };

    let expected_command = expected_gate_command(args, phase);
    if actual_command != expected_command {
        return Err(format!(
            "{label} comment 的 `Gate 断言` 命令与当前参数不一致：期望 `{expected_command}`；实际 `{actual_command}`"
        ));
    }

    if !matches!(result.trim(), "已通过" | "已通过。") {
        return Err(format!(
            "{label} comment 的 `Gate 断言` 必须在规范命令后明确记录 `已通过`"
        ));
    }

    Ok(())
}

fn expected_gate_command(args: &GateEvidenceArgs, phase: GateEvidencePhase) -> String {
    let phase = match phase {
        GateEvidencePhase::G3 => "g3",
        GateEvidencePhase::G4 => "g4",
    };
    let mut command = format!(
        "cargo +1.96.0 run --locked -p xtask -- check-gate-evidence {phase} --repo {} --issue {} --delivery-pr {}",
        args.repo, args.issue, args.delivery_pr
    );
    for related_pr in &args.related_prs {
        command.push_str(&format!(" --related-pr {related_pr}"));
    }
    command
}

fn validate_g3_timing(pr: &GitHubPullRequest, permalink: &str, label: &str) -> Result<(), String> {
    let Some(merged_at) = pr.merged_at.as_deref() else {
        return Ok(());
    };
    let comment = pr
        .comments
        .iter()
        .find(|comment| comment.url == permalink)
        .ok_or_else(|| format!("{label} permalink 未指向该 PR 的 comment"))?;
    if comment.created_at.as_str() > merged_at {
        return Err(format!("{label} comment 创建时间晚于 PR 合并时间"));
    }
    Ok(())
}

fn validate_message(commit_hash: &str, message: &str) -> Vec<String> {
    validate_message_with_gate_policy(commit_hash, message, CommitGatePolicy::LocalHook)
}

fn validate_message_with_gate_policy(
    commit_hash: &str,
    message: &str,
    gate_policy: CommitGatePolicy,
) -> Vec<String> {
    let message = normalize_commit_message_line_endings(message);
    let message = message.as_ref();
    let title = message.lines().next().unwrap_or_default();
    let mut errors = Vec::new();
    let has_breaking_bang = title_has_breaking_bang(title);
    let breaking_change_footer_count = breaking_change_footer_count(message);
    let has_breaking_change_footer = breaking_change_footer_count > 0;

    if !valid_conventional_title(title) {
        errors.push("标题不符合 Conventional Commits".to_string());
    }

    for field in REQUIRED_FIELDS {
        if !has_non_empty_field(message, field) {
            errors.push(format!("缺少 `{field}: ` 行"));
        }
    }

    if !has_valid_governance_block(message) {
        errors.push(
            "`Gate`/`Slice`/`Impact`/`Scope`/`Validation`/`Docs` 必须作为连续治理字段块；标题后空一行，`Docs` 后空一行并接最后的 `Refs:`/`Closes:` footer"
                .to_string(),
        );
    }

    if !has_valid_gate(message, gate_policy) {
        let allowed = match gate_policy {
            CommitGatePolicy::LocalHook => "`G3 Candidate` 或 `G3 Block`",
            CommitGatePolicy::RangeCurrent => "`G3 Candidate`；`G3 Block` 不得进入合并范围",
            CommitGatePolicy::RangeLegacyCompatible => {
                "`G3 Candidate` 或迁移期 legacy 值 `G3 Pass` / `G3 Waived` / `Docs Only`；`G3 Block` 不得进入合并范围"
            }
        };
        errors.push(format!("`Gate` 必须是 {allowed}"));
    }

    if !has_valid_slice(message) {
        errors.push("`Slice` 缺失或不是支持的 LaneFlow 切片类型".to_string());
    }

    if !has_valid_impact(message) {
        errors.push("`Impact` 必须同时覆盖 core-api、data-format 和 adapter-api".to_string());
    }

    if !has_valid_docs(message) {
        errors.push("`Docs` 必须是 updated、not required 或 pending <reason>".to_string());
    }

    if has_breaking_bang && !has_breaking_change_footer {
        errors.push("标题包含 `!` 时必须提供 `BREAKING CHANGE: ` footer".to_string());
    }

    if has_breaking_change_footer && !has_breaking_bang {
        errors.push("`BREAKING CHANGE: ` footer 必须与标题 `!` 同时使用".to_string());
    }

    if has_breaking_change_footer && !has_single_valid_breaking_change_footer(message) {
        errors.push(
            "`BREAKING CHANGE: ` footer 格式不正确，必须在 `Refs:` / `Closes:` 前提供单行非空说明"
                .to_string(),
        );
    }

    if (has_breaking_bang || has_breaking_change_footer) && !has_changed_impact(message) {
        errors.push("破坏性变更必须将 `Impact` 至少一项标为 changed".to_string());
    }

    if let Err(error) = validate_issue_footer(message) {
        errors.push(error.message().to_string());
    }

    errors
        .into_iter()
        .map(|error| {
            let short_hash = commit_hash.chars().take(12).collect::<String>();
            format!("{short_hash} {title}: {error}")
        })
        .collect()
}

fn has_valid_gate(message: &str, gate_policy: CommitGatePolicy) -> bool {
    message.lines().any(|line| {
        let Some(gate) = field_value(line, "Gate") else {
            return false;
        };
        match gate_policy {
            CommitGatePolicy::LocalHook => CURRENT_GATE_VALUES.contains(&gate),
            CommitGatePolicy::RangeCurrent => gate == "G3 Candidate",
            CommitGatePolicy::RangeLegacyCompatible => {
                gate == "G3 Candidate" || LEGACY_GATE_VALUES.contains(&gate)
            }
        }
    })
}

fn valid_conventional_title(title: &str) -> bool {
    let Some((prefix, description)) = title.split_once(": ") else {
        return false;
    };
    if description.trim().is_empty() {
        return false;
    }

    let prefix = prefix.strip_suffix('!').unwrap_or(prefix);
    let (commit_type, scope) = match prefix.split_once('(') {
        Some((commit_type, scope_with_suffix)) => {
            let Some(scope) = scope_with_suffix.strip_suffix(')') else {
                return false;
            };
            (commit_type, Some(scope))
        }
        None => (prefix, None),
    };

    ALLOWED_TYPES.contains(&commit_type) && scope.is_none_or(valid_scope)
}

fn title_has_breaking_bang(title: &str) -> bool {
    title
        .split_once(": ")
        .is_some_and(|(prefix, _description)| prefix.ends_with('!'))
}

fn valid_scope(scope: &str) -> bool {
    let mut chars = scope.chars();
    match chars.next() {
        Some(ch) if ch.is_ascii_lowercase() || ch.is_ascii_digit() => {}
        _ => return false,
    }

    chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-'))
}

fn has_non_empty_field(message: &str, field: &str) -> bool {
    message
        .lines()
        .any(|line| field_value(line, field).is_some_and(|value| !value.trim().is_empty()))
}

fn field_value<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    line.strip_prefix(field)?.strip_prefix(": ")
}

fn has_valid_governance_block(message: &str) -> bool {
    let lines = message.lines().collect::<Vec<_>>();
    let field_start = 2;
    let blank_before_footer = field_start + REQUIRED_FIELDS.len();
    let footer_start = blank_before_footer + 1;

    if lines.get(1).is_none_or(|line| !line.trim().is_empty()) {
        return false;
    }

    for (offset, field) in REQUIRED_FIELDS.iter().enumerate() {
        let Some(line) = lines.get(field_start + offset) else {
            return false;
        };
        if field_value(line, field).is_none_or(|value| value.trim().is_empty()) {
            return false;
        }
    }

    if lines
        .get(blank_before_footer)
        .is_none_or(|line| !line.trim().is_empty())
    {
        return false;
    }

    let Some(last_non_empty_index) = lines.iter().rposition(|line| !line.trim().is_empty()) else {
        return false;
    };

    if !lines
        .get(last_non_empty_index)
        .is_some_and(|line| valid_issue_footer_line(line))
    {
        return false;
    }

    match last_non_empty_index.checked_sub(footer_start) {
        Some(0) => true,
        Some(1) => lines
            .get(footer_start)
            .is_some_and(|line| valid_breaking_change_footer_line(line)),
        _ => false,
    }
}

fn has_valid_slice(message: &str) -> bool {
    message
        .lines()
        .any(|line| field_value(line, "Slice").is_some_and(|slice| ALLOWED_SLICES.contains(&slice)))
}

fn has_valid_impact(message: &str) -> bool {
    message.lines().any(|line| {
        let Some(value) = field_value(line, "Impact") else {
            return false;
        };
        let parts = value.split("; ").collect::<Vec<_>>();
        parts.len() == 3
            && matches!(parts[0], "core-api=none" | "core-api=changed")
            && matches!(parts[1], "data-format=none" | "data-format=changed")
            && matches!(parts[2], "adapter-api=none" | "adapter-api=changed")
    })
}

fn has_changed_impact(message: &str) -> bool {
    message.lines().any(|line| {
        let Some(value) = field_value(line, "Impact") else {
            return false;
        };
        value.split("; ").any(|part| {
            matches!(
                part,
                "core-api=changed" | "data-format=changed" | "adapter-api=changed"
            )
        })
    })
}

fn has_valid_docs(message: &str) -> bool {
    message.lines().any(|line| {
        field_value(line, "Docs").is_some_and(|docs| {
            matches!(docs, "updated" | "not required")
                || docs
                    .strip_prefix("pending ")
                    .is_some_and(|reason| !reason.trim().is_empty())
        })
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IssueFooterError {
    Missing,
    InvalidFormat,
    NotLast,
}

impl IssueFooterError {
    fn message(self) -> &'static str {
        match self {
            Self::Missing => "缺少 `Refs:` 或 `Closes:` footer",
            Self::InvalidFormat => {
                "`Refs:` / `Closes:` footer 格式不正确，应使用 `Refs: #<id>`、`Refs: pending, <reason>` 或 `Closes: #<id>`"
            }
            Self::NotLast => "`Refs:` / `Closes:` footer 必须是提交信息最后一个非空行",
        }
    }
}

fn validate_issue_footer(message: &str) -> Result<(), IssueFooterError> {
    let lines = message.lines().collect::<Vec<_>>();
    let Some(last_non_empty_index) = lines.iter().rposition(|line| !line.trim().is_empty()) else {
        return Err(IssueFooterError::Missing);
    };

    let mut has_issue_footer_candidate = false;
    let mut has_valid_issue_footer_before_last_line = false;

    for (index, line) in lines.iter().enumerate() {
        if !is_issue_footer_candidate(line) {
            continue;
        }

        has_issue_footer_candidate = true;
        if valid_issue_footer_line(line) {
            if index == last_non_empty_index {
                return Ok(());
            }
            has_valid_issue_footer_before_last_line = true;
        }
    }

    if is_issue_footer_candidate(lines[last_non_empty_index]) {
        Err(IssueFooterError::InvalidFormat)
    } else if has_valid_issue_footer_before_last_line {
        Err(IssueFooterError::NotLast)
    } else if has_issue_footer_candidate {
        Err(IssueFooterError::InvalidFormat)
    } else {
        Err(IssueFooterError::Missing)
    }
}

fn valid_refs_footer_line(line: &str) -> bool {
    line.strip_prefix("Refs: ")
        .is_some_and(|value| valid_issue_reference(value) || valid_pending_reason(value))
}

fn valid_closes_footer_line(line: &str) -> bool {
    line.strip_prefix("Closes: ")
        .is_some_and(valid_issue_reference)
}

fn valid_issue_footer_line(line: &str) -> bool {
    valid_refs_footer_line(line) || valid_closes_footer_line(line)
}

fn valid_breaking_change_footer_line(line: &str) -> bool {
    line.strip_prefix("BREAKING CHANGE: ")
        .is_some_and(|description| !description.trim().is_empty())
}

fn breaking_change_footer_count(message: &str) -> usize {
    message
        .lines()
        .filter(|line| line.starts_with("BREAKING CHANGE:"))
        .count()
}

fn has_single_valid_breaking_change_footer(message: &str) -> bool {
    let mut breaking_change_lines = message
        .lines()
        .filter(|line| line.starts_with("BREAKING CHANGE:"));

    let Some(line) = breaking_change_lines.next() else {
        return false;
    };

    breaking_change_lines.next().is_none() && valid_breaking_change_footer_line(line)
}

fn is_issue_footer_candidate(line: &str) -> bool {
    line.starts_with("Refs:") || line.starts_with("Closes:")
}

fn valid_issue_reference(value: &str) -> bool {
    value
        .strip_prefix('#')
        .is_some_and(|digits| !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit()))
}

fn valid_pending_reason(value: &str) -> bool {
    value
        .strip_prefix("pending, ")
        .is_some_and(|reason| !reason.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_MESSAGE: &str = "\
docs(governance): 对齐提交规范

Gate: G3 Candidate
Slice: governance
Impact: core-api=none; data-format=none; adapter-api=none
Scope: 以 Conventional Commits 标题格式重写提交规范
Validation: cargo +1.96.0 test --workspace --locked
Docs: updated

Refs: #23
";

    const BREAKING_MESSAGE: &str = "\
feat(core)!: 调整 tick API

Gate: G3 Candidate
Slice: core-runtime
Impact: core-api=changed; data-format=none; adapter-api=none
Scope: 将 TickInput.delta_time_ms 固化为必填字段
Validation: cargo +1.96.0 test --workspace --locked
Docs: updated

BREAKING CHANGE: TickInput.delta_time_ms 从可选改为必填，调用方必须显式传入 tick 间隔。
Refs: #12
";

    const DELIVERY_G3_URL: &str =
        "https://github.com/illusion-tech/laneflow/pull/61#issuecomment-100";
    const ISSUE_G4_URL: &str =
        "https://github.com/illusion-tech/laneflow/issues/60#issuecomment-200";
    const RELATED_G3_URL: &str =
        "https://github.com/illusion-tech/laneflow/pull/62#issuecomment-300";

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

    fn gate_comment_body(required_fields: &[&str], args: &GateEvidenceArgs) -> String {
        required_fields
            .iter()
            .map(|field| {
                if *field == GATE_ASSERTION_PREFIX {
                    format!(
                        "{GATE_ASSERTION_PREFIX}`{}` 已通过。",
                        expected_gate_command(args, args.phase)
                    )
                } else {
                    (*field).to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn g3_comment_for_args(url: &str, created_at: &str, args: &GateEvidenceArgs) -> GitHubComment {
        GitHubComment {
            url: url.to_string(),
            body: gate_comment_body(G3_COMMENT_FIELDS, args),
            created_at: created_at.to_string(),
        }
    }

    fn g3_comment(url: &str, created_at: &str) -> GitHubComment {
        g3_comment_for_args(url, created_at, &gate_args(GateEvidencePhase::G3))
    }

    fn g4_comment(url: &str, created_at: &str) -> GitHubComment {
        GitHubComment {
            url: url.to_string(),
            body: gate_comment_body(G4_COMMENT_FIELDS, &gate_args(GateEvidencePhase::G4)),
            created_at: created_at.to_string(),
        }
    }

    fn delivery_pr(merged_at: Option<&str>) -> GitHubPullRequest {
        GitHubPullRequest {
            body: format!(
                "- [x] G3 合并判断已记录：[G3 评论]({DELIVERY_G3_URL})\n- G4 回写：[Issue G4 评论]({ISSUE_G4_URL})"
            ),
            state: if merged_at.is_some() {
                "MERGED".to_string()
            } else {
                "OPEN".to_string()
            },
            merged_at: merged_at.map(ToOwned::to_owned),
            closing_issues_references: vec![IssueReference { number: 60 }],
            project_items: vec![ProjectItem {
                title: "LaneFlow".to_string(),
                status: Some(ProjectStatus {
                    name: if merged_at.is_some() {
                        "Done".to_string()
                    } else {
                        "In Review".to_string()
                    },
                }),
            }],
            comments: vec![g3_comment(DELIVERY_G3_URL, "2026-07-10T05:00:00Z")],
        }
    }

    fn issue(state: &str, project_status: &str) -> GitHubIssue {
        GitHubIssue {
            body: format!(
                "- Delivery PR：#61\n- Related PRs：N/A，原因：无部分交付。\n- [x] G0 立项已记录：\n- [x] G1 设计判断已记录：\n- [x] G2 开工判断已记录：\n- [x] G3 合并判断已记录：[Delivery G3 评论]({DELIVERY_G3_URL})\n- [x] G4 完成判断已记录：[G4 评论]({ISSUE_G4_URL})"
            ),
            state: state.to_string(),
            project_items: vec![ProjectItem {
                title: "LaneFlow".to_string(),
                status: Some(ProjectStatus {
                    name: project_status.to_string(),
                }),
            }],
            comments: vec![g4_comment(ISSUE_G4_URL, "2026-07-10T06:00:00Z")],
        }
    }

    fn related_pr(closes_issue: bool) -> GitHubPullRequest {
        let mut args = gate_args(GateEvidencePhase::G3);
        args.related_prs = vec![62];
        GitHubPullRequest {
            body: format!("- [x] G3 合并判断已记录：[G3 评论]({RELATED_G3_URL})\nRefs: #60"),
            state: "OPEN".to_string(),
            merged_at: None,
            closing_issues_references: closes_issue
                .then_some(vec![IssueReference { number: 60 }])
                .unwrap_or_default(),
            project_items: vec![ProjectItem {
                title: "LaneFlow".to_string(),
                status: Some(ProjectStatus {
                    name: "In Review".to_string(),
                }),
            }],
            comments: vec![g3_comment_for_args(
                RELATED_G3_URL,
                "2026-07-10T05:00:00Z",
                &args,
            )],
        }
    }

    fn gate_args(phase: GateEvidencePhase) -> GateEvidenceArgs {
        GateEvidenceArgs {
            phase,
            repo: "illusion-tech/laneflow".to_string(),
            issue: 60,
            delivery_pr: 61,
            related_prs: Vec::new(),
        }
    }

    #[test]
    fn parses_gate_evidence_arguments() {
        let args = vec![
            "g4".to_string(),
            "--repo".to_string(),
            "illusion-tech/laneflow".to_string(),
            "--issue".to_string(),
            "#60".to_string(),
            "--delivery-pr".to_string(),
            "61".to_string(),
            "--related-pr".to_string(),
            "62".to_string(),
        ];

        assert_eq!(
            parse_gate_evidence_args(&args),
            Ok(GateEvidenceArgs {
                phase: GateEvidencePhase::G4,
                repo: "illusion-tech/laneflow".to_string(),
                issue: 60,
                delivery_pr: 61,
                related_prs: vec![62],
            })
        );
    }

    #[test]
    fn deserializes_gh_project_items_with_top_level_title() {
        let pr: GitHubPullRequest = serde_json::from_str(
            r#"{
                "body": "body",
                "state": "MERGED",
                "mergedAt": "2026-07-10T05:30:00Z",
                "closingIssuesReferences": [],
                "projectItems": [{
                    "status": {"optionId": "6114ac6a", "name": "Done"},
                    "title": "LaneFlow"
                }],
                "comments": []
            }"#,
        )
        .expect("current gh pr view projectItems shape should deserialize");

        assert_eq!(pr.project_items[0].title, "LaneFlow");
        assert_eq!(
            pr.project_items[0]
                .status
                .as_ref()
                .map(|status| status.name.as_str()),
            Some("Done")
        );
    }

    #[test]
    fn rejects_duplicate_delivery_and_related_pr() {
        let args = vec![
            "g3".to_string(),
            "--repo".to_string(),
            "illusion-tech/laneflow".to_string(),
            "--issue".to_string(),
            "60".to_string(),
            "--delivery-pr".to_string(),
            "61".to_string(),
            "--related-pr".to_string(),
            "61".to_string(),
        ];

        let error =
            parse_gate_evidence_args(&args).expect_err("delivery PR cannot also be a related PR");

        assert!(error.contains("不能重复"));
    }

    #[test]
    fn accepts_complete_g3_evidence() {
        let issue = issue("OPEN", "In Review");
        let delivery_pr = delivery_pr(None);

        assert!(
            validate_g3_evidence(&gate_args(GateEvidencePhase::G3), &issue, &delivery_pr, &[])
                .is_ok()
        );
    }

    #[test]
    fn g4_invocation_still_validates_pr_comment_as_g3() {
        let issue = issue("OPEN", "In Review");
        let delivery_pr = delivery_pr(None);

        assert!(
            validate_g3_evidence(&gate_args(GateEvidencePhase::G4), &issue, &delivery_pr, &[])
                .is_ok()
        );
    }

    #[test]
    fn rejects_g3_assertion_that_is_still_pending() {
        let issue = issue("OPEN", "In Review");
        let mut delivery_pr = delivery_pr(None);
        delivery_pr.comments[0].body = delivery_pr.comments[0]
            .body
            .replace("` 已通过。", "` 待运行。");

        let error =
            validate_g3_evidence(&gate_args(GateEvidencePhase::G3), &issue, &delivery_pr, &[])
                .expect_err("pending G3 assertion must not pass");

        assert!(error.contains("明确记录 `已通过`"));
    }

    #[test]
    fn rejects_g3_assertion_with_mismatched_command_arguments() {
        let issue = issue("OPEN", "In Review");
        let mut delivery_pr = delivery_pr(None);
        delivery_pr.comments[0].body = delivery_pr.comments[0]
            .body
            .replace("--delivery-pr 61`", "--delivery-pr 99`");

        let error =
            validate_g3_evidence(&gate_args(GateEvidencePhase::G3), &issue, &delivery_pr, &[])
                .expect_err("G3 assertion arguments must match the current invocation");

        assert!(error.contains("命令与当前参数不一致"));
    }

    #[test]
    fn rejects_g3_when_issue_does_not_link_delivery_comment() {
        let mut issue = issue("OPEN", "In Review");
        issue.body = issue.body.replace(DELIVERY_G3_URL, ISSUE_G4_URL);
        let delivery_pr = delivery_pr(None);

        let error =
            validate_g3_evidence(&gate_args(GateEvidencePhase::G3), &issue, &delivery_pr, &[])
                .expect_err("Issue G3 must link the delivery PR G3 comment");

        assert!(error.contains("未回链"));
    }

    #[test]
    fn ignores_acceptance_items_that_start_with_gate_names() {
        let body = format!(
            "- [x] G3/G4 收口流程具有可执行的远端状态断言。\n- [x] G3 合并判断已记录：[Delivery G3 评论]({DELIVERY_G3_URL})"
        );

        assert_eq!(
            completed_gate_permalink(&body, "G3"),
            Ok(DELIVERY_G3_URL.to_string())
        );
    }

    #[test]
    fn rejects_related_pr_that_closes_the_delivery_issue() {
        let mut issue = issue("OPEN", "In Review");
        issue.body = issue
            .body
            .replace(
                DELIVERY_G3_URL,
                &format!("{DELIVERY_G3_URL})，[Related G3 评论]({RELATED_G3_URL}"),
            )
            .replace("Related PRs：N/A，原因：无部分交付。", "Related PRs：#62");
        let mut delivery_pr = delivery_pr(None);
        let related_pr = related_pr(true);
        let mut args = gate_args(GateEvidencePhase::G3);
        args.related_prs = vec![62];
        delivery_pr.comments[0] =
            g3_comment_for_args(DELIVERY_G3_URL, "2026-07-10T05:00:00Z", &args);

        let error = validate_g3_evidence(&args, &issue, &delivery_pr, &[related_pr])
            .expect_err("Related PR cannot close the delivery Issue");

        assert!(error.contains("不得以 closing keyword"));
    }

    #[test]
    fn rejects_related_pr_arguments_that_do_not_match_issue_metadata() {
        let issue = issue("OPEN", "In Review");
        let mut delivery_pr = delivery_pr(None);
        let related_pr = related_pr(false);
        let mut args = gate_args(GateEvidencePhase::G3);
        args.related_prs = vec![62];
        delivery_pr.comments[0] =
            g3_comment_for_args(DELIVERY_G3_URL, "2026-07-10T05:00:00Z", &args);

        let error = validate_g3_evidence(&args, &issue, &delivery_pr, &[related_pr])
            .expect_err("Related PR arguments must match Issue metadata");

        assert!(error.contains("字段与命令参数不一致"));
    }

    #[test]
    fn accepts_complete_g4_evidence() {
        let issue = issue("OPEN", "Done");
        let delivery_pr = delivery_pr(Some("2026-07-10T05:30:00Z"));

        assert!(
            validate_g4_evidence(&gate_args(GateEvidencePhase::G4), &issue, &delivery_pr, &[])
                .is_ok()
        );
    }

    #[test]
    fn rejects_g4_assertion_that_is_still_pending() {
        let mut issue = issue("OPEN", "Done");
        issue.comments[0].body = issue.comments[0]
            .body
            .replace("` 已通过。", "` 待 body 回链后运行。");
        let delivery_pr = delivery_pr(Some("2026-07-10T05:30:00Z"));

        let error =
            validate_g4_evidence(&gate_args(GateEvidencePhase::G4), &issue, &delivery_pr, &[])
                .expect_err("pending G4 assertion must not pass");

        assert!(error.contains("明确记录 `已通过`"));
    }

    #[test]
    fn rejects_g4_assertion_with_mismatched_command_arguments() {
        let mut issue = issue("OPEN", "Done");
        issue.comments[0].body = issue.comments[0]
            .body
            .replace("--delivery-pr 61`", "--delivery-pr 99`");
        let delivery_pr = delivery_pr(Some("2026-07-10T05:30:00Z"));

        let error =
            validate_g4_evidence(&gate_args(GateEvidencePhase::G4), &issue, &delivery_pr, &[])
                .expect_err("G4 assertion arguments must match the current invocation");

        assert!(error.contains("命令与当前参数不一致"));
    }

    #[test]
    fn rejects_g4_comment_created_before_merge() {
        let mut issue = issue("OPEN", "Done");
        issue.comments[0].created_at = "2026-07-10T05:00:00Z".to_string();
        let delivery_pr = delivery_pr(Some("2026-07-10T05:30:00Z"));

        let error =
            validate_g4_evidence(&gate_args(GateEvidencePhase::G4), &issue, &delivery_pr, &[])
                .expect_err("G4 comment must be created after merge");

        assert!(error.contains("早于最后一个关联 PR"));
    }

    #[test]
    fn rejects_g4_when_delivery_pr_is_not_project_done() {
        let issue = issue("OPEN", "Done");
        let mut delivery_pr = delivery_pr(Some("2026-07-10T05:30:00Z"));
        delivery_pr.project_items[0].status = Some(ProjectStatus {
            name: "In Review".to_string(),
        });

        let error =
            validate_g4_evidence(&gate_args(GateEvidencePhase::G4), &issue, &delivery_pr, &[])
                .expect_err("delivery PR must be Project Done before G4");

        assert!(error.contains("Delivery PR 尚未处于 LaneFlow Project 的 Done"));
    }

    #[test]
    fn accepts_lane_flow_commit_message() {
        assert!(validate_message("0123456789abcdef", VALID_MESSAGE).is_empty());
    }

    #[test]
    fn accepts_g3_block_for_a_non_mergeable_branch_record() {
        let message = VALID_MESSAGE.replace("Gate: G3 Candidate", "Gate: G3 Block");

        assert!(validate_message("0123456789abcdef", &message).is_empty());
    }

    #[test]
    fn rejects_g3_block_in_a_commit_range() {
        let message = VALID_MESSAGE.replace("Gate: G3 Candidate", "Gate: G3 Block");

        for gate_policy in [
            CommitGatePolicy::RangeCurrent,
            CommitGatePolicy::RangeLegacyCompatible,
        ] {
            let errors =
                validate_message_with_gate_policy("0123456789abcdef", &message, gate_policy);
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("不得进入合并范围"))
            );
        }
    }

    #[test]
    fn rejects_legacy_gate_for_a_new_commit() {
        let message = VALID_MESSAGE.replace("Gate: G3 Candidate", "Gate: G3 Pass");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("G3 Candidate")));
    }

    #[test]
    fn accepts_legacy_gate_before_the_migration_cutoff() {
        let message = VALID_MESSAGE.replace("Gate: G3 Candidate", "Gate: G3 Pass");

        let errors = validate_message_with_gate_policy(
            "0123456789abcdef",
            &message,
            CommitGatePolicy::RangeLegacyCompatible,
        );

        assert!(errors.is_empty());
    }

    #[test]
    fn switches_to_current_gate_policy_at_the_cutoff() {
        assert_eq!(
            commit_gate_policy_for_timestamp(LEGACY_GATE_CUTOFF_UNIX - 1),
            CommitGatePolicy::RangeLegacyCompatible
        );
        assert_eq!(
            commit_gate_policy_for_timestamp(LEGACY_GATE_CUTOFF_UNIX),
            CommitGatePolicy::RangeCurrent
        );
    }

    #[test]
    fn accepts_commit_message_with_crlf_line_endings() {
        let message = VALID_MESSAGE.replace('\n', "\r\n");

        assert!(validate_message("0123456789abcdef", &message).is_empty());
    }

    #[test]
    fn accepts_commit_message_with_lone_cr_line_endings() {
        let message = VALID_MESSAGE.replace('\n', "\r");

        assert!(validate_message("0123456789abcdef", &message).is_empty());
    }

    #[test]
    fn accepts_breaking_change_with_bang_footer_and_changed_impact() {
        assert!(validate_message("0123456789abcdef", BREAKING_MESSAGE).is_empty());
    }

    #[test]
    fn rejects_breaking_bang_without_breaking_change_footer() {
        let message = BREAKING_MESSAGE.replace(
            "\nBREAKING CHANGE: TickInput.delta_time_ms 从可选改为必填，调用方必须显式传入 tick 间隔。",
            "",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("必须提供")));
    }

    #[test]
    fn rejects_breaking_change_footer_without_bang() {
        let message = BREAKING_MESSAGE.replace("feat(core)!:", "feat(core):");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(
            errors
                .iter()
                .any(|error| error.contains("必须与标题 `!` 同时使用"))
        );
    }

    #[test]
    fn rejects_breaking_change_with_unchanged_impact() {
        let message = BREAKING_MESSAGE.replace(
            "Impact: core-api=changed; data-format=none; adapter-api=none",
            "Impact: core-api=none; data-format=none; adapter-api=none",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(
            errors
                .iter()
                .any(|error| error.contains("至少一项标为 changed"))
        );
    }

    #[test]
    fn rejects_empty_breaking_change_footer() {
        let message = BREAKING_MESSAGE.replace(
            "BREAKING CHANGE: TickInput.delta_time_ms 从可选改为必填，调用方必须显式传入 tick 间隔。",
            "BREAKING CHANGE: ",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("格式不正确")));
    }

    #[test]
    fn rejects_breaking_change_after_issue_footer() {
        let message = BREAKING_MESSAGE.replace(
            "BREAKING CHANGE: TickInput.delta_time_ms 从可选改为必填，调用方必须显式传入 tick 间隔。\nRefs: #12",
            "Refs: #12\nBREAKING CHANGE: TickInput.delta_time_ms 从可选改为必填，调用方必须显式传入 tick 间隔。",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("最后一个非空行")));
    }

    #[test]
    fn rejects_multiple_breaking_change_footers() {
        let message = BREAKING_MESSAGE.replace(
            "BREAKING CHANGE: TickInput.delta_time_ms 从可选改为必填，调用方必须显式传入 tick 间隔。",
            "BREAKING CHANGE: TickInput.delta_time_ms 从可选改为必填，调用方必须显式传入 tick 间隔。\nBREAKING CHANGE: 第二条破坏性说明",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("格式不正确")));
    }

    #[test]
    fn accepts_valid_commit_message_file() {
        let path = temp_message_path("valid");
        std::fs::write(&path, VALID_MESSAGE).expect("test commit message should be written");

        let result = check_commit_message_file(path.to_str().expect("path should be UTF-8"));

        std::fs::remove_file(&path).expect("test commit message should be removed");
        assert!(result.is_ok());
    }

    #[test]
    fn accepts_valid_commit_message_file_with_git_comments() {
        let path = temp_message_path("valid-with-comments");
        let message = format!(
            "{VALID_MESSAGE}\n# Please enter the commit message for your changes.\n  # On branch docs/23-conventional-commits\n# Changes to be committed:\n"
        );
        std::fs::write(&path, message).expect("test commit message should be written");

        let result = check_commit_message_file(path.to_str().expect("path should be UTF-8"));

        std::fs::remove_file(&path).expect("test commit message should be removed");
        assert!(result.is_ok());
    }

    #[test]
    fn rejects_non_comment_line_after_issue_footer_in_commit_message_file() {
        let path = temp_message_path("invalid-after-footer");
        let message = VALID_MESSAGE.replace(
            "Refs: #23\n",
            "Refs: #23\nnot a Git comment\n# This comment should be ignored\n",
        );
        std::fs::write(&path, message).expect("test commit message should be written");

        let result = check_commit_message_file(path.to_str().expect("path should be UTF-8"));

        std::fs::remove_file(&path).expect("test commit message should be removed");
        let error = result.expect_err("non-comment content after issue footer should fail");
        assert!(error.contains("最后一个非空行"));
    }

    #[test]
    fn rejects_invalid_commit_message_file() {
        let path = temp_message_path("invalid");
        std::fs::write(&path, "update files\n").expect("test commit message should be written");

        let result = check_commit_message_file(path.to_str().expect("path should be UTF-8"));

        std::fs::remove_file(&path).expect("test commit message should be removed");
        let error = result.expect_err("invalid commit message should fail");
        assert!(error.contains("标题不符合 Conventional Commits"));
    }

    #[test]
    fn rejects_missing_blank_line_after_title() {
        let message = VALID_MESSAGE.replace(
            "docs(governance): 对齐提交规范\n\nGate: G3 Candidate",
            "docs(governance): 对齐提交规范\nGate: G3 Candidate",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("连续治理字段块")));
    }

    #[test]
    fn rejects_blank_line_between_governance_fields() {
        let message = VALID_MESSAGE.replace(
            "Gate: G3 Candidate\nSlice: governance",
            "Gate: G3 Candidate\n\nSlice: governance",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("连续治理字段块")));
    }

    #[test]
    fn rejects_governance_fields_out_of_order() {
        let message = VALID_MESSAGE.replace(
            "Gate: G3 Candidate\nSlice: governance",
            "Slice: governance\nGate: G3 Candidate",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("连续治理字段块")));
    }

    #[test]
    fn rejects_missing_blank_line_before_issue_footer() {
        let message =
            VALID_MESSAGE.replace("Docs: updated\n\nRefs: #23", "Docs: updated\nRefs: #23");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("连续治理字段块")));
    }

    #[test]
    fn rejects_extra_blank_line_before_issue_footer() {
        let message =
            VALID_MESSAGE.replace("Docs: updated\n\nRefs: #23", "Docs: updated\n\n\nRefs: #23");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("连续治理字段块")));
    }

    #[test]
    fn rejects_legacy_title_and_type_field() {
        let message = VALID_MESSAGE
            .replace("docs(governance): 对齐提交规范", "LF-23: 对齐提交规范")
            .replace("Slice: governance", "Type: governance");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("标题不符合")));
        assert!(errors.iter().any(|error| error.contains("`Slice`")));
    }

    #[test]
    fn rejects_required_field_without_space_after_colon() {
        let message = VALID_MESSAGE.replace("Gate: G3 Candidate", "Gate:G3 Candidate");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("Gate: ")));
    }

    #[test]
    fn rejects_slice_without_space_after_colon() {
        let message = VALID_MESSAGE.replace("Slice: governance", "Slice:governance");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("Slice")));
    }

    #[test]
    fn rejects_impact_without_separator_space() {
        let message = VALID_MESSAGE.replace(
            "Impact: core-api=none; data-format=none; adapter-api=none",
            "Impact: core-api=none;data-format=none; adapter-api=none",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("`Impact`")));
    }

    #[test]
    fn rejects_impact_without_space_after_colon() {
        let message = VALID_MESSAGE.replace(
            "Impact: core-api=none; data-format=none; adapter-api=none",
            "Impact:core-api=none; data-format=none; adapter-api=none",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("Impact")));
    }

    #[test]
    fn rejects_impact_fields_out_of_order() {
        let message = VALID_MESSAGE.replace(
            "Impact: core-api=none; data-format=none; adapter-api=none",
            "Impact: data-format=none; core-api=none; adapter-api=none",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("`Impact`")));
    }

    #[test]
    fn rejects_impact_with_missing_field() {
        let message = VALID_MESSAGE.replace(
            "Impact: core-api=none; data-format=none; adapter-api=none",
            "Impact: core-api=none; data-format=none",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("`Impact`")));
    }

    #[test]
    fn rejects_impact_with_extra_field() {
        let message = VALID_MESSAGE.replace(
            "Impact: core-api=none; data-format=none; adapter-api=none",
            "Impact: core-api=none; data-format=none; adapter-api=none; docs=changed",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("`Impact`")));
    }

    #[test]
    fn rejects_impact_with_invalid_value() {
        let message = VALID_MESSAGE.replace(
            "Impact: core-api=none; data-format=none; adapter-api=none",
            "Impact: core-api=maybe; data-format=none; adapter-api=none",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("`Impact`")));
    }

    #[test]
    fn accepts_docs_not_required() {
        let message = VALID_MESSAGE.replace("Docs: updated", "Docs: not required");

        assert!(validate_message("0123456789abcdef", &message).is_empty());
    }

    #[test]
    fn accepts_docs_pending_reason() {
        let message = VALID_MESSAGE.replace("Docs: updated", "Docs: pending 后续由 #25 跟踪补齐");

        assert!(validate_message("0123456789abcdef", &message).is_empty());
    }

    #[test]
    fn rejects_docs_pending_without_reason() {
        let message = VALID_MESSAGE.replace("Docs: updated", "Docs: pending");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("`Docs`")));
    }

    #[test]
    fn rejects_docs_unknown_value() {
        let message = VALID_MESSAGE.replace("Docs: updated", "Docs: maybe");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("`Docs`")));
    }

    #[test]
    fn rejects_non_numeric_issue_reference() {
        let message = VALID_MESSAGE.replace("Refs: #23", "Refs: #abc");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("格式不正确")));
    }

    #[test]
    fn accepts_pending_issue_reason() {
        let message = VALID_MESSAGE.replace(
            "Refs: #23",
            "Refs: pending, initial repository governance bootstrap",
        );

        assert!(validate_message("0123456789abcdef", &message).is_empty());
    }

    #[test]
    fn rejects_pending_without_reason() {
        let message = VALID_MESSAGE.replace("Refs: #23", "Refs: pending");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("格式不正确")));
    }

    #[test]
    fn rejects_pending_without_space_after_comma() {
        let message = VALID_MESSAGE.replace("Refs: #23", "Refs: pending,missing space");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("格式不正确")));
    }

    #[test]
    fn accepts_closes_issue_reference() {
        let message = VALID_MESSAGE.replace("Refs: #23", "Closes: #23");

        assert!(validate_message("0123456789abcdef", &message).is_empty());
    }

    #[test]
    fn rejects_closes_pending_reason() {
        let message = VALID_MESSAGE.replace(
            "Refs: #23",
            "Closes: pending, initial repository governance bootstrap",
        );

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("格式不正确")));
    }

    #[test]
    fn rejects_missing_issue_footer() {
        let message = VALID_MESSAGE.replace("\nRefs: #23\n", "\n");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("缺少 `Refs:`")));
    }

    #[test]
    fn rejects_issue_reference_outside_footer_block() {
        let message =
            VALID_MESSAGE.replace("Refs: #23\n", "Refs: #23\n\nNote: footer must stay last\n");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("最后一个非空行")));
    }

    #[test]
    fn rejects_issue_reference_followed_by_non_empty_footer_line() {
        let message =
            VALID_MESSAGE.replace("Refs: #23\n", "Refs: #23\nNote: footer must stay last\n");

        let errors = validate_message("0123456789abcdef", &message);

        assert!(errors.iter().any(|error| error.contains("最后一个非空行")));
    }

    #[test]
    fn accepts_breaking_change_bang() {
        assert!(valid_conventional_title("feat(core)!: 调整 tick API"));
    }

    #[test]
    fn accepts_dependabot_dependency_commit_without_governance_body() {
        assert!(is_allowed_dependabot_commit(
            DEPENDABOT_AUTHOR_NAME,
            DEPENDABOT_AUTHOR_EMAIL,
            "build(deps): bump serde from 1.0.227 to 1.0.228\n"
        ));
    }

    #[test]
    fn rejects_human_commit_that_mimics_dependabot_title() {
        assert!(!is_allowed_dependabot_commit(
            "LaneFlow Maintainer",
            "maintainer@example.com",
            "build(deps): bump serde from 1.0.227 to 1.0.228\n"
        ));
    }

    #[test]
    fn rejects_dependabot_commit_outside_dependency_scope() {
        assert!(!is_allowed_dependabot_commit(
            DEPENDABOT_AUTHOR_NAME,
            DEPENDABOT_AUTHOR_EMAIL,
            "fix(core): change runtime behavior\n"
        ));
    }

    #[test]
    fn extracts_top_level_push_event_payload_fields() {
        let payload = r#"{
            "head_commit": {
                "message": "commit message mentions \"before\" and \"after\""
            },
            "commits": [
                {
                    "before": "nested-before",
                    "after": "nested-after"
                }
            ],
            "before": "abc",
            "after": "def"
        }"#;
        let event = push_event_payload(payload).expect("payload should parse");

        assert_eq!(event.before.as_deref(), Some("abc"));
        assert_eq!(event.after.as_deref(), Some("def"));
    }

    #[test]
    fn local_run_requires_explicit_commit_range() {
        let error = commit_range_from_event(None, None, None, None).unwrap_err();

        assert!(error.contains("显式传入 commit rev-range"));
    }

    #[test]
    fn unsupported_event_requires_explicit_commit_range() {
        let error = commit_range_from_event(Some("workflow_dispatch"), None, Some("def"), None)
            .unwrap_err();

        assert!(error.contains("显式传入 rev-range"));
    }

    #[test]
    fn pull_request_event_uses_base_branch_range() {
        let range =
            commit_range_from_event(Some("pull_request"), Some("main"), Some("def"), None).unwrap();

        assert_eq!(range, "origin/main..HEAD");
    }

    #[test]
    fn push_event_uses_before_after_range() {
        let payload = r#"{
            "head_commit": {
                "message": "commit message mentions \"before\" and \"after\""
            },
            "nested": {
                "before": "nested-before",
                "after": "nested-after"
            },
            "before": "abc",
            "after": "def"
        }"#;

        let range = commit_range_from_event(Some("push"), None, None, Some(payload)).unwrap();

        assert_eq!(range, "abc..def");
    }

    #[test]
    fn push_event_with_zero_before_checks_tip_commit_only() {
        let payload = r#"{"before":"0000000000000000000000000000000000000000","after":"def"}"#;

        let range = commit_range_from_event(Some("push"), None, None, Some(payload)).unwrap();

        assert_eq!(range, "def^!");
    }

    #[test]
    fn push_event_without_payload_checks_github_sha_only() {
        let range = commit_range_from_event(Some("push"), None, Some("def"), None).unwrap();

        assert_eq!(range, "def^!");
    }

    fn temp_message_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("laneflow-xtask-{name}-{}.txt", std::process::id()))
    }
}
