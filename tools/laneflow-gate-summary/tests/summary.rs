//! 汇编器集成测试：fixture manifest → 回执；以及失败关闭路径。
//!
//! fixture 记录在 `tests/fixtures/`（repo/ 充当仓库根、evidence-root/ 充当
//! 仓库外证据根）；真实试跑见 `fixtures/v0.10-trial/` 与 README。

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

fn base_manifest() -> String {
    r##"
format = "lf-unified-gate-manifest-1"
title = "测试汇总"
mode_note = "单元测试 fixture"
required_l1_identities = ["10000-L1-16"]

[global]
single_world_statement = "不以多世界吞吐替代单世界扩展语义。"

[global.performance]
verdict = "未认证"
product_status = "Product TBD / Uncertified"
real_budget_failures = 0
authorized_exceptions = 0
contract_level_exceedances = ["测试超标条目（研究观察）"]
receipt = "https://github.com/illusion-tech/laneflow/issues/1266#issuecomment-6651487168"
evidence = [ { kind = "repo-doc", path = "research/mini-formal.md", anchors = ["150/150"] } ]

[global.allocation_diagnostics]
statement = "测试 allocation 口径"
evidence = [ { kind = "repo-doc", path = "research/mini-formal.md" } ]

[global.checkpoint_verification_cost]
statement = "测试 checkpoint 成本"
evidence = [ { kind = "repo-doc", path = "research/mini-formal.md" } ]

[[scope_adjudication]]
item = "worker 两两组合"
disposition = "授权豁免"
uncovered = "测试未覆盖范围"
authorization = "测试授权"

[[capability_limits]]
statement = "测试能力限制"
boundary = "测试边界"
evidence = [ { kind = "github", url = "https://github.com/illusion-tech/laneflow/issues/1271" } ]

[[l1_checks]]
identity = "10000-L1-16"
record = { kind = "l1-record", path = "<evidence-root>/l1.json", expect = { verdict = "clean" } }

[[loads]]
id = "LF-SYNTH-v2"
kind = "axes"
input_identity = "测试输入身份（10 身份）"

[[loads.cells]]
axis = "behavior"
status = "semantic-pass"
conclusion = "测试 behavior 结论"
source_commit = "8b05abce00000000000000000000000000000000"
binary = "bin9/fixture"
input_identity = "同负载输入身份"
coverage = "测试覆盖边界"
evidence = [
  { kind = "repo-doc", path = "research/mini-formal.md", anchors = ["8b05abce", "150/150"] },
  { kind = "synth-summary-json", path = "records/synth-summary.json", expect = { implementation_commit = "8b05abce00000000000000000000000000000000", complete = true } },
]
pending = []

[[loads.cells]]
axis = "determinism"
status = "semantic-pass"
conclusion = "测试 determinism 结论"
source_commit = "8b05abce00000000000000000000000000000000"
binary = "bin9/fixture"
input_identity = "同负载输入身份"
coverage = "测试覆盖边界"
evidence = [ { kind = "gate-json", path = "records/gate.json", expect = { status = "gate-witness-pass", commit = "85ec589200000000000000000000000000000000" } } ]
pending = []

[[loads.cells]]
axis = "performance"
status = "uncertified-performance"
conclusion = "测试 performance 结论：未认证"
source_commit = "8b05abce00000000000000000000000000000000"
binary = "bin9/fixture"
input_identity = "同负载输入身份"
coverage = "测试覆盖边界"
evidence = [ { kind = "routing-kv", path = "records/routing.txt", expect = ["workload_id=LF-ROUTING-G2-LINEAR-v1"] } ]
pending = []

[[loads.cells]]
axis = "snapshot"
status = "semantic-pass"
conclusion = "测试 snapshot 结论"
source_commit = "8b05abce00000000000000000000000000000000"
binary = "bin9/fixture"
input_identity = "同负载输入身份"
coverage = "测试覆盖边界"
evidence = [ { kind = "repo-doc", path = "research/mini-formal.md", anchors = ["300/300"] } ]
pending = []

[[loads.cells]]
axis = "routing"
status = "pending"
conclusion = "测试 routing 待 #1279"
evidence = [ { kind = "github", url = "https://github.com/illusion-tech/laneflow/issues/1279" } ]
pending = ["#1279 计时记录（冻结后运行）"]

[[loads]]
id = "LuST"
kind = "deferred"
note = "五轴均未在 LuST 上验证，不记为通过"
authorization = "测试延后授权"
"##
    .to_owned()
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
    assert!(markdown.contains("lf-unified-gate-receipt-1"));
    assert!(markdown.contains("试跑结果不作为 v0.10 结论"));
    assert!(markdown.contains("范围裁定清单"));
    assert!(markdown.contains("worker 两两组合"));
    assert!(markdown.contains("能力限制登记"));
    assert!(markdown.contains("L1 输入 #1271 拓扑核查"));
    assert!(markdown.contains("verdict=clean"));
    assert!(markdown.contains("未认证"));
    assert!(markdown.contains("implementation_commit=8b05abce"));
    assert!(markdown.contains("status=gate-witness-pass"));
    assert!(markdown.contains("workload_id=LF-ROUTING-G2-LINEAR-v1"));
    assert!(markdown.contains("已延后 v0.11"));

    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(output.join("receipt.json")).expect("json"))
            .expect("receipt.json 解析");
    assert_eq!(json["format"], "lf-unified-gate-receipt-1");
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
    // 声明 100000-L1-33 却挂 10000-L1-16 的记录：必须拒绝（#1285 评审 P1）。
    // 同步改声明集，让失败落在记录身份比对而非恰好覆盖校验上。
    let body = base_manifest()
        .replace(
            r#"required_l1_identities = ["10000-L1-16"]"#,
            r#"required_l1_identities = ["100000-L1-33"]"#,
        )
        .replace(
            r#"identity = "10000-L1-16""#,
            r#"identity = "100000-L1-33""#,
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
        r#"evidence = [ { kind = "repo-doc", path = "research/mini-formal.md", anchors = ["300/300"] } ]
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
fn violations_found_l1_record_is_rejected_even_without_expect() {
    // 不带 expect.verdict 的 violations-found 记录：必须无条件拒绝
    // （#1285 第二轮评审 P2——含 #1271 拓扑的输入不得用于安全验收）。
    let body = base_manifest().replace(
        r#"record = { kind = "l1-record", path = "<evidence-root>/l1.json", expect = { verdict = "clean" } }"#,
        r#"record = { kind = "l1-record", path = "<evidence-root>/l1-violations.json" }"#,
    );
    let manifest = write_manifest(&body);
    // trial + 不设 require-complete：绕开 pending 格的前置拒绝，直达证据解析，
    // 证明 clean 是无条件要求（formal 模式包含 trial 的全部检查）。
    let error = summary::run(&manifest, &ctx(), &output_dir("l1viol"), Mode::Trial, false)
        .expect_err("violations-found 记录必须被拒绝");
    assert!(error.to_string().contains("必须为 clean"));
}

#[test]
fn missing_required_l1_identity_is_rejected() {
    // 声明集含 100000-L1-33 但挂接缺失：必须拒绝（#1285 第三轮评审——
    // 漏检身份不得带着「已核查」外观进入结论）。
    let body = base_manifest().replace(
        r#"required_l1_identities = ["10000-L1-16"]"#,
        r#"required_l1_identities = ["10000-L1-16", "100000-L1-33"]"#,
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
    assert!(error.to_string().contains("100000-L1-33"));
}

#[test]
fn duplicate_l1_identity_is_rejected() {
    // 同一身份挂两条记录：必须拒绝（每份输入身份恰一条）。
    let body = base_manifest().replace(
        r#"record = { kind = "l1-record", path = "<evidence-root>/l1.json", expect = { verdict = "clean" } }"#,
        r#"record = { kind = "l1-record", path = "<evidence-root>/l1.json", expect = { verdict = "clean" } }

[[l1_checks]]
identity = "10000-L1-16"
record = { kind = "l1-record", path = "<evidence-root>/l1.json" }"#,
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
