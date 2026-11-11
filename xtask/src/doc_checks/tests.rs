use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::markdown::code_span_matches;
use super::scan::collect_sources;
use super::slug::{Slugger, github_subset_slug};
use super::{FindingKind, SkipKind, check, load_config};

static TEST_DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

struct TestRepo {
    path: PathBuf,
}

impl TestRepo {
    fn new(label: &str) -> Self {
        let id = TEST_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("laneflow-xtask-docs-{label}-{pid}-{nanos}-{id}"));
        fs::create_dir_all(&path).expect("temp repo");
        let repo = Self { path };
        repo.config("required = []\n");
        repo
    }

    fn config(&self, contents: &str) {
        self.write("xtask/data/doc-checks.toml", contents);
    }

    fn write(&self, rel: &str, contents: &str) {
        let path = self.path.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        fs::write(path, contents).expect("write fixture");
    }

    fn check(&self) -> super::Report {
        self.check_explicit(Some(std::slice::from_ref(&self.path)))
    }

    fn check_explicit(&self, explicit: Option<&[PathBuf]>) -> super::Report {
        let config = load_config(&self.path.join("xtask/data/doc-checks.toml")).expect("config");
        check(&self.path, explicit, &config).expect("check executes")
    }
}

impl Drop for TestRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn texts(report: &super::Report) -> Vec<String> {
    report.findings.iter().map(ToString::to_string).collect()
}

#[test]
fn slug_examples_are_hand_written_literals() {
    assert_eq!(github_subset_slug("快速开始"), "快速开始");
    assert_eq!(
        github_subset_slug("6.1 模型目录与登记"),
        "61-模型目录与登记"
    );
    assert_eq!(github_subset_slug("8. 测试与验证"), "8-测试与验证");
    assert_eq!(
        github_subset_slug("6.2 Required checks"),
        "62-required-checks"
    );
    assert_eq!(
        github_subset_slug("5. 同拍联合求解与预约约束（裁定 A3/A4 回写，核心）"),
        "5-同拍联合求解与预约约束裁定-a3a4-回写核心"
    );
    assert_eq!(
        github_subset_slug("7.2 LF-ROUTING-CANDIDATE-LFSYNTH-v1 首轮确定性证据（#1267）"),
        "72-lf-routing-candidate-lfsynth-v1-首轮确定性证据1267"
    );
    assert_eq!(github_subset_slug("Foo_Bar-Baz"), "foo_bar-baz");
    assert_eq!(github_subset_slug("Title {#custom}"), "title-custom");
    assert_eq!(github_subset_slug("É"), "É");
    assert_eq!(github_subset_slug("a  b"), "a-b");
    assert_eq!(
        github_subset_slug("6. 能力检查（裁定 A6 = 选项 3 回写）"),
        "6-能力检查裁定-a6-选项-3-回写"
    );

    let mut slugger = Slugger::default();
    assert_eq!(slugger.next("重复"), "重复");
    assert_eq!(slugger.next("重复"), "重复-1");
    assert_eq!(slugger.next("重复-1"), "重复-1-1");

    let mut collision = Slugger::default();
    assert_eq!(collision.next("Dup-1"), "dup-1");
    assert_eq!(collision.next("Dup"), "dup");
    assert_eq!(collision.next("Dup"), "dup-2");
}

#[test]
fn reports_missing_path_and_missing_anchor() {
    let repo = TestRepo::new("missing");
    repo.write(
        "a.md",
        "# Hello\n\n[gone](missing.md)\n\n[bad](b.md#nope)\n\n[ok](b.md#hello)\n\n[self](#hello)\n",
    );
    repo.write("b.md", "# Hello\n");
    let report = repo.check();
    assert_eq!(
        texts(&report),
        vec![
            "a.md:3:1: target=missing.md: missing-path: 本地路径不存在".to_string(),
            "a.md:5:1: target=b.md#nope: missing-anchor: 目标 Markdown 中没有匹配的标题或显式 HTML 锚点"
                .to_string(),
        ]
    );
    assert!(report.findings.iter().all(|finding| finding.line > 0));
    assert_eq!(report.links, 4);
}

#[test]
fn accepts_same_file_and_cross_file_references() {
    let repo = TestRepo::new("refs");
    repo.write(
        "a.md",
        "## 快速开始\n\n[self](#快速开始)\n\n[cross](b.md#第二节)\n",
    );
    repo.write("b.md", "## 第二节\n");
    let report = repo.check();
    assert!(texts(&report).is_empty(), "{}", texts(&report).join("\n"));
}

