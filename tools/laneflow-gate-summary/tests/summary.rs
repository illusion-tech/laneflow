//! 汇编器集成测试：fixture manifest → 回执；以及失败关闭路径。
//!
//! fixture 记录在 `tests/fixtures/`（repo/ 充当仓库根、evidence-root/ 充当
//! 仓库外证据根）；基础 manifest 是 `fixtures/example/unified-gate.toml`，与 README
//! 默认命令共用。

use std::path::Path;

use laneflow_gate_summary::evidence::PathContext;
use laneflow_gate_summary::receipt::Mode;
use laneflow_gate_summary::summary;

fn fixtures() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn ctx() -> PathContext {
    PathContext {
        repo_root: fixtures().join("repo"),
        evidence_root: Some(fixtures().join("evidence-root")),
    }
}

/// 以占位符替换构造测试 manifest，写入临时目录。
fn write_manifest(body: &str) -> std::path::PathBuf {
    let directory = tempfile::tempdir().expect("tempdir");
    // manifest 生命周期须长过返回值：保留目录由测试进程收尾清理。
    let path = directory.keep().join("unified-gate.toml");
    std::fs::write(&path, body).expect("write manifest");
    path
}

/// 自包含示例 manifest（README 默认命令同用），各失败路径测试在其上逐项改写。
fn base_manifest() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/example/unified-gate.toml");
    std::fs::read_to_string(&path).expect("读取示例 manifest")
}

fn output_dir(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("gate-summary-{tag}-{}", std::process::id()))
}

#[test]
fn trial_run_produces_receipt_with_all_sections() {
    let manifest = write_manifest(&base_manifest());
    let output = output_dir("ok");
    let receipt = summary::run(&manifest, &ctx(), &output, Mode::Trial, false).expect("汇总成功");
    assert_eq!(receipt.mode, "trial");
    assert_eq!(receipt.overview.len(), 2);
    assert!(receipt.overview[0].cells[2].contains("未认证"));
    assert!(receipt.overview[1].cells[0].contains("已延后"));
    assert_eq!(receipt.pending_items.len(), 1);
    assert_eq!(receipt.pending_items[0].axis, "routing");

    let markdown = std::fs::read_to_string(output.join("receipt.md")).expect("receipt.md");
    assert!(markdown.contains("lf-unified-gate-receipt-2"));
    assert!(markdown.contains("试跑仅核对声明和证据"));
    assert!(markdown.contains("范围裁定清单"));
    assert!(markdown.contains("worker 两两组合"));
    assert!(markdown.contains("能力限制登记"));
    assert!(markdown.contains("L1 动态换道步进记录"));
    assert!(markdown.contains("已核对 2 辆车的进入、前杠交接和车尾清空"));
    assert!(markdown.contains("未认证"));
    assert!(markdown.contains("implementation_commit=8b05abce"));
    assert!(markdown.contains("status=gate-witness-pass"));
    assert!(markdown.contains("workload_id=LF-ROUTING-G2-LINEAR-v1"));
    assert!(markdown.contains("已延后"));

    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(output.join("receipt.json")).expect("json"))
            .expect("receipt.json 解析");
    assert_eq!(json["format"], "lf-unified-gate-receipt-2");
    assert_eq!(json["mode"], "trial");
    assert_eq!(json["receipt"]["pending_items"][0]["axis"], "routing");
    std::fs::remove_dir_all(&output).ok();
}

#[test]
fn performance_pass_status_is_rejected() {
    let body = base_manifest().replace(
        r#"axis = "performance"
status = "uncertified-performance""#,
        r#"axis = "performance"
status = "semantic-pass""#,
    );
    let manifest = write_manifest(&body);
    let error = summary::run(&manifest, &ctx(), &output_dir("perf"), Mode::Trial, false)
        .expect_err("性能轴写成通过必须被拒绝");
    assert!(error.to_string().contains("uncertified-performance"));
}

