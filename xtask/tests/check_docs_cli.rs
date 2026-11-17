//! `check-docs` 的进程退出码。规则细节在 `doc_checks` 单元测试里。

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "laneflow-xtask-docs-cli-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(path.join("xtask/data")).expect("temp repo");
        fs::write(path.join("xtask/data/doc-checks.toml"), "required = []\n").expect("config");
        fs::create_dir_all(path.join("docs")).expect("docs");
        fs::write(path.join("docs/page.md"), "+++\nschema_version = 1\nid = \"page\"\nkind = \"reference\"\nstatus = \"active\"\nsummary = \"说明\"\nscope = \"测试\"\n+++\n# Page\n").expect("formal document");
        fs::create_dir_all(path.join("docs/reference/terms")).unwrap();
        fs::write(path.join("docs/reference/terms/catalog.toml"), "schema_version = 1\nfiles = [\"terms.toml\"]\n[[domains]]\nid = \"example\"\nname = \"示例\"\n").unwrap();
        fs::write(path.join("docs/reference/terms/terms.toml"), "schema_version = 1\n[[terms]]\nid = \"example\"\nname = \"示例概念\"\nenglish_aliases = [\"example concept\"]\ndomains = [\"example\"]\ndefinition = \"用于说明的概念。\"\nreferences = [{ document = \"page\" }]\n").unwrap();
        for (file, id) in [
            ("glossary", "reference-glossary"),
            ("wording-guide", "reference-wording-guide"),
        ] {
            fs::write(path.join(format!("docs/reference/{file}.md")), format!("+++\nschema_version = 1\nid = \"{id}\"\nkind = \"reference\"\nstatus = \"active\"\nsummary = \"说明\"\nscope = \"测试\"\n+++\n# 阅读页\n\n<!-- generated-terminology:start -->\n\n<!-- generated-terminology:end -->\n")).unwrap();
        }
        let repo = Self(path);
        let generated = repo.xtask(&["generate-glossary"]);
        assert!(
            generated.status.success(),
            "{}",
            String::from_utf8_lossy(&generated.stderr)
        );
        repo
    }

    fn xtask(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .current_dir(&self.0)
            .args(args)
            .output()
            .expect("xtask")
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn check_docs_exits_nonzero_when_a_local_path_is_missing() {
    let repo = TestDirectory::new();
    fs::write(repo.0.join("README.md"), "[x][x]\n\n[x]: missing.md\n").expect("fixture");
    let output = repo.xtask(&["check-docs"]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("missing-path"), "{stderr}");
    assert!(stderr.contains("missing.md"), "{stderr}");
    assert!(stderr.contains("errors=1"), "{stderr}");
}

#[test]
fn check_docs_exits_zero_when_the_fixture_is_clean() {
    let repo = TestDirectory::new();
    fs::write(
        repo.0.join("README.md"),
        "# Title\n\n[self][self]\n\n[self]: #title\n",
    )
    .expect("fixture");
    let output = repo.xtask(&["check-docs"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("errors=0"), "{stdout}");
}

#[test]
fn export_docs_checks_metadata_and_writes_a_site_catalog() {
    let repo = TestDirectory::new();
    let page_path = repo.0.join("docs/page.md");
    let source = fs::read_to_string(&page_path).unwrap().replace(
        "status = \"active\"",
        "status = \"active\"\nrevised_date = \"2026-11-16\"",
    );
    fs::write(page_path, source).unwrap();
    let output = repo.xtask(&["export-docs"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let exported = fs::read_to_string(repo.0.join("target/docs/catalog.json")).unwrap();
    let catalog: serde_json::Value = serde_json::from_str(&exported).unwrap();
    assert_eq!(catalog["schema_version"], 2);
    assert_eq!(catalog["terminology"]["schema_version"], 1);
    let concept = &catalog["terminology"]["terms"][0];
    assert_eq!(concept["name"], "示例概念");
    assert_eq!(concept["source"], "docs/reference/terms/terms.toml");
    assert_eq!(concept["url"], "/docs/reference-glossary/#term-example");
    assert_eq!(concept["references"][0]["document"], "page");
    assert_eq!(catalog["pages"][0]["url"], "/docs/page/");
    assert_eq!(
        catalog["pages"][0]["metadata"]["revised_date"],
        "2026-11-16"
    );
    assert_eq!(catalog["pages"][0]["headings"][0]["anchor"], "page");
    assert!(
        !catalog["pages"][0]["body"]
            .as_str()
            .unwrap()
            .contains("schema_version")
    );
    fs::write(repo.0.join("docs/page.md"), "# Missing metadata\n").unwrap();
    let output = repo.xtask(&["export-docs"]);
    assert!(!output.status.success());
    assert_eq!(
        exported,
        fs::read_to_string(repo.0.join("target/docs/catalog.json")).unwrap()
    );
}

#[test]
fn check_docs_path_filter_does_not_bypass_formal_metadata() {
    let repo = TestDirectory::new();
    fs::write(repo.0.join("README.md"), "# Readme\n").unwrap();
    fs::write(repo.0.join("docs/page.md"), "# Missing metadata\n").unwrap();
    assert!(!repo.xtask(&["check-docs", "README.md"]).status.success());
}

#[test]
fn doc_checks_and_export_reject_terminology_drift_until_regenerated() {
    let repo = TestDirectory::new();
    fs::write(repo.0.join("README.md"), "# Readme\n").unwrap();
    assert!(repo.xtask(&["export-docs"]).status.success());
    let destination = repo.0.join("target/docs/catalog.json");
    let exported = fs::read_to_string(&destination).unwrap();
    let source_path = repo.0.join("docs/reference/terms/terms.toml");
    let source = fs::read_to_string(&source_path).unwrap();
    fs::write(
        source_path,
        source.replace("用于说明的概念。", "修订后的概念定义。"),
    )
    .unwrap();
    let check = repo.xtask(&["check-docs", "README.md"]);
    assert_eq!(check.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&check.stderr).contains("阅读页已漂移"));
    assert!(
        !repo
            .xtask(&["generate-glossary", "--check"])
            .status
            .success()
    );
    assert!(!repo.xtask(&["export-docs"]).status.success());
    assert_eq!(fs::read_to_string(&destination).unwrap(), exported);
    assert!(repo.xtask(&["generate-glossary"]).status.success());
    assert!(repo.xtask(&["export-docs"]).status.success());
    assert!(
        fs::read_to_string(destination)
            .unwrap()
            .contains("修订后的概念定义。")
    );
}