#[test]
fn accepts_chinese_names_and_percent_encoding() {
    let repo = TestRepo::new("encoding");
    repo.write("中文.md", "## 快速开始\n");
    repo.write("a b.md", "# Spaced\n");
    repo.write(
        "index.md",
        "[cn](%E4%B8%AD%E6%96%87.md#%E5%BF%AB%E9%80%9F%E5%BC%80%E5%A7%8B)\n[sp](a%20b.md#spaced)\n",
    );
    let report = repo.check();
    assert!(texts(&report).is_empty(), "{}", texts(&report).join("\n"));
}

#[test]
fn duplicate_heading_suffixes_and_collisions_use_hand_written_fragments() {
    let repo = TestRepo::new("dup");
    repo.write(
        "suffix.md",
        "# 重复\n\n# 重复\n\n# 重复-1\n\n[a](#重复)\n[b](#重复-1)\n[c](#重复-1-1)\n[d](#重复-2)\n",
    );
    repo.write(
        "collision.md",
        "# Dup-1\n\n# Dup\n\n# Dup\n\n[a](#dup-1)\n[b](#dup)\n[c](#dup-2)\n[d](#dup-3)\n",
    );
    let report = repo.check();
    assert_eq!(
        texts(&report),
        vec![
            "collision.md:10:1: target=#dup-3: missing-anchor: 目标 Markdown 中没有匹配的标题或显式 HTML 锚点"
                .to_string(),
            "suffix.md:10:1: target=#重复-2: missing-anchor: 目标 Markdown 中没有匹配的标题或显式 HTML 锚点"
                .to_string(),
        ]
    );
}

#[test]
fn html_id_and_name_are_anchors_and_custom_heading_syntax_is_not() {
    let repo = TestRepo::new("html");
    repo.write(
        "a.md",
        "<a id=\"templates\"></a>\n<a name='legacy'></a>\n<a id=\"gamma\" name=\"delta\"></a>\n<!-- <a id=\"hidden\"></a> -->\n<!--\n<a id=\"multiline\"></a>\n-->\n<a id=raw></a>\n<abbr id=\"nope\"></abbr>\n## Title {#custom}\n\n[t](#templates)\n[l](#legacy)\n[g](#gamma)\n[d](#delta)\n[h](#hidden)\n[m](#multiline)\n[r](#raw)\n[n](#nope)\n[c](#custom)\n[s](#title-custom)\n<a title=\"example id='quoted'\"></a>\n<a title=\"id='skip'\" id=\"kept\"></a>\n[q](#quoted)\n[k](#kept)\n",
    );
    let report = repo.check();
    let targets: Vec<_> = report
        .findings
        .iter()
        .map(|finding| finding.target.as_str())
        .collect();
    assert_eq!(
        targets,
        vec![
            "#hidden",
            "#multiline",
            "#raw",
            "#nope",
            "#custom",
            "#quoted"
        ]
    );
    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.kind == FindingKind::Anchor)
    );
}

#[test]
fn reference_links_resolve_and_undefined_references_fail() {
    let repo = TestRepo::new("reference");
    repo.write(
        "a.md",
        "See [other][id] and [broken][missing-label].\n\n[id]: b.md#第二节\n",
    );
    repo.write("b.md", "## 第二节\n");
    let report = repo.check();
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].kind, FindingKind::Path);
    assert!(report.findings[0].reason.contains("引用链接没有定义"));
    assert!(report.findings[0].target.contains("missing-label"));
}

#[test]
fn ignores_fake_links_in_code_and_comments_and_keeps_real_ones() {
    let repo = TestRepo::new("fake");
    repo.write(
        "a.md",
        "# Title\n\n```\n[nope](missing-from-fence.md)\n```\n\n    [nope](missing-from-indent.md)\n\nInline `[nope](missing-from-code.md)` stays.\n\n<!-- [nope](missing-from-comment.md) -->\n\n[shortcut]\n\n[real](missing-real.md)\n",
    );
    let report = repo.check();
    assert_eq!(
        texts(&report),
        vec!["a.md:15:1: target=missing-real.md: missing-path: 本地路径不存在".to_string()]
    );
}

#[test]
fn historical_and_http_urls_are_counted_not_fetched() {
    let repo = TestRepo::new("http");
    repo.write(
        "a.md",
        "[ext](https://example.invalid/nope)\n[hist](https://github.com/illusion-tech/laneflow/blob/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/README.md)\n[mail](mailto:dev@example.com)\n<user@example.com>\n[sym](std::vec::Vec)\n[local](http://127.0.0.1:9/should-not-be-fetched)\n",
    );
    let report = repo.check();
    assert!(texts(&report).is_empty(), "{}", texts(&report).join("\n"));
    assert_eq!(report.skips.get(&SkipKind::Http).copied(), Some(3));
    assert_eq!(report.skips.get(&SkipKind::Mailto).copied(), Some(2));
    assert_eq!(report.skips.get(&SkipKind::Rustdoc).copied(), Some(1));
    assert!(report.skip_notes.is_empty());
}

