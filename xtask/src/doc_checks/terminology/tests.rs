use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Repo(PathBuf);

impl Repo {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "laneflow-terms-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        let repo = Self(path);
        repo.put(
            "docs/reference/terms/catalog.toml",
            &manifest(&["one.toml"]),
        );
        repo.put(
            "docs/reference/terms/one.toml",
            &source(&term("one", "概念", "concept", "first")),
        );
        repo.put(
            GLOSSARY,
            &page("reference-glossary", "reference", "active", ""),
        );
        repo.put(
            WORDING,
            &page("reference-wording-guide", "reference", "active", ""),
        );
        repo.put(
            "docs/design/owner.md",
            &page(
                "design-owner",
                "design",
                "review",
                "<a id=\"contract\"></a>\n## 合同\n\n说明。\n",
            ),
        );
        repo
    }
    fn put(&self, path: &str, value: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, value).unwrap();
    }
    fn documents(&self) -> Catalog {
        Catalog::load(&self.0).unwrap()
    }
    fn load(&self) -> Result<TermCatalog, String> {
        TermCatalog::load(&self.0, &self.documents())
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn manifest(files: &[&str]) -> String {
    format!(
        "schema_version = 1\nfiles = {files:?}\n[[domains]]\nid = \"first\"\nname = \"第一领域\"\n[[domains]]\nid = \"second\"\nname = \"第二领域\"\n"
    )
}

fn page(id: &str, kind: &str, status: &str, body: &str) -> String {
    format!(
        "+++\nschema_version = 1\nid = \"{id}\"\nkind = \"{kind}\"\nstatus = \"{status}\"\nsummary = \"说明\"\nscope = \"测试\"\nrevised_date = \"2026-11-01\"\n+++\n\n# 页面\n\n{body}\n{BEGIN}\n\n{END}\n"
    )
}

fn term(id: &str, name: &str, english: &str, domain: &str) -> String {
    format!(
        "[[terms]]\nid = \"{id}\"\nname = \"{name}\"\nenglish_aliases = [\"{english}\"]\ndomains = [\"{domain}\"]\ndefinition = \"具有明确边界的概念。\"\nreferences = [{{ document = \"design-owner\", anchor = \"contract\" }}]\n"
    )
}

fn source(records: &str) -> String {
    format!("schema_version = 1\n{records}")
}

fn rename(id: &str, name: &str, target: &str) -> String {
    format!(
        "[[name_changes]]\nid = \"{id}\"\nnames = [\"{name}\"]\ndomains = [\"first\"]\nscope = \"规定语境。\"\ninclude_compounds = true\nreplacements = [{{ target = \"{target}\", condition = \"符合该语境时。\" }}]\n"
    )
}

#[test]
fn rejects_schema_errors_at_all_object_levels() {
    let repo = Repo::new();
    let good = source(&term("one", "概念", "concept", "first"));
    for invalid in [
        good.replace("schema_version = 1", "schema_version = 2"),
        good.replace(
            "schema_version = 1",
            "schema_version = 1\nschema_version = 1",
        ),
        format!("{good}status = \"accepted\"\n"),
        format!("{good}implementation_status = \"done\"\n"),
        format!("{good}symbols = [{{ value = \"Type\", kind = \"unknown\" }}]\n"),
        format!("{good}symbols = [{{ value = \"Type\", kind = \"type\", status = \"done\" }}]\n"),
        format!("{good}short_forms = [{{ name = \"简称\", unknown = true }}]\n"),
        good.replace(
            "anchor = \"contract\"",
            "anchor = \"contract\", extra = true",
        ),
        format!("{good}retirement = {{ reason = \"停止使用。\", unknown = true }}\n"),
        good.replace("english_aliases = [\"concept\"]", "english_aliases = [42]"),
        good.replace("definition =", "definition = \"重复\"\ndefinition ="),
    ] {
        repo.put("docs/reference/terms/one.toml", &invalid);
        assert!(repo.load().is_err(), "{invalid}");
    }
}

#[test]
fn requires_complete_safe_manifest_and_plain_files() {
    let repo = Repo::new();
    for files in [
        vec!["../one.toml"],
        vec!["one.toml", "one.toml"],
        vec!["missing.toml"],
        vec!["catalog.toml"],
        vec![],
    ] {
        repo.put("docs/reference/terms/catalog.toml", &manifest(&files));
        assert!(repo.load().is_err(), "{files:?}");
    }
    repo.put(
        "docs/reference/terms/catalog.toml",
        &manifest(&["one.toml"]),
    );
    repo.put("docs/reference/terms/unlisted.toml", "schema_version = 1");
    assert!(repo.load().unwrap_err().contains("files"));
    fs::remove_file(repo.0.join("docs/reference/terms/unlisted.toml")).unwrap();
    fs::create_dir(repo.0.join("docs/reference/terms/nested")).unwrap();
    assert!(repo.load().is_err());
}

#[test]
fn duplicate_ids_fail_across_files_and_record_kinds() {
    let repo = Repo::new();
    repo.put(
        "docs/reference/terms/catalog.toml",
        &manifest(&["one.toml", "two.toml"]),
    );
    repo.put(
        "docs/reference/terms/two.toml",
        &source(&term("one", "其他概念", "another", "second")),
    );
    assert!(repo.load().unwrap_err().contains("重复术语记录 ID"));
    repo.put(
        "docs/reference/terms/two.toml",
        &source(&rename("one", "旧名", "one")),
    );
    assert!(repo.load().unwrap_err().contains("重复术语记录 ID"));
}

#[test]
fn label_collisions_are_scoped_and_symbols_need_not_be_unique() {
    let repo = Repo::new();
    let first = term("one", "概念", "Concept", "first");
    let second = term("two", "另一概念", "concept", "first");
    repo.put(
        "docs/reference/terms/one.toml",
        &source(&(first.clone() + &second)),
    );
    assert!(repo.load().unwrap_err().contains("名称冲突"));
    repo.put(
        "docs/reference/terms/one.toml",
        &source(&(first.clone() + &second.replace("[\"first\"]", "[\"second\"]"))),
    );
    assert!(repo.load().is_ok());
    let shared = "symbols = [{ value = \"Shared\", kind = \"type\" }]\n";
    repo.put(
        "docs/reference/terms/one.toml",
        &source(&(first + shared + &second + "context = \"other\"\n" + shared)),
    );
    assert!(repo.load().is_ok());
}

#[test]
fn short_forms_and_former_names_cannot_claim_another_active_name() {
    let repo = Repo::new();
    let first = term("one", "概念", "concept", "first");
    for tail in [
        term("two", "另一概念", "other", "first")
            + "short_forms = [{ name = \"概念\", scope = \"相同领域。\" }]\n",
        rename("former", "概念", "one"),
    ] {
        repo.put(
            "docs/reference/terms/one.toml",
            &source(&(first.clone() + &tail)),
        );
        assert!(repo.load().unwrap_err().contains("名称冲突"));
    }
}

#[test]
fn references_and_related_concepts_must_resolve() {
    let repo = Repo::new();
    let good = source(&term("one", "概念", "concept", "first"));
    for invalid in [
        good.replace("design-owner", "missing"),
        good.replace("anchor = \"contract\"", "anchor = \"missing\""),
        good.replace("anchor = \"contract\"", "anchor = \"#contract\""),
        format!("{good}related = [\"one\"]\n"),
        format!("{good}related = [\"missing\"]\n"),
        good.replace("design-owner", "reference-glossary"),
    ] {
        repo.put("docs/reference/terms/one.toml", &invalid);
        assert!(repo.load().is_err(), "{invalid}");
    }
}

#[test]
fn redirects_must_terminate_at_a_current_concept() {
    let repo = Repo::new();
    let first = term("one", "概念", "concept", "first");
    for tail in [
        rename("former", "旧名", "missing"),
        rename("former", "旧名", "former"),
        rename("former", "旧名", "other") + &rename("other", "另一旧名", "former"),
    ] {
        repo.put(
            "docs/reference/terms/one.toml",
            &source(&(first.clone() + &tail)),
        );
        assert!(repo.load().is_err());
    }
    repo.put(
        "docs/reference/terms/one.toml",
        &source(
            &(first.clone()
                + &rename("former", "旧名", "middle")
                + &rename("middle", "中间名", "one")),
        ),
    );
    assert!(repo.load().is_ok());
    repo.put(
        "docs/reference/terms/one.toml",
        &source(
            &(first
                + "retirement = { reason = \"停止使用。\" }\n"
                + &rename("former", "旧名", "one")),
        ),
    );
    assert!(repo.load().unwrap_err().contains("已退役"));
}

#[test]
fn retired_concepts_need_no_invented_replacement() {
    let repo = Repo::new();
    repo.put(
        "docs/reference/terms/one.toml",
        &source(
            &(term("one", "历史概念", "historical", "first")
                + "retirement = { reason = \"该方案已经取消。\" }\n"),
        ),
    );
    let catalog = repo.load().unwrap();
    let rendered = catalog.glossary(&repo.documents());
    assert!(
        rendered.find("<a id=\"retired-concepts\"").unwrap()
            < rendered.find("<a id=\"term-one\"").unwrap()
    );
    assert!(
        catalog.terms[0]
            .record
            .retirement
            .as_ref()
            .unwrap()
            .replacements
            .is_empty()
    );
}

#[test]
fn borrowed_names_require_a_first_use_gloss_and_keep_original_spelling() {
    let repo = Repo::new();
    let base = term("fixture", "fixture", "fixture", "first")
        .replace("english_aliases = [\"fixture\"]\n", "");
    repo.put("docs/reference/terms/one.toml", &source(&base));
    assert!(repo.load().is_err());
    repo.put(
        "docs/reference/terms/one.toml",
        &source(&(base + "borrowed = { language = \"en\", first_use_gloss = \"固定测试环境\" }\n")),
    );
    let catalog = repo.load().unwrap();
    assert!(catalog.glossary(&repo.documents()).contains("### fixture"));
}

#[test]
fn rejects_empty_duplicate_and_block_markup_fields() {
    let repo = Repo::new();
    let good = source(&term("one", "概念", "concept", "first"));
    for value in [
        "",
        " leading",
        "# heading",
        "[link](https://example.com)",
        "<script>run()</script>",
        "**emphasis**",
        "~~removed~~",
    ] {
        repo.put(
            "docs/reference/terms/one.toml",
            &good.replace("具有明确边界的概念。", value),
        );
        assert!(repo.load().is_err(), "{value}");
    }
    for extra in [
        "scope_notes = [\"note\", \"note\"]",
        "short_forms = [{ name = \"简称\", scope = \"\" }]",
        "symbols = [{ value = \"X\", kind = \"type\" }, { value = \"X\", kind = \"type\" }]",
    ] {
        repo.put("docs/reference/terms/one.toml", &format!("{good}{extra}\n"));
        assert!(repo.load().is_err());
    }
}

#[test]
fn terminology_does_not_infer_design_acceptance_or_implementation() {
    let repo = Repo::new();
    let catalog = repo.load().unwrap();
    let value = serde_json::to_value(&catalog).unwrap();
    let entry = &value["terms"][0];
    assert_eq!(entry["references"][0]["document"], "design-owner");
    assert_eq!(entry["url"], "/docs/reference-glossary/#term-one");
    assert!(entry.get("status").is_none());
    assert!(entry.get("implementation_status").is_none());
    assert_eq!(
        repo.documents()
            .pages
            .iter()
            .find(|p| p.metadata.id == "design-owner")
            .unwrap()
            .metadata
            .status,
        crate::doc_checks::metadata::Status::Review
    );
    assert!(!catalog.glossary(&repo.documents()).contains("已实现"));
}

#[test]
fn stable_ids_and_urls_survive_names_and_source_file_moves() {
    let repo = Repo::new();
    let before = repo.load().unwrap();
    repo.put(
        "docs/reference/terms/catalog.toml",
        &manifest(&["renamed.toml"]),
    );
    fs::remove_file(repo.0.join("docs/reference/terms/one.toml")).unwrap();
    repo.put(
        "docs/reference/terms/renamed.toml",
        &source(&term("one", "新名称", "new label", "second")),
    );
    let after = repo.load().unwrap();
    assert_eq!(before.terms[0].url, after.terms[0].url);
    assert_ne!(before.terms[0].source, after.terms[0].source);
}

#[test]
fn multi_domain_concepts_have_one_definition_and_cross_links() {
    let repo = Repo::new();
    let data = source(&term("one", "概念", "concept", "first"))
        .replace("domains = [\"first\"]", "domains = [\"first\", \"second\"]");
    repo.put("docs/reference/terms/one.toml", &data);
    let rendered = repo.load().unwrap().glossary(&repo.documents());
    assert_eq!(rendered.matches("具有明确边界的概念。").count(), 1);
    assert!(rendered.contains("[概念][term-one]"));
}

#[test]
fn generation_is_read_only_when_checking_and_preserves_revision_date() {
    let repo = Repo::new();
    let documents = repo.documents();
    let catalog = repo.load().unwrap();
    let original = fs::read_to_string(repo.0.join(GLOSSARY)).unwrap();
    assert!(catalog.pages(&repo.0, &documents, false).is_err());
    assert_eq!(fs::read_to_string(repo.0.join(GLOSSARY)).unwrap(), original);
    catalog.pages(&repo.0, &documents, true).unwrap();
    let expected = fs::read_to_string(repo.0.join(GLOSSARY)).unwrap();
    assert!(expected.contains("revised_date = \"2026-11-01\""));
    catalog.pages(&repo.0, &repo.documents(), false).unwrap();
    repo.put(GLOSSARY, &expected.replace('\n', "\r\n"));
    catalog.pages(&repo.0, &repo.documents(), false).unwrap();
    repo.put(
        "docs/reference/terms/one.toml",
        &source(&term("one", "概念", "concept", "first")).replace('\n', "\r\n"),
    );
    assert_eq!(
        serde_json::to_value(repo.load().unwrap()).unwrap(),
        serde_json::to_value(catalog).unwrap()
    );
}

#[test]
fn all_markers_are_validated_before_any_page_write() {
    let repo = Repo::new();
    let original = fs::read_to_string(repo.0.join(GLOSSARY)).unwrap();
    repo.put(
        WORDING,
        &page("reference-wording-guide", "reference", "active", "").replace(END, ""),
    );
    let catalog = repo.load().unwrap();
    assert!(catalog.pages(&repo.0, &repo.documents(), true).is_err());
    assert_eq!(fs::read_to_string(repo.0.join(GLOSSARY)).unwrap(), original);
    for malformed in [
        format!("{END}\n{BEGIN}\n"),
        format!("{BEGIN}\n{END}\nafter"),
        format!("{BEGIN}\n{BEGIN}\n{END}\n"),
        format!("```\n{BEGIN}\n{END}\n"),
        format!("    {BEGIN}\n{END}\n"),
    ] {
        assert!(replace_region(&malformed, "body").is_err());
    }
    assert_eq!(
        replace_region(&format!("{BEGIN}\n{END}"), "body").unwrap(),
        format!("{BEGIN}\n\nbody\n{END}\n")
    );
}

#[test]
fn deterministic_order_does_not_depend_on_manifest_file_order() {
    let repo = Repo::new();
    repo.put(
        "docs/reference/terms/two.toml",
        &source(&term("two", "另一概念", "other", "second")),
    );
    repo.put(
        "docs/reference/terms/catalog.toml",
        &manifest(&["one.toml", "two.toml"]),
    );
    let before = repo.load().unwrap();
    repo.put(
        "docs/reference/terms/catalog.toml",
        &manifest(&["two.toml", "one.toml"]),
    );
    let after = repo.load().unwrap();
    assert_eq!(
        before.glossary(&repo.documents()),
        after.glossary(&repo.documents())
    );
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(after).unwrap()
    );
}

