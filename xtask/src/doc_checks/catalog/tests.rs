use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;
use crate::doc_checks::metadata::{date, parse};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Repo(PathBuf);
impl Repo {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "laneflow-doc-catalog-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(path.join("docs")).unwrap();
        Self(path)
    }
    fn put(&self, path: &str, text: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fn load(&self) -> Result<Catalog, String> {
        Catalog::load(&self.0)
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn doc(id: &str, status: &str, extra: &str) -> String {
    format!(
        "+++\nschema_version = 1\nid = \"{id}\"\nkind = \"design\"\nstatus = \"{status}\"\nsummary = \"规范说明\"\nscope = \"测试合同\"\n{extra}\n+++\n\n# 测试 `contract`\n\n<a id=\"replacement\"></a>\n## 取代范围\n\n仅替代所列条款。\n"
    )
}
fn edge(target: &str, extent: &str) -> String {
    format!(
        "[[supersedes]]\ntarget = \"{target}\"\nextent = \"{extent}\"\nexplanation = \"#replacement\"\n"
    )
}

#[test]
fn rejects_duplicate_unknown_and_wrong_types_at_every_level() {
    for extra in [
        "status = \"active\"",
        "unknown = true",
        "decision_date = 2026-11-17",
        "related = [7]",
        "issues = [\"#42\"]",
        "scope = \"\"",
        "schema_version = 2",
    ] {
        assert!(parse(&doc("one", "accepted", extra)).is_err(), "{extra}");
    }
    assert!(
        parse(&doc(
            "one",
            "accepted",
            &(edge("two", "partial") + "status = \"accepted\"")
        ))
        .is_err()
    );
    assert!(parse(&doc("one", "accepted", "decision_date = \"2026-02-29\"")).is_err());
    assert!(parse(&doc("one", "accepted", "related = [\"two\", \"two\"]")).is_err());
    assert!(parse(&doc("one", "accepted", "verified_date = \"2026-11-17\"")).is_err());
}

#[test]
fn dates_are_calendar_dates_not_just_patterns() {
    for good in ["2000-02-29", "2024-02-29", "2026-11-17"] {
        assert!(date(good));
    }
    for bad in [
        "1900-02-29",
        "2026-02-29",
        "2026-04-31",
        "0000-01-01",
        "2026-13-01",
        "2026-01-00",
        "2026-1-01",
        "２０２６-01-01",
    ] {
        assert!(!date(bad), "{bad}");
    }
}

#[test]
fn revision_date_is_optional_validated_and_exported_without_date_inference() {
    let absent = parse(&doc(
        "one",
        "accepted",
        "date = \"2026-11-01\"\ndecision_date = \"2026-11-17\"",
    ))
    .unwrap();
    assert!(absent.revised_date.is_none());
    assert!(
        serde_json::to_value(absent)
            .unwrap()
            .get("revised_date")
            .is_none()
    );
    for value in ["2026-02-29", "2026-11-17（修订）", "", "2026-1-17"] {
        assert!(
            parse(&doc(
                "one",
                "accepted",
                &format!("revised_date = \"{value}\"")
            ))
            .is_err()
        );
    }
    assert!(parse(&doc("one", "accepted", "revised_date = 2026-11-17")).is_err());
    assert!(
        parse(&doc(
            "one",
            "accepted",
            "revised_date = \"2026-11-16\"\nrevised_date = \"2026-11-17\""
        ))
        .is_err()
    );
    let metadata = parse(&doc("one", "accepted", "revised_date = \"2026-11-16\"\ndecision_date = \"2026-11-17\"\nverified_date = \"2026-11-01\"\nverified_revision = \"0123456789abcdef0123456789abcdef01234567\"")).unwrap();
    let exported = serde_json::to_value(metadata).unwrap();
    assert_eq!(exported["revised_date"], "2026-11-16");
    assert_eq!(exported["decision_date"], "2026-11-17");
    assert_eq!(exported["verified_date"], "2026-11-01");
}

#[test]
fn scalar_validation_rejects_invalid_identity_status_scope_and_revision() {
    let valid = doc("one", "accepted", "");
    for (from, to) in [
        ("schema_version = 1", "schema_version = 2"),
        ("id = \"one\"", "id = \"One\""),
        ("id = \"one\"", "id = \"one--two\""),
        ("kind = \"design\"", "kind = \"reference\""),
        ("status = \"accepted\"", "status = \"accepted (done)\""),
        ("scope = \"测试合同\"", "scope = \" \""),
        ("scope = \"测试合同\"", ""),
        ("summary = \"规范说明\"", "summary = 7"),
    ] {
        assert!(parse(&valid.replace(from, to)).is_err(), "{to}");
    }
    for revision in [
        "abc",
        "0123456789ABCDEF0123456789ABCDEF01234567",
        "gggggggggggggggggggggggggggggggggggggggg",
    ] {
        assert!(
            parse(&doc(
                "one",
                "accepted",
                &format!("verified_date = \"2026-11-17\"\nverified_revision = \"{revision}\"")
            ))
            .is_err()
        );
    }
    assert!(parse(&doc("one", "active", "decision_date = \"2026-11-17\"")).is_err());
    parse(&doc("one", "accepted", "verified_date = \"2026-11-17\"\nverified_revision = \"0123456789abcdef0123456789abcdef01234567\"")).unwrap();
}

#[test]
fn header_is_not_markdown_and_crlf_preserves_diagnostic_lines() {
    let text = doc("one", "accepted", "related = [\"two\"]");
    let lf = markdown::parse_document(&text, "docs/one.md");
    let crlf = markdown::parse_document(&text.replace('\n', "\r\n"), "docs/one.md");
    assert_eq!(lf.titles, vec!["测试 contract"]);
    assert_eq!(lf.anchors, crlf.anchors);
    assert!(lf.links.is_empty());
    let with_link = text + "\n[坏链接][missing]\n\n[missing]: gone.md\n";
    let a = markdown::parse_document(&with_link, "docs/one.md");
    let b = markdown::parse_document(&with_link.replace('\n', "\r\n"), "docs/one.md");
    assert_eq!(a.links[0].line, b.links[0].line);
    assert_eq!(
        a.links[0].line,
        with_link
            .lines()
            .position(|s| s == "[坏链接][missing]")
            .unwrap()
            + 1
    );
}

#[test]
fn replacement_acceptance_and_extent_have_independent_meaning() {
    let repo = Repo::new();
    repo.put("docs/old.md", &doc("old", "accepted", ""));
    repo.put("docs/new.md", &doc("new", "draft", &edge("old", "full")));
    assert_eq!(
        repo.load()
            .unwrap()
            .pages
            .iter()
            .find(|p| p.metadata.id == "old")
            .unwrap()
            .validity,
        Validity::Unreplaced
    );
    repo.put(
        "docs/new.md",
        &doc("new", "accepted", &edge("old", "partial")),
    );
    let catalog = repo.load().unwrap();
    let old = catalog
        .pages
        .iter()
        .find(|p| p.metadata.id == "old")
        .unwrap();
    assert_eq!(old.validity, Validity::PartiallySuperseded);
    assert_eq!(old.metadata.status, metadata::Status::Accepted);
    assert_eq!(old.replaced_by, vec!["new"]);
    repo.put("docs/new.md", &doc("new", "retired", &edge("old", "full")));
    assert_eq!(
        repo.load()
            .unwrap()
            .pages
            .iter()
            .find(|p| p.metadata.id == "old")
            .unwrap()
            .validity,
        Validity::Superseded
    );
    repo.put(
        "docs/latest.md",
        &doc("latest", "accepted", &edge("new", "full")),
    );
    let catalog = repo.load().unwrap();
    assert!(
        catalog
            .pages
            .iter()
            .filter(|p| p.metadata.id != "latest")
            .all(|p| p.validity == Validity::Superseded)
    );
}

#[test]
fn proposed_and_rejected_edges_do_not_take_effect_and_partial_edges_do_not_add_to_full() {
    let repo = Repo::new();
    repo.put("docs/old.md", &doc("old", "accepted", ""));
    for status in ["draft", "review", "rejected"] {
        repo.put(
            "docs/new.md",
            &doc("new", status, &edge("old", "full"))
                .replace("kind = \"design\"", "kind = \"adr\""),
        );
        let catalog = repo.load().unwrap();
        let old = catalog
            .pages
            .iter()
            .find(|p| p.metadata.id == "old")
            .unwrap();
        assert_eq!(old.validity, Validity::Unreplaced);
        assert!(old.replaced_by.is_empty());
    }
    repo.put(
        "docs/new.md",
        &doc("new", "accepted", &edge("old", "partial")),
    );
    repo.put(
        "docs/another.md",
        &doc("another", "active", &edge("old", "partial")),
    );
    let catalog = repo.load().unwrap();
    let old = catalog
        .pages
        .iter()
        .find(|p| p.metadata.id == "old")
        .unwrap();
    assert_eq!(old.validity, Validity::PartiallySuperseded);
    assert_eq!(old.replaced_by, vec!["another", "new"]);
}

#[test]
fn graph_rejects_duplicates_dangling_cycles_and_conflicting_full_replacements() {
    let repo = Repo::new();
    repo.put("docs/a.md", &doc("a", "accepted", ""));
    repo.put("docs/b.md", &doc("a", "accepted", ""));
    assert!(repo.load().err().unwrap().contains("重复文档 ID"));
    repo.put("docs/b.md", &doc("b", "accepted", &edge("absent", "full")));
    assert!(repo.load().err().unwrap().contains("目标不存在"));
    repo.put("docs/b.md", &doc("b", "draft", &edge("a", "full")));
    repo.put("docs/a.md", &doc("a", "accepted", &edge("b", "partial")));
    assert!(repo.load().err().unwrap().contains("循环"));
    repo.put("docs/a.md", &doc("a", "accepted", ""));
    repo.put("docs/b.md", &doc("b", "accepted", &edge("a", "full")));
    repo.put("docs/c.md", &doc("c", "accepted", &edge("a", "full")));
    assert!(repo.load().err().unwrap().contains("多个直接全部取代"));
}

#[test]
fn indexes_are_derived_and_export_is_stable_across_line_endings() {
    let repo = Repo::new();
    let index = "+++\nschema_version = 1\nid = \"index\"\nkind = \"index\"\nstatus = \"active\"\nsummary = \"索引\"\n+++\n# 目录\n\n<!-- generated-doc-index:start -->\n<!-- generated-doc-index:end -->\n";
    repo.put("docs/README.md", index);
    repo.put("docs/a.md", &doc("a", "accepted", ""));
    let catalog = repo.load().unwrap();
    assert!(catalog.indexes(&repo.0, false).is_err());
    catalog.indexes(&repo.0, true).unwrap();
    let generated = fs::read_to_string(repo.0.join("docs/README.md")).unwrap();
    assert!(generated.contains("[测试 contract][doc-a] — 已接受；无生效取代。规范说明"));
    repo.load().unwrap().indexes(&repo.0, false).unwrap();
    let lf_catalog = serde_json::to_value(repo.load().unwrap()).unwrap();
    repo.put("docs/README.md", &generated.replace('\n', "\r\n"));
    repo.put("docs/a.md", &doc("a", "accepted", "").replace('\n', "\r\n"));
    repo.load().unwrap().indexes(&repo.0, false).unwrap();
    assert_eq!(
        lf_catalog,
        serde_json::to_value(repo.load().unwrap()).unwrap()
    );
    repo.put("docs/a.md", &doc("a", "active", ""));
    assert!(repo.load().unwrap().indexes(&repo.0, false).is_err());
}

#[test]
fn body_title_and_applicability_are_required_and_skill_fixtures_excluded() {
    let repo = Repo::new();
    repo.put(
        "docs/a.md",
        &doc("a", "accepted", "applicability = \"#missing\""),
    );
    assert!(repo.load().err().unwrap().contains("范围入口不存在"));
    repo.put("docs/a.md", &(doc("a", "accepted", "") + "\n# 第二标题\n"));
    assert!(repo.load().err().unwrap().contains("一级标题"));
    repo.put("docs/a.md", &doc("a", "accepted", ""));
    repo.put("docs/fixtures/raw.md", "not formal");
    repo.put(
        ".agents/skills/test/SKILL.md",
        "---\nname: test\n---\n# Skill\n",
    );
    assert_eq!(repo.load().unwrap().pages.len(), 1);
}

#[test]
fn explicit_anchors_cannot_collide_with_each_other_or_heading_slugs() {
    let repo = Repo::new();
    let text = doc("a", "accepted", "");
    for suffix in [
        "\n<a id=\"replacement\"></a>\n",
        "\n# replacement\n",
        "\n<a name=\"取代范围\"></a>\n",
    ] {
        repo.put("docs/a.md", &(text.clone() + suffix));
        assert!(repo.load().err().unwrap().contains("锚点冲突"));
    }
    repo.put(
        "docs/a.md",
        &(text + "\n<a id=\"same\" name=\"same\"></a>\n## 重复标题\n## 重复标题\n"),
    );
    let catalog = repo.load().unwrap();
    assert!(catalog.pages[0].anchors.contains("重复标题-1"));
}

#[test]
fn stable_identity_resolves_links_after_file_move() {
    let repo = Repo::new();
    repo.put("docs/old.md", &doc("stable", "accepted", ""));
    repo.put(
        "docs/caller.md",
        &(doc("caller", "accepted", "related = [\"stable\"]")
            + "\n[链接][target]\n\n[target]: old.md#replacement\n"),
    );
    let before = repo.load().unwrap();
    fs::create_dir_all(repo.0.join("docs/nested")).unwrap();
    fs::rename(
        repo.0.join("docs/old.md"),
        repo.0.join("docs/nested/new.md"),
    )
    .unwrap();
    repo.put(
        "docs/nested/new.md",
        &doc("stable", "accepted", "").replace("# 测试 `contract`", "# 新标题"),
    );
    repo.put(
        "docs/caller.md",
        &(doc("caller", "accepted", "related = [\"stable\"]")
            + "\n[链接][target]\n\n[target]: nested/new.md#replacement\n"),
    );
    let catalog = repo.load().unwrap();
    let link = &catalog
        .pages
        .iter()
        .find(|p| p.metadata.id == "caller")
        .unwrap()
        .links[0];
    assert_eq!(link.url.as_deref(), Some("/docs/stable/#replacement"));
    assert_eq!(
        before
            .pages
            .iter()
            .find(|p| p.metadata.id == "stable")
            .unwrap()
            .url,
        catalog
            .pages
            .iter()
            .find(|p| p.metadata.id == "stable")
            .unwrap()
            .url
    );
}

#[test]
fn directory_links_resolve_to_formal_index_identity() {
    let repo = Repo::new();
    repo.put(
        "docs/section/README.md",
        &doc("section", "active", "").replace("kind = \"design\"", "kind = \"index\""),
    );
    repo.put(
        "docs/caller.md",
        &(doc("caller", "active", "") + "\n[分类][section]\n\n[section]: section/\n"),
    );
    let catalog = repo.load().unwrap();
    let link = &catalog
        .pages
        .iter()
        .find(|p| p.metadata.id == "caller")
        .unwrap()
        .links[0];
    assert!(link.directory);
    assert_eq!(link.raw, "section/");
    assert_eq!(link.url.as_deref(), Some("/docs/section/"));
}