#[test]
fn rejects_path_escape_symlinks_queries_and_non_markdown_fragments() {
    let repo = TestRepo::new("escape");
    repo.write(
        "nested/a.md",
        "[out](../../outside.md)\n[up](/outside.md)\n",
    );
    repo.write("real.md", "# Title\n");
    repo.write("file.rs", "fn main() {}\n");
    repo.write("dir/readme.txt", "x\n");
    repo.write(
        "links.md",
        "[sym](link.md)\n[code](file.rs#L10)\n[dir](dir/#section)\n[query](real.md?plain=1)\n[bad](%ZZ.md)\n[slash](nested/a.md/)\n",
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(repo.path.join("real.md"), repo.path.join("link.md"))
            .expect("symlink");
    }
    let report = repo.check();
    let rendered = texts(&report).join("\n");
    assert!(rendered.contains("nested/a.md"), "{rendered}");
    assert!(rendered.contains("路径越出仓库"), "{rendered}");
    assert!(rendered.contains("target=../../outside.md"), "{rendered}");
    assert!(rendered.contains("target=/outside.md"), "{rendered}");
    #[cfg(unix)]
    assert!(
        rendered.contains("target=link.md") && rendered.contains("不跟随符号链接"),
        "{rendered}"
    );
    assert!(rendered.contains("带斜杠的路径不是目录"), "{rendered}");
    assert!(!rendered.contains("file.rs"), "{rendered}");
    assert_eq!(
        report.skips.get(&SkipKind::NonMarkdownFragment).copied(),
        Some(1)
    );
    let notes = report
        .skip_notes
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(notes.contains("带 fragment 的目录链接不支持"), "{notes}");
    assert!(notes.contains("查询字符串不支持"), "{notes}");
    assert!(notes.contains("百分号编码无效"), "{notes}");
    assert!(
        !rendered.contains("missing-anchor") || !rendered.contains("file.rs"),
        "{rendered}"
    );
}

#[test]
fn inline_formatting_setext_and_atx_closing_hashes() {
    let repo = TestRepo::new("format");
    repo.write(
        "a.md",
        "## **Foo** `Bar` [baz](https://example.invalid)\n\n[ok](#foo-bar-baz)\n\nHello\n=====\n\n[setext](#hello)\n\n## Closing ##\n\n[close](#closing)\n",
    );
    let report = repo.check();
    assert!(texts(&report).is_empty(), "{}", texts(&report).join("\n"));
    assert_eq!(report.skips.get(&SkipKind::Http).copied(), Some(1));
}

#[test]
fn required_entries_accept_links_or_exact_code_spans_and_survive_path_filters() {
    let repo = TestRepo::new("required");
    repo.config(
        r#"
[[required]]
source = "AGENTS.md"
target = ".agents/README.md"
accept_code_span = true
reason = "行内代码入口"

[[required]]
source = ".agents/README.md"
target = ".agents/skills/demo/SKILL.md"
accept_code_span = true
reason = "相对代码路径"

[[required]]
source = "docs/adr/README.md"
target = "docs/governance/development-gates.md#62-required-checks"
reason = "必须指向锚点"
"#,
    );
    repo.write("AGENTS.md", "读 `.agents/README.md`。\n");
    repo.write(".agents/README.md", "- `skills/demo/SKILL.md`\n");
    repo.write(".agents/skills/demo/SKILL.md", "# Demo\n");
    repo.write(
        "docs/adr/README.md",
        "[门禁](../governance/development-gates.md#62-required-checks)\n",
    );
    repo.write(
        "docs/governance/development-gates.md",
        "### 6.2 Required checks\n",
    );
    repo.write("notes.md", "# Notes\n");
    let narrowed = repo.check_explicit(Some(&[PathBuf::from("notes.md")]));
    assert!(
        texts(&narrowed).is_empty(),
        "{}",
        texts(&narrowed).join("\n")
    );
    assert_eq!(narrowed.sources, 1);

    repo.write("AGENTS.md", "没有入口。\n");
    let missing = repo.check();
    assert_eq!(missing.findings.len(), 1);
    assert_eq!(missing.findings[0].kind, FindingKind::RequiredEntry);
    assert_eq!(missing.findings[0].target, ".agents/README.md");
    assert_eq!(missing.findings[0].line, 1);
    assert!(missing.findings[0].reason.contains("行内代码入口"));
}