#[test]
fn render_escapes_names_and_uses_existing_markdown_link_rules() {
    let repo = Repo::new();
    let data = source(&term(
        "one",
        "<Name> [label] * ~~value~~",
        "concept",
        "first",
    )) + "symbols = [{ value = \"Vec<T> `literal`\", kind = \"type\" }]\n";
    repo.put("docs/reference/terms/one.toml", &data);
    let catalog = repo.load().unwrap();
    catalog.pages(&repo.0, &repo.documents(), true).unwrap();
    let generated = fs::read_to_string(repo.0.join(GLOSSARY)).unwrap();
    let parsed = crate::doc_checks::markdown::parse_document(&generated, GLOSSARY);
    assert!(parsed.anchors.contains("term-one"));
    assert!(parsed.duplicate_anchors.is_empty());
    assert!(parsed.links.iter().all(|l| !matches!(
        l.kind,
        crate::doc_checks::markdown::LinkKind::Failure { .. }
    )));
    assert!(!generated.contains("### <Name>"));
    assert!(
        parsed
            .headings
            .iter()
            .any(|h| h.text == "<Name> [label] * ~~value~~")
    );
}

#[cfg(unix)]
#[test]
fn refuses_symlinked_terminology_sources() {
    use std::os::unix::fs::symlink;
    let repo = Repo::new();
    repo.put(
        "outside.toml",
        &source(&term("two", "其他", "other", "second")),
    );
    fs::remove_file(repo.0.join("docs/reference/terms/one.toml")).unwrap();
    symlink(
        repo.0.join("outside.toml"),
        repo.0.join("docs/reference/terms/one.toml"),
    )
    .unwrap();
    assert!(repo.load().is_err());
}
