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
        Self(path)
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