#[test]
fn code_span_resolution_is_exact() {
    assert!(code_span_matches(
        ".agents/README.md",
        "skills/demo/SKILL.md",
        ".agents/skills/demo/SKILL.md",
        None
    ));
    assert!(code_span_matches(
        ".cursor/skills/demo/SKILL.md",
        ".agents/skills/demo/SKILL.md",
        ".agents/skills/demo/SKILL.md",
        None
    ));
    assert!(!code_span_matches(
        "AGENTS.md",
        "请阅读 .agents/README.md 再继续",
        ".agents/README.md",
        None
    ));
    assert!(!code_span_matches("a.md", "b.md", "a.md", None));
}

#[test]
fn scan_does_not_write_files() {
    let repo = TestRepo::new("readonly");
    repo.write("a.md", "[x](missing.md)\n");
    let before = fs::read(repo.path.join("a.md")).expect("read");
    let listing_before = list_files(&repo.path);
    let _ = repo.check();
    assert_eq!(fs::read(repo.path.join("a.md")).expect("read"), before);
    assert_eq!(list_files(&repo.path), listing_before);
}

#[test]
fn findings_are_stably_sorted_and_duplicate_links_are_both_reported() {
    let repo = TestRepo::new("sort");
    repo.config(
        r#"
[[required]]
source = "a.md"
target = "missing-entry.md"
reason = "入口缺失"
"#,
    );
    repo.write("z.md", "[z](no-z.md)\n");
    repo.write("a.md", "[one](no-a.md)\n[two](no-a.md)\n");
    let report = repo.check();
    assert_eq!(
        texts(&report),
        vec![
            "a.md:1:1: target=no-a.md: missing-path: 本地路径不存在".to_string(),
            "a.md:1:1: target=missing-entry.md: missing-required-entry: 入口缺失".to_string(),
            "a.md:2:1: target=no-a.md: missing-path: 本地路径不存在".to_string(),
            "z.md:1:1: target=no-z.md: missing-path: 本地路径不存在".to_string(),
        ]
    );
}

#[test]
fn column_counts_unicode_scalars_and_crlf_lines() {
    let repo = TestRepo::new("columns");
    repo.write("utf8.md", "中文 [x](missing.md)\n");
    repo.write("crlf.md", "# Title\r\n\r\n[bad](missing.md)\r\n");
    let report = repo.check();
    let utf8 = report
        .findings
        .iter()
        .find(|finding| finding.path == "utf8.md")
        .expect("utf8 finding");
    assert_eq!((utf8.line, utf8.column), (1, 4));
    let crlf = report
        .findings
        .iter()
        .find(|finding| finding.path == "crlf.md")
        .expect("crlf finding");
    assert_eq!((crlf.line, crlf.column), (3, 1));
}

#[test]
fn default_source_set_skips_research_history_and_explicit_paths_can_enter() {
    let repo = TestRepo::new("scope");
    for rel in [
        "README.md",
        "CONTRIBUTING.md",
        "AGENTS.md",
        "research/README.md",
        "research/history/old.md",
        "docs/guide.md",
        "docs/tests/hidden.md",
        "docs/pkg/src/generated/skip.md",
        "crates/demo/README.md",
        "crates/demo/nested/README.md",
        "crates/demo/tests/nope.md",
        "tools/demo/README.md",
        "schemas/nested/a.md",
        ".github/PULL_REQUEST_TEMPLATE.md",
        ".github/fixtures/nope.md",
        ".cursor/skills/demo/SKILL.md",
        ".cursor/skills/demo/extra.md",
        ".agents/skills/demo/SKILL.md",
        "vendor/x.md",
        "target/x.md",
        "node_modules/x.md",
    ] {
        repo.write(rel, "# X\n");
    }
    let config = load_config(&repo.path.join("xtask/data/doc-checks.toml")).expect("config");
    let default_sources = collect_sources(&repo.path, None).expect("default");
    assert_eq!(
        default_sources,
        BTreeSet::from([
            "README.md".to_string(),
            "CONTRIBUTING.md".to_string(),
            "AGENTS.md".to_string(),
            "research/README.md".to_string(),
            "docs/guide.md".to_string(),
            "crates/demo/README.md".to_string(),
            "tools/demo/README.md".to_string(),
            "schemas/nested/a.md".to_string(),
            ".github/PULL_REQUEST_TEMPLATE.md".to_string(),
            ".cursor/skills/demo/SKILL.md".to_string(),
            ".agents/skills/demo/SKILL.md".to_string(),
        ])
    );
    let explicit = collect_sources(
        &repo.path,
        Some(&[PathBuf::from("research/history/old.md")]),
    )
    .expect("explicit");
    assert_eq!(
        explicit,
        BTreeSet::from(["research/history/old.md".to_string()])
    );
    let report = check(
        &repo.path,
        Some(&[PathBuf::from("research/history/old.md")]),
        &config,
    )
    .expect("explicit still checks required edges");
    assert_eq!(report.sources, 1);
}