#[test]
fn performance_verdict_word_is_enforced() {
    let body = base_manifest().replace(r#"verdict = "未认证""#, r#"verdict = "通过""#);
    let manifest = write_manifest(&body);
    let error = summary::run(
        &manifest,
        &ctx(),
        &output_dir("verdict"),
        Mode::Trial,
        false,
    )
    .expect_err("判定词必须被拒绝");
    assert!(error.to_string().contains("未认证"));
}

#[test]
fn missing_anchor_is_rejected() {
    let body = base_manifest().replace(
        r#"anchors = ["8b05abce", "150/150"]"#,
        r#"anchors = ["不存在的锚点"]"#,
    );
    let manifest = write_manifest(&body);
    let error = summary::run(&manifest, &ctx(), &output_dir("anchor"), Mode::Trial, false)
        .expect_err("锚点未命中必须被拒绝");
    assert!(error.to_string().contains("锚点未命中"));
}

#[test]
fn synth_identity_mismatch_is_rejected() {
    let body = base_manifest().replace(
        r#"expect = { implementation_commit = "8b05abce00000000000000000000000000000000", complete = true }"#,
        r#"expect = { implementation_commit = "0000000000000000000000000000000000000000" }"#,
    );
    let manifest = write_manifest(&body);
    let error = summary::run(&manifest, &ctx(), &output_dir("synth"), Mode::Trial, false)
        .expect_err("身份不符必须被拒绝");
    assert!(error.to_string().contains("身份不符"));
}

#[test]
fn require_complete_rejects_pending_cells() {
    let manifest = write_manifest(&base_manifest());
    let error = summary::run(
        &manifest,
        &ctx(),
        &output_dir("complete"),
        Mode::Trial,
        true,
    )
    .expect_err("未完成项必须被拒绝");
    assert!(error.to_string().contains("拒绝未完成项"));
}

#[test]
fn formal_mode_rejects_pending_status() {
    let manifest = write_manifest(&base_manifest());
    let error = summary::run(
        &manifest,
        &ctx(),
        &output_dir("formal"),
        Mode::Formal,
        false,
    )
    .expect_err("正式模式拒绝未完成格");
    assert!(error.to_string().contains("正式汇总拒绝未完成格"));
}

#[test]
fn evidence_root_placeholder_requires_root() {
    let manifest = write_manifest(&base_manifest());
    let no_root = PathContext {
        repo_root: fixtures().join("repo"),
        evidence_root: None,
    };
    let error = summary::run(
        &manifest,
        &no_root,
        &output_dir("rootless"),
        Mode::Trial,
        false,
    )
    .expect_err("占位路径必须被拒绝");
    assert!(error.to_string().contains("--evidence-root"));
}

#[test]
fn existing_output_directory_is_rejected() {
    let manifest = write_manifest(&base_manifest());
    let output = output_dir("exists");
    std::fs::create_dir_all(&output).expect("mkdir");
    let error = summary::run(&manifest, &ctx(), &output, Mode::Trial, false)
        .expect_err("输出目录必须不存在");
    assert!(error.to_string().contains("输出目录必须不存在"));
    std::fs::remove_dir_all(&output).ok();
}

#[test]
fn l1_record_identity_mismatch_is_rejected() {
    // 声明 100000-L1-33-clusters-1 却挂 10000-L1-16-clusters-1 的记录：必须拒绝（#1285 评审 P1）。
    // 同步改声明集，让失败落在记录身份比对而非恰好覆盖校验上。
    let body = base_manifest()
        .replace(
            r#"required_l1_identities = ["10000-L1-16-clusters-1"]"#,
            r#"required_l1_identities = ["100000-L1-33-clusters-1"]"#,
        )
        .replace(
            r#"identity = "10000-L1-16-clusters-1""#,
            r#"identity = "100000-L1-33-clusters-1""#,
        );
    let manifest = write_manifest(&body);
    let error = summary::run(
        &manifest,
        &ctx(),
        &output_dir("l1mismatch"),
        Mode::Trial,
        false,
    )
    .expect_err("记录身份错配必须被拒绝");
    assert!(error.to_string().contains("记录身份不符"));
}

#[test]
fn blank_exemption_scope_or_authorization_is_rejected() {
    // 授权豁免缺范围/授权文本：必须拒绝（#1285 评审 P2）。
    let body = base_manifest().replace(
        r#"axis = "snapshot"
status = "semantic-pass""#,
        r#"axis = "snapshot"
status = "authorized-exemption"
exception = { disposition = "authorized-exemption", scope = "", authorization = "" }"#,
    );
    let manifest = write_manifest(&body);
    let error = summary::run(
        &manifest,
        &ctx(),
        &output_dir("blankexempt"),
        Mode::Trial,
        false,
    )
    .expect_err("空白豁免字段必须被拒绝");
    assert!(
        error
            .to_string()
            .contains("授权豁免缺少未覆盖范围或维护者授权")
    );
}

#[test]
fn empty_evidence_list_on_passing_cell_is_rejected() {
    // 通过性格清空证据列表：必须拒绝（#1285 评审 P2）。
    let body = base_manifest().replace(
        r#"evidence = [ { kind = "repo-doc", path = "docs/mini-formal.md", anchors = ["300/300"] } ]
pending = []"#,
        r#"evidence = []
pending = []"#,
    );
    let manifest = write_manifest(&body);
    let error = summary::run(
        &manifest,
        &ctx(),
        &output_dir("noevidence"),
        Mode::Trial,
        false,
    )
    .expect_err("空证据列表必须被拒绝");
    assert!(error.to_string().contains("证据列表为空"));
}

#[test]
fn invalid_handoff_evidence_is_rejected() {
    use laneflow_gate_summary::evidence;
    use laneflow_gate_summary::manifest::Evidence;
    use serde_json::{Value, json};
    let stale_format =
        base_manifest().replace("lf-unified-gate-manifest-2", "lf-unified-gate-manifest-1");
    let error = summary::run(
        &write_manifest(&stale_format),
        &ctx(),
        &output_dir("archive-format"),
        Mode::Trial,
        false,
    )
    .unwrap_err();
    assert!(error.to_string().contains("manifest 格式不符"));
    let sources = fixtures().join("evidence-root");
    let read = |name: &str| -> Value {
        serde_json::from_slice(&std::fs::read(sources.join(name)).unwrap()).unwrap()
    };
    let root = tempfile::tempdir().unwrap();
    std::fs::copy(
        sources.join("input-manifest.json"),
        root.path().join("input.json"),
    )
    .unwrap();
    let context = PathContext {
        repo_root: root.path().to_owned(),
        evidence_root: None,
    };
    let anchor = Evidence::HandoffRun {
        path: "run.json".into(),
        input_manifest: "input.json".into(),
        implementation_commit: "c".repeat(40),
    };
    let original = read("l1.json");
    let resolve = |report: &Value| {
        std::fs::write(
            root.path().join("run.json"),
            serde_json::to_vec(report).unwrap(),
        )
        .unwrap();
        evidence::resolve(&anchor, &context)
    };
    assert!(resolve(&original).is_ok());
    for (pointer, replacement) in [
        ("/complete", json!(false)),
        (
            "/scope",
            json!("W1_W2_semantics_only_not_performance_or_product_certification"),
        ),
        ("/implementation_commit", json!("d".repeat(40))),
        ("/input_manifest/files/0/digest", json!("d".repeat(64))),
        (
            "/input_manifest/identity/case/RouteHandoff/clusters",
            json!(2),
        ),
        ("/validated_boundaries", json!(1)),
        ("/checkpoints/3/handoff_journal/5/2", json!(11)),
        ("/checkpoints/3/handoff_journal/3/3", json!(400)),
        ("/checkpoints/1/handoff_journal", json!([])),
    ] {
        let mut report = original.clone();
        *report.pointer_mut(pointer).unwrap() = replacement;
        assert!(resolve(&report).is_err(), "{pointer}");
    }
    assert!(resolve(&json!({"format":"lf-l1-topology-check-1","verdict":"clean"})).is_err());
}

#[test]
fn missing_required_l1_identity_is_rejected() {
    // 声明集含 100000-L1-33-clusters-1 但挂接缺失：必须拒绝（#1285 第三轮评审——
    // 漏检身份不得带着「已核查」外观进入结论）。
    let body = base_manifest().replace(
        r#"required_l1_identities = ["10000-L1-16-clusters-1"]"#,
        r#"required_l1_identities = ["10000-L1-16-clusters-1", "100000-L1-33-clusters-1"]"#,
    );
    let manifest = write_manifest(&body);
    let error = summary::run(
        &manifest,
        &ctx(),
        &output_dir("l1missing"),
        Mode::Formal,
        true,
    )
    .expect_err("缺失必需 L1 身份必须被拒绝");
    assert!(error.to_string().contains("缺少必需身份"));
    assert!(error.to_string().contains("100000-L1-33-clusters-1"));
}

#[test]
fn duplicate_l1_identity_is_rejected() {
    let repeated = base_manifest().replace(
        r#"required_l1_identities = ["10000-L1-16-clusters-1"]"#,
        r#"required_l1_identities = ["10000-L1-16-clusters-1","10000-L1-16-clusters-1"]"#,
    );
    let error = summary::run(
        &write_manifest(&repeated),
        &ctx(),
        &output_dir("l1repeated"),
        Mode::Trial,
        false,
    )
    .expect_err("必需输入身份重复");
    assert!(error.to_string().contains("身份重复"));
    // 同一身份挂两条记录：必须拒绝（每份输入身份恰一条）。
    let body = base_manifest().replace(
        r#"record = { kind = "handoff-run", path = "<evidence-root>/l1.json", input_manifest = "<evidence-root>/input-manifest.json", implementation_commit = "cccccccccccccccccccccccccccccccccccccccc" }"#,
        r#"record = { kind = "handoff-run", path = "<evidence-root>/l1.json", input_manifest = "<evidence-root>/input-manifest.json", implementation_commit = "cccccccccccccccccccccccccccccccccccccccc" }

[[l1_checks]]
identity = "10000-L1-16-clusters-1"
record = { kind = "handoff-run", path = "<evidence-root>/l1.json", input_manifest = "<evidence-root>/input-manifest.json", implementation_commit = "cccccccccccccccccccccccccccccccccccccccc" }"#,
    );
    let manifest = write_manifest(&body);
    let error = summary::run(&manifest, &ctx(), &output_dir("l1dup"), Mode::Trial, false)
        .expect_err("重复身份必须被拒绝");
    assert!(error.to_string().contains("重复声明身份"));
}

#[test]
fn nested_binary_identity_is_consumed() {
    // F1/F2/F3 生产端形状：binary={bytes,sha256} 嵌套（#1285 第四轮评审——
    // 此前只读顶层 binary_sha256，正式汇总无法绑定这些记录的被测二进制）。
    let body = base_manifest().replace(
        r#"{ kind = "synth-summary-json", path = "records/synth-summary.json", expect = { implementation_commit = "8b05abce00000000000000000000000000000000", complete = true } }"#,
        r#"{ kind = "synth-summary-json", path = "records/synth-summary-nested.json", expect = { implementation_commit = "8b05abce00000000000000000000000000000000", binary_sha256 = "be2a5ddd00000000", complete = true } }"#,
    );
    let manifest = write_manifest(&body);
    let output = output_dir("nested");
    let receipt =
        summary::run(&manifest, &ctx(), &output, Mode::Trial, false).expect("嵌套形状应可消费");
    let markdown = std::fs::read_to_string(output.join("receipt.md")).expect("receipt.md");
    assert!(markdown.contains("binary.sha256=be2a5ddd00000000"));
    let _ = receipt;
    std::fs::remove_dir_all(&output).ok();
}

#[test]
fn deferred_status_on_axis_cell_is_rejected() {
    // 轴格用 deferred 绕过未完成/豁免要求：必须拒绝（延后是负载级声明）。
    let body = base_manifest().replace(
        r#"axis = "routing"
status = "pending""#,
        r#"axis = "routing"
status = "deferred""#,
    );
    let manifest = write_manifest(&body);
    let error = summary::run(
        &manifest,
        &ctx(),
        &output_dir("deferred"),
        Mode::Formal,
        true,
    )
    .expect_err("轴格 deferred 必须被拒绝");
    assert!(error.to_string().contains("延后是负载级声明"));
}

#[test]
fn libtest_prefixed_routing_header_is_located() {
    // #1279 经 --nocapture 捕获的 stdout 首行带 libtest 前缀：
    // 标记不必在行首（词边界即可），身份键仍须命中。
    let body = base_manifest().replace(
        r#"{ kind = "routing-kv", path = "records/routing.txt", expect = ["workload_id=LF-ROUTING-G2-LINEAR-v1"] }"#,
        r#"{ kind = "routing-kv", path = "records/routing-libtest.txt", expect = ["workload_id=LF-ROUTING-G2-LINEAR-v1", "samples=21"] }"#,
    );
    let manifest = write_manifest(&body);
    let output = output_dir("libtest");
    let receipt =
        summary::run(&manifest, &ctx(), &output, Mode::Trial, false).expect("前缀头行应可定位");
    assert!(
        receipt.loads[0].cells[2].evidence[0]
            .detail
            .contains("workload_id=LF-ROUTING-G2-LINEAR-v1")
    );
    std::fs::remove_dir_all(&output).ok();
}

#[test]
fn missing_required_workload_is_rejected() {
    // 声明集含 LF-CN-URBAN-v1 但整个负载被删：必须拒绝（#1285 第五轮——
    // 负载级漏挂不能静默通过 formal --require-complete）。
    let body = base_manifest().replace(
        r#"required_loads = ["LF-SYNTH-v2", "LuST"]"#,
        r#"required_loads = ["LF-SYNTH-v2", "LF-CN-URBAN-v1", "LuST"]"#,
    );
    let manifest = write_manifest(&body);
    let error = summary::run(
        &manifest,
        &ctx(),
        &output_dir("loadmiss"),
        Mode::Formal,
        true,
    )
    .expect_err("缺失必需负载必须被拒绝");
    assert!(error.to_string().contains("缺少必需负载"));
    assert!(error.to_string().contains("LF-CN-URBAN-v1"));
}

#[test]
fn undeclared_workload_is_rejected() {
    // required_loads 之外的负载挂接：必须拒绝（多挂与删声明同样封堵）。
    let body = base_manifest().replace(
        r#"required_loads = ["LF-SYNTH-v2", "LuST"]"#,
        r#"required_loads = ["LF-SYNTH-v2"]"#,
    );
    let manifest = write_manifest(&body);
    let error = summary::run(
        &manifest,
        &ctx(),
        &output_dir("loadextra"),
        Mode::Trial,
        false,
    )
    .expect_err("未声明负载必须被拒绝");
    assert!(error.to_string().contains("含未声明负载"));
}

#[test]
fn blank_deferred_authorization_is_rejected() {
    // 延后负载清空授权：必须拒绝（#1285 第六轮——未经授权的延后不得
    // 以「五轴全部延后」满足 formal --require-complete）。
    let body =
        base_manifest().replace(r#"authorization = "测试延后授权""#, r#"authorization = """#);
    let manifest = write_manifest(&body);
    // trial 直达延后授权校验（该检查无条件生效，formal 含于其中）；
    // formal 下本例会被更早的 pending 格拒绝。
    let error = summary::run(
        &manifest,
        &ctx(),
        &output_dir("blankdefer"),
        Mode::Trial,
        false,
    )
    .expect_err("空授权延后必须被拒绝");
    assert!(error.to_string().contains("缺少维护者授权"));
}

/// 把 base manifest 的 routing 证据换成给定记录与期望键。
fn routing_manifest(path: &str, expect: &[&str]) -> std::path::PathBuf {
    let expect = expect
        .iter()
        .map(|item| format!("\"{item}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let body = base_manifest().replace(
        r#"{ kind = "routing-kv", path = "records/routing.txt", expect = ["workload_id=LF-ROUTING-G2-LINEAR-v1"] }"#,
        &format!(r#"{{ kind = "routing-kv", path = "{path}", expect = [{expect}] }}"#),
    );
    write_manifest(&body)
}

const LFSYNTH_EXPECT: [&str; 2] = [
    "workload_id=LF-ROUTING-CANDIDATE-LFSYNTH-v1",
    "identity=10000-W2-16-a",
];

#[test]
fn lfsynth_routing_rounds_are_all_checked() {
    // #1289：#1279 的 LF-SYNTH 入口输出 routing-lfsynth-wall-clock 头行；
    // 多轮拼接时每个头行都要命中，带 operation= 的统计行不算头行。
    let manifest = routing_manifest("records/routing-lfsynth.txt", &LFSYNTH_EXPECT);
    let output = output_dir("lfsynth");
    let receipt = summary::run(&manifest, &ctx(), &output, Mode::Trial, false)
        .expect("LF-SYNTH 头行应可定位");
    let detail = &receipt.loads[0].cells[2].evidence[0].detail;
    assert!(detail.contains("identity=10000-W2-16-a"), "{detail}");
    assert!(detail.contains("头行 2 个"), "{detail}");
    std::fs::remove_dir_all(&output).ok();
}

#[test]
fn lfsynth_routing_expect_must_bind_identity() {
    let manifest = routing_manifest(
        "records/routing-lfsynth.txt",
        &["workload_id=LF-ROUTING-CANDIDATE-LFSYNTH-v1"],
    );
    let error = summary::run(
        &manifest,
        &ctx(),
        &output_dir("lfsynth-noid"),
        Mode::Trial,
        false,
    )
    .expect_err("缺 identity 期望键必须被拒绝");
    assert!(
        error.to_string().contains("expect 须包含 identity"),
        "{error}"
    );
}

#[test]
fn lfsynth_routing_round_with_other_identity_is_rejected() {
    let manifest = routing_manifest(
        "records/routing-lfsynth-mixed-identity.txt",
        &LFSYNTH_EXPECT,
    );
    let error = summary::run(
        &manifest,
        &ctx(),
        &output_dir("lfsynth-mixid"),
        Mode::Trial,
        false,
    )
    .expect_err("任一轮身份不符必须被拒绝");
    assert!(
        error.to_string().contains("identity=100000-W2-33-a"),
        "{error}"
    );
}

#[test]
fn mixed_routing_header_markers_are_rejected() {
    let manifest = routing_manifest("records/routing-mixed-marker.txt", &LFSYNTH_EXPECT);
    let error = summary::run(
        &manifest,
        &ctx(),
        &output_dir("mixed-marker"),
        Mode::Trial,
        false,
    )
    .expect_err("混合头行必须被拒绝");
    assert!(error.to_string().contains("混有两种墙钟头行"), "{error}");
}

#[test]
fn routing_statistics_lines_are_not_headers() {
    let manifest = routing_manifest("records/routing-lfsynth-stats-only.txt", &LFSYNTH_EXPECT);
    let error = summary::run(
        &manifest,
        &ctx(),
        &output_dir("stats-only"),
        Mode::Trial,
        false,
    )
    .expect_err("只有统计行必须被拒绝");
    assert!(error.to_string().contains("记录缺少墙钟头行"), "{error}");
}