#[test]
fn explicit_symlink_and_outside_path_are_execution_errors() {
    let repo = TestRepo::new("explicit-edge");
    repo.write("real.md", "# X\n");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(repo.path.join("real.md"), repo.path.join("link.md"))
            .expect("symlink");
        let config = load_config(&repo.path.join("xtask/data/doc-checks.toml")).expect("config");
        let error =
            check(&repo.path, Some(&[PathBuf::from("link.md")]), &config).expect_err("symlink");
        assert!(error.contains("不跟随符号链接"), "{error}");
    }
    let config = load_config(&repo.path.join("xtask/data/doc-checks.toml")).expect("config");
    let error =
        check(&repo.path, Some(&[PathBuf::from("../outside.md")]), &config).expect_err("escape");
    assert!(
        error.contains("越出仓库") || error.contains("无法读取路径"),
        "{error}"
    );
}

#[cfg(unix)]
#[test]
fn explicit_source_rejects_a_symlinked_ancestor() {
    let repo = TestRepo::new("sym-ancestor");
    let outside = std::env::temp_dir().join(format!(
        "laneflow-xtask-docs-outside-{}-{}",
        std::process::id(),
        TEST_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&outside).expect("outside dir");
    let _cleanup = RemoveDir(&outside);
    fs::write(outside.join("secret.md"), "# Secret\n").expect("outside file");
    std::os::unix::fs::symlink(&outside, repo.path.join("linked")).expect("symlink");
    let config = load_config(&repo.path.join("xtask/data/doc-checks.toml")).expect("config");
    let error = check(
        &repo.path,
        Some(&[PathBuf::from("linked/secret.md")]),
        &config,
    )
    .expect_err("symlink ancestor");
    assert!(error.contains("不跟随符号链接"), "{error}");
    assert!(!error.contains("Secret"), "{error}");
}

#[test]
fn images_do_not_satisfy_required_edges_but_their_paths_are_still_checked() {
    let repo = TestRepo::new("image-edge");
    repo.config(
        r#"
[[required]]
source = "README.md"
target = "docs/README.md"
reason = "必须是导航链接"
"#,
    );
    repo.write("docs/README.md", "# Docs\n");
    repo.write(
        "README.md",
        "![preview](docs/README.md)\n![gone](missing.png)\n",
    );
    let image_only = texts(&repo.check()).join("\n");
    assert!(
        image_only.contains("missing-required-entry"),
        "{image_only}"
    );
    assert!(image_only.contains("target=missing.png"), "{image_only}");

    repo.write(
        "README.md",
        "[docs](docs/README.md)\n![gone](missing.png)\n",
    );
    let linked = texts(&repo.check());
    assert_eq!(
        linked,
        vec!["README.md:2:1: target=missing.png: missing-path: 本地路径不存在".to_string()]
    );
}

#[test]
fn code_spans_inside_image_alt_do_not_satisfy_required_edges() {
    let repo = TestRepo::new("image-alt");
    repo.config(
        r#"
[[required]]
source = "AGENTS.md"
target = ".agents/README.md"
accept_code_span = true
reason = "不能藏在图片替代文字里"
"#,
    );
    repo.write(".agents/README.md", "# Agents\n");
    repo.write("preview.png", "png");
    repo.write("AGENTS.md", "![see `.agents/README.md`](preview.png)\n");
    assert!(
        texts(&repo.check())
            .join("\n")
            .contains("missing-required-entry"),
        "{}",
        texts(&repo.check()).join("\n")
    );
    repo.write("AGENTS.md", "`.agents/README.md`\n");
    assert!(
        texts(&repo.check()).is_empty(),
        "{}",
        texts(&repo.check()).join("\n")
    );
}

#[test]
fn directory_form_references_do_not_satisfy_file_edges() {
    let repo = TestRepo::new("dir-span");
    repo.config(
        r#"
[[required]]
source = "AGENTS.md"
target = ".agents/README.md"
accept_code_span = true
reason = "必须是文件路径"
"#,
    );
    repo.write(".agents/README.md", "# Agents\n");
    repo.write("AGENTS.md", "`.agents/README.md/`\n");
    assert!(
        texts(&repo.check())
            .iter()
            .any(|line| line.contains("missing-required-entry")),
        "{}",
        texts(&repo.check()).join("\n")
    );
    repo.write("AGENTS.md", "[agents](.agents/README.md/)\n");
    assert!(
        texts(&repo.check())
            .iter()
            .any(|line| line.contains("missing-required-entry"))
    );
    repo.write("AGENTS.md", "`.agents/README.md`\n");
    assert!(
        texts(&repo.check()).is_empty(),
        "{}",
        texts(&repo.check()).join("\n")
    );
}

#[test]
fn protocol_relative_urls_stay_external_and_repo_root_paths_stay_local() {
    let repo = TestRepo::new("protocol-relative");
    repo.write("docs/README.md", "# Docs\n");
    repo.write(
        "a.md",
        "[ext](//example.com/docs)\n[root](/docs/README.md)\n",
    );
    let report = repo.check();
    assert!(texts(&report).is_empty(), "{}", texts(&report).join("\n"));
    assert_eq!(report.skips.get(&SkipKind::Http).copied(), Some(1));
}

#[test]
fn anchor_values_decode_entities_and_ignore_raw_text() {
    let repo = TestRepo::new("entities");
    repo.write(
        "a.md",
        "<a id=\"a&amp;b\"></a>\n<script type=\"text/plain\">\n<a id=\"hidden\"></a>\n</script>\n\n[ok](#a%26b)\n[raw](#a&amp;b)\n[no](#hidden)\n",
    );
    let targets: Vec<_> = texts(&repo.check())
        .into_iter()
        .filter_map(|line| {
            line.split("target=")
                .nth(1)
                .and_then(|rest| rest.split(':').next())
                .map(str::to_string)
        })
        .collect();
    assert_eq!(targets, vec!["#hidden".to_string()]);
}

#[test]
fn numeric_references_follow_html_replacements_and_duplicate_attributes_keep_the_first() {
    let repo = TestRepo::new("numeric-anchor");
    repo.write(
        "a.md",
        "<a id=\"&#128;\"></a>\n<a id=\"real\" id=\"hidden\"></a>\n\n[euro](#%E2%82%AC)\n[control](#%C2%80)\n[first](#real)\n[second](#hidden)\n",
    );
    let targets: Vec<_> = repo
        .check()
        .findings
        .iter()
        .map(|finding| finding.target.clone())
        .collect();
    assert_eq!(targets, vec!["#%C2%80".to_string(), "#hidden".to_string()]);
}

#[test]
fn semicolonless_numeric_references_and_template_contents_follow_html() {
    let repo = TestRepo::new("template-ref");
    repo.write(
        "a.md",
        "<a id=\"&#65\"></a>\n<template>\n<a id=\"hidden\"></a>\n</template>\n<a id=\"shown\"></a>\n\n[letter](#A)\n[no](#hidden)\n[yes](#shown)\n",
    );
    let targets: Vec<_> = repo
        .check()
        .findings
        .iter()
        .map(|finding| finding.target.clone())
        .collect();
    assert_eq!(targets, vec!["#hidden".to_string()]);
}

#[test]
fn crate_and_tool_child_readmes_skip_excluded_directory_names() {
    let repo = TestRepo::new("child-skip");
    repo.write("crates/demo/README.md", "# Demo\n");
    repo.write("crates/tests/README.md", "[x](missing.md)\n");
    repo.write("crates/fixtures/README.md", "# Fixture\n");
    repo.write("tools/demo/README.md", "# Tool\n");
    repo.write("tools/evidence/README.md", "# Evidence\n");
    repo.write("tools/node_modules/README.md", "# Modules\n");
    let sources = collect_sources(&repo.path, None).expect("default sources");
    assert!(sources.contains("crates/demo/README.md"), "{sources:?}");
    assert!(sources.contains("tools/demo/README.md"), "{sources:?}");
    assert!(
        sources.iter().all(|path| {
            !path.contains("/tests/")
                && !path.contains("/fixtures/")
                && !path.contains("/evidence/")
                && !path.contains("/node_modules/")
        }),
        "{sources:?}"
    );
}

#[test]
fn self_closing_raw_text_tags_still_hide_following_anchors() {
    let repo = TestRepo::new("script-slash");
    repo.write(
        "a.md",
        "<script/>\n<a id=\"hidden\"></a>\n</script>\n<a id=\"shown\"></a>\n\n[h](#hidden)\n[s](#shown)\n",
    );
    let targets: Vec<_> = repo
        .check()
        .findings
        .iter()
        .map(|finding| finding.target.clone())
        .collect();
    assert_eq!(targets, vec!["#hidden".to_string()]);
}

#[test]
fn path_spelling_is_case_sensitive() {
    let repo = TestRepo::new("case");
    repo.write("docs/README.md", "# Docs\n");
    repo.write(
        "a.md",
        "[wrong-dir](Docs/README.md)\n[wrong-file](docs/readme.md)\n[ok](docs/README.md)\n",
    );
    let targets: Vec<_> = repo
        .check()
        .findings
        .iter()
        .map(|finding| finding.target.clone())
        .collect();
    assert_eq!(
        targets,
        vec!["Docs/README.md".to_string(), "docs/readme.md".to_string()]
    );
}

#[test]
fn fragmentless_edges_reject_extra_fragments() {
    let repo = TestRepo::new("extra-fragment");
    repo.config(
        r#"
[[required]]
source = "AGENTS.md"
target = ".agents/README.md"
accept_code_span = true
reason = "不带 fragment"
"#,
    );
    repo.write(".agents/README.md", "# Agents\n");
    repo.write("AGENTS.md", "`.agents/README.md#missing`\n");
    assert!(
        texts(&repo.check())
            .join("\n")
            .contains("missing-required-entry")
    );
    repo.write("AGENTS.md", "[agents](.agents/README.md#missing)\n");
    assert!(
        texts(&repo.check())
            .join("\n")
            .contains("missing-required-entry")
    );
    repo.write("AGENTS.md", "`.agents/README.md`\n");
    assert!(
        texts(&repo.check()).is_empty(),
        "{}",
        texts(&repo.check()).join("\n")
    );
}

#[test]
fn quoted_greater_than_does_not_hide_the_anchor_id() {
    let repo = TestRepo::new("gt-attr");
    repo.write(
        "a.md",
        "<a title=\"1 > 0\" id=\"section\"></a>\n\n[ok](#section)\n",
    );
    assert!(
        texts(&repo.check()).is_empty(),
        "{}",
        texts(&repo.check()).join("\n")
    );
}

#[cfg(unix)]
#[test]
fn unreadable_existing_source_root_is_an_execution_error() {
    use std::os::unix::fs::PermissionsExt;
    let repo = TestRepo::new("unreadable-root");
    let crates = repo.path.join("crates");
    fs::create_dir(&crates).expect("crates");
    let _restore = RestoreMode(crates.clone());
    let blocked = fs::Permissions::from_mode(0o0);
    fs::set_permissions(&crates, blocked).expect("chmod");
    let readable = fs::read_dir(&crates).is_ok();
    let result = collect_sources(&repo.path, None);
    if readable {
        return;
    }
    let error = result.expect_err("unreadable crates");
    assert!(error.contains("无法读取目录"), "{error}");
}

#[cfg(unix)]
struct RestoreMode(PathBuf);

#[cfg(unix)]
impl Drop for RestoreMode {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&self.0)
            .map(|metadata| metadata.permissions())
            .unwrap_or_else(|_| fs::Permissions::from_mode(0o755));
        permissions.set_mode(0o755);
        let _ = fs::set_permissions(&self.0, permissions);
    }
}

#[cfg(unix)]
#[test]
fn default_sources_do_not_follow_symlinked_roots() {
    let repo = TestRepo::new("sym-roots");
    let outside = std::env::temp_dir().join(format!(
        "laneflow-xtask-docs-roots-{}-{}",
        std::process::id(),
        TEST_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(outside.join("evil")).expect("outside dir");
    let _cleanup = RemoveDir(&outside);
    fs::write(outside.join("evil/README.md"), "# Outside\n").expect("readme");
    fs::write(outside.join("NOTES.md"), "# Notes\n").expect("notes");
    std::os::unix::fs::symlink(&outside, repo.path.join("crates")).expect("crates symlink");
    std::os::unix::fs::symlink(&outside, repo.path.join("tools")).expect("tools symlink");
    std::os::unix::fs::symlink(&outside, repo.path.join(".github")).expect("github symlink");
    let sources = collect_sources(&repo.path, None).expect("default sources");
    assert!(
        sources.iter().all(|path| !path.starts_with("crates/")
            && !path.starts_with("tools/")
            && !path.starts_with(".github/")),
        "{sources:?}"
    );
}

#[cfg(unix)]
struct RemoveDir<'a>(&'a Path);

#[cfg(unix)]
impl Drop for RemoveDir<'_> {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.0);
    }
}

#[test]
fn configured_required_edges_match_the_agreed_set() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let config = load_config(&root.join("xtask/data/doc-checks.toml")).expect("config");
    let actual: Vec<_> = config
        .required
        .iter()
        .map(|edge| {
            (
                edge.source.as_str(),
                edge.display.as_str(),
                edge.accept_code_span,
            )
        })
        .collect();
    assert_eq!(actual, AGREED_EDGES);
}

#[test]
fn repository_required_entries_are_present() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let config = load_config(&root.join("xtask/data/doc-checks.toml")).expect("config");
    let report = check(&root, None, &config).expect("repository scan executes");
    let missing: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| finding.kind == FindingKind::RequiredEntry)
        .map(ToString::to_string)
        .collect();
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

fn list_files(root: &Path) -> BTreeSet<String> {
    let mut files = BTreeSet::new();
    visit(root, root, &mut files);
    files
}

fn visit(root: &Path, dir: &Path, files: &mut BTreeSet<String>) {
    let mut entries = fs::read_dir(dir)
        .expect("read")
        .collect::<Result<Vec<_>, _>>()
        .expect("entries");
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            visit(root, &path, files);
        } else {
            files.insert(
                path.strip_prefix(root)
                    .expect("prefix")
                    .display()
                    .to_string(),
            );
        }
    }
}

const AGREED_EDGES: &[(&str, &str, bool)] = &[
    ("README.md", "docs/README.md", false),
    ("README.md", "CONTRIBUTING.md", false),
    ("README.md", "AGENTS.md", false),
    ("README.md", "docs/reference/capabilities.md", false),
    ("README.md", "docs/guides/source-integration.md", false),
    ("docs/README.md", "README.md", false),
    ("docs/README.md", "docs/architecture.md", false),
    ("docs/README.md", "docs/roadmap.md", false),
    ("docs/README.md", "docs/reference/capabilities.md", false),
    ("docs/README.md", "docs/guides/source-integration.md", false),
    ("docs/README.md", "docs/adr/README.md", false),
    ("docs/README.md", "docs/design/README.md", false),
    (
        "docs/README.md",
        "docs/governance/documentation-policy.md",
        false,
    ),
    ("docs/README.md", "docs/reference/README.md", false),
    (
        "docs/reference/README.md",
        "docs/reference/capabilities.md",
        false,
    ),
    (
        "docs/reference/README.md",
        "docs/reference/glossary.md",
        false,
    ),
    (
        "docs/reference/README.md",
        "docs/reference/validation-matrix.md",
        false,
    ),
    (
        "docs/design/README.md",
        "docs/reference/capabilities.md",
        false,
    ),
    (
        "docs/design/README.md",
        "docs/design/traffic-runtime-shared-consumption.md",
        false,
    ),
    (
        "docs/adr/README.md",
        "docs/governance/development-gates.md#62-required-checks",
        false,
    ),
    ("AGENTS.md", ".agents/README.md", true),
    ("AGENTS.md", "docs/governance/development-gates.md", true),
    ("AGENTS.md", "docs/reference/glossary.md", true),
    (
        ".agents/README.md",
        ".agents/skills/laneflow-governance/SKILL.md",
        true,
    ),
    (
        ".agents/README.md",
        ".agents/skills/laneflow-development/SKILL.md",
        true,
    ),
    (
        ".agents/README.md",
        ".agents/skills/laneflow-core-design/SKILL.md",
        true,
    ),
    (
        ".agents/README.md",
        ".agents/skills/laneflow-adapter/SKILL.md",
        true,
    ),
    (
        ".agents/README.md",
        ".agents/skills/laneflow-pre-1-0/SKILL.md",
        true,
    ),
    (
        ".cursor/skills/laneflow-governance/SKILL.md",
        ".agents/skills/laneflow-governance/SKILL.md",
        true,
    ),
    (
        ".cursor/skills/laneflow-development/SKILL.md",
        ".agents/skills/laneflow-development/SKILL.md",
        true,
    ),
    (
        ".cursor/skills/laneflow-core-design/SKILL.md",
        ".agents/skills/laneflow-core-design/SKILL.md",
        true,
    ),
    (
        ".cursor/skills/laneflow-adapter/SKILL.md",
        ".agents/skills/laneflow-adapter/SKILL.md",
        true,
    ),
    (
        ".cursor/skills/laneflow-pre-1-0/SKILL.md",
        ".agents/skills/laneflow-pre-1-0/SKILL.md",
        true,
    ),
    (
        "crates/laneflow-runtime/README.md",
        "docs/guides/source-integration.md",
        false,
    ),
    (
        "crates/laneflow-runtime/README.md",
        "docs/reference/capabilities.md",
        false,
    ),
    (
        "crates/laneflow-runtime/README.md",
        "docs/design/traffic-runtime-shared-consumption.md",
        false,
    ),
];
