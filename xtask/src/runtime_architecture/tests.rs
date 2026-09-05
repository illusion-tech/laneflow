use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{SourceInputs, dependencies, sources};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Temporary(PathBuf);
impl Temporary {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "laneflow-architecture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn write(&self, relative: &str, contents: &str) {
        let file = self.0.join(relative);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, contents).unwrap();
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

// 独立的可编译输入；具体类型与三函数合同一致，函数体只提供类型占位。
const ADMISSION: &str = r#"
use crate::admin::cutover::{SemanticDiffOriginBinding, CutoverDescriptorError};
use crate::admin::snapshot::CapturedSnapshot;
use crate::admin::snapshot_restore::{RestoredSnapshot, SnapshotRestoreError, SnapshotRestoreLimits};
use crate::facade::source::CommittedNetworkSource;
use crate::kernel::config::WorldConfig;
use laneflow_static_network::{CanonicalNetworkOrigin, SharedNetworkRevision};
use std::sync::Arc;
pub(super) fn verify_semantic_diff(binding: Option<&SemanticDiffOriginBinding>, bytes: &[u8], base: CanonicalNetworkOrigin, target: CanonicalNetworkOrigin) -> Result<(), CutoverDescriptorError> { todo!() }
pub(super) fn encode_lfrs(snapshot: &CapturedSnapshot) -> Vec<u8> { todo!() }
pub(super) fn restore_lfrs(bytes: &[u8], revision: Arc<SharedNetworkRevision>, source: CommittedNetworkSource, config: WorldConfig, limits: SnapshotRestoreLimits) -> Result<RestoredSnapshot, SnapshotRestoreError> { todo!() }
"#;

struct SourceFixture {
    directory: Temporary,
    inputs: SourceInputs,
}
impl SourceFixture {
    fn new(kernel: &str, root_extra: &str) -> Self {
        let directory = Temporary::new();
        directory.write(
            "lib.rs",
            &format!(
                "mod kernel; mod admin; mod facade; pub use facade::TrafficWorld; {root_extra}"
            ),
        );
        directory.write(
            "kernel/mod.rs",
            &format!("pub(crate) mod config {{ pub struct WorldConfig; }} {kernel}"),
        );
        directory.write("admin/mod.rs", r#"
pub(crate) mod format_admission;
pub(crate) mod cutover { pub struct SemanticDiffOriginBinding; pub struct CutoverDescriptorError; }
pub(crate) mod snapshot { pub struct CapturedSnapshot; impl crate::TrafficWorld { pub fn capture_snapshot(&self) {} } }
pub(crate) mod snapshot_restore { pub struct RestoredSnapshot; pub struct SnapshotRestoreError; pub struct SnapshotRestoreLimits; }
"#);
        directory.write("admin/format_admission.rs", ADMISSION);
        directory.write(
            "facade/mod.rs",
            "pub struct TrafficWorld; pub(crate) mod source { pub struct CommittedNetworkSource; }",
        );
        let inputs = SourceInputs {
            entry: directory.0.join("lib.rs"),
            package_root: directory.0.clone(),
            externals: [
                "std",
                "core",
                "alloc",
                "laneflow_static_network",
                "laneflow_format",
                "laneflow_runtime_snapshot_wire",
                "renamed_format",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            formats: [
                "laneflow_format",
                "laneflow_runtime_snapshot_wire",
                "renamed_format",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        };
        Self { directory, inputs }
    }
    fn check(&self) -> Result<(), String> {
        sources::check(&self.inputs)
    }
    fn admission(&self, source: &str) {
        self.directory.write("admin/format_admission.rs", source);
    }
    fn compile(&self) {
        self.compile_with(&[]);
    }
    fn compile_with(&self, extra_args: &[&str]) {
        self.directory.write(
            "static.rs",
            "pub struct SharedNetworkRevision; pub struct CanonicalNetworkOrigin;",
        );
        let library = self.directory.0.join("liblaneflow_static_network.rlib");
        rustc(
            &self.directory.0.join("static.rs"),
            &["--crate-type=rlib", "--crate-name=laneflow_static_network"],
            &library,
            &[],
        );
        self.directory.write("wire.rs", "pub struct Raw;");
        let wire = self
            .directory
            .0
            .join("liblaneflow_runtime_snapshot_wire.rlib");
        rustc(
            &self.directory.0.join("wire.rs"),
            &[
                "--crate-type=rlib",
                "--crate-name=laneflow_runtime_snapshot_wire",
            ],
            &wire,
            &[],
        );
        let mut args = vec![
            "--crate-type=lib",
            "--crate-name=fixture",
            "--emit=metadata",
        ];
        args.extend_from_slice(extra_args);
        rustc(
            &self.inputs.entry,
            &args,
            &self.directory.0.join("fixture.rmeta"),
            &[
                ("laneflow_static_network", &library),
                ("laneflow_runtime_snapshot_wire", &wire),
            ],
        );
    }
}

fn rustc(source: &Path, args: &[&str], output: &Path, externals: &[(&str, &Path)]) {
    let mut command = Command::new("rustc");
    command
        .args(["--edition=2024", "-A", "warnings"])
        .args(args)
        .arg(source)
        .arg("-o")
        .arg(output);
    for (name, path) in externals {
        command
            .arg("--extern")
            .arg(format!("{name}={}", path.display()));
    }
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn legal_tree_compiles_and_accepts_method_calls_outside_the_checkers_contract() {
    let fixture = SourceFixture::new(
        "fn ordinary(world: &crate::TrafficWorld) { world.capture_snapshot(); crate::TrafficWorld::capture_snapshot(world); }",
        "",
    );
    fixture.compile();
    fixture.check().unwrap();
}

#[test]
fn raw_identifiers_use_the_same_module_import_and_interface_rules() {
    let fixture = SourceFixture::new(
        "use crate::r#kernel::r#config as r#settings; fn ordinary(_: r#settings::WorldConfig) {}",
        "",
    );
    fixture.directory.write(
        "lib.rs",
        "mod r#kernel; mod r#admin; mod r#facade; pub use r#facade::TrafficWorld;",
    );
    fixture.admission(
        &ADMISSION
            .replace("crate::admin", "crate::r#admin")
            .replace("fn encode_lfrs", "fn r#encode_lfrs")
            .replace("Vec<u8>", "r#Vec<r#u8>"),
    );
    fixture.compile();
    fixture.check().unwrap();
    for source in [
        "fn bad() { let _ = crate::r#admin::r#snapshot::CapturedSnapshot; }",
        "use crate::r#admin as r#control; fn bad() { let _ = r#control::snapshot::CapturedSnapshot; }",
        "macro_rules! bad { () => { crate::r#admin::snapshot::CapturedSnapshot } } fn call() { let _ = bad!(); }",
        "use crate::r#Captured;",
    ] {
        let fixture = SourceFixture::new(
            source,
            "pub use r#admin::snapshot::CapturedSnapshot as r#Captured;",
        );
        fixture.compile();
        assert!(fixture.check().unwrap_err().contains("禁止依赖"));
    }
}

#[test]
fn extern_crate_self_is_rejected_at_module_and_block_scope() {
    for source in [
        "extern crate self as runtime; use runtime::admin::snapshot;",
        "fn bad() { extern crate self as runtime; let _ = runtime::admin::snapshot::CapturedSnapshot; }",
    ] {
        let fixture = SourceFixture::new(source, "");
        fixture.compile();
        assert!(fixture.check().unwrap_err().contains("extern crate self"));
    }
    let fixture = SourceFixture::new("#[cfg(test)] extern crate self as runtime;", "");
    fixture.compile();
    fixture.check().unwrap();
}

#[test]
fn extra_root_module_and_root_business_code_are_rejected() {
    for extra in [
        "mod legacy { use crate::admin::snapshot; }",
        "fn business() {}",
    ] {
        let fixture = SourceFixture::new("", extra);
        fixture.compile();
        assert!(fixture.check().is_err());
    }
}

#[test]
fn actual_entry_and_physical_group_directories_are_checked() {
    let inline = SourceFixture::new("", "");
    let entry = fs::read_to_string(&inline.inputs.entry).unwrap().replace(
        "mod kernel;",
        "mod kernel { pub(crate) mod config { pub struct WorldConfig; } }",
    );
    fs::write(&inline.inputs.entry, entry).unwrap();
    inline.compile();
    assert!(inline.check().unwrap_err().contains("kernel/"));
    let mut fixture = SourceFixture::new("", "");
    fixture.inputs.entry = fixture.directory.0.join("entry.rs");
    fixture.directory.write(
        "entry.rs",
        "mod kernel; mod admin; mod facade; pub use facade::TrafficWorld; mod legacy {}",
    );
    fixture.compile();
    assert!(fixture.check().unwrap_err().contains("根模块"));
    fixture.directory.write(
        "entry.rs",
        "mod kernel; mod admin; mod facade; pub use facade::TrafficWorld;",
    );
    let kernel = fs::read_to_string(fixture.directory.0.join("kernel/mod.rs")).unwrap();
    fs::remove_file(fixture.directory.0.join("kernel/mod.rs")).unwrap();
    fixture.directory.write("kernel.rs", &kernel);
    fixture.compile();
    assert!(fixture.check().unwrap_err().contains("kernel/"));
}

#[test]
fn admission_rejects_extra_visible_interfaces_including_traits() {
    let fixture = SourceFixture::new("", "");
    // 这些声明均是合法 Rust；即使没有引用 wire，额外出口也违反有限接口合同。
    for extra in [
        "pub(crate) trait Raw { fn view(&self) -> u32; }",
        "pub(crate) type Raw = u32;",
        "pub(crate) use std::vec::Vec;",
        "pub(crate) struct Raw;",
        "pub(super) fn extra() {}",
        "pub(crate) mod extra {}",
        "const PRIVATE: u32 = 0; pub(crate) const EXPORTED: u32 = PRIVATE;",
    ] {
        fixture.admission(&format!("{ADMISSION}\n{extra}"));
        fixture.compile();
        assert!(fixture.check().is_err(), "{extra}");
    }
}

#[test]
fn admission_checks_concrete_types_and_complete_function_inventory() {
    let fixture = SourceFixture::new("", "");
    for mutated in [
        ADMISSION.replace("-> Vec<u8>", "-> impl Iterator<Item = u8>"), // 语法夹具：todo! 的推断不足，不用于编译证据。
        ADMISSION.replace("-> Vec<u8>", "-> u32"),
        ADMISSION.replace("fn encode_lfrs(", "fn encode_lfrs<T>("),
        ADMISSION.replace("pub(super) fn encode_lfrs", "fn encode_lfrs"),
        ADMISSION.replace("pub(super) fn encode_lfrs", "pub(crate) fn encode_lfrs"),
    ] {
        fixture.admission(&mutated);
        assert!(fixture.check().is_err());
    }
    for shadow in ["Vec", "r#Vec"] {
        fixture.admission(&format!("{ADMISSION}\nstruct {shadow}<T>(T);"));
        fixture.compile();
        assert!(fixture.check().unwrap_err().contains("具体接口合同"));
    }
}

#[test]
fn explicit_format_paths_and_management_reexports_are_rejected() {
    // 只测试路径语法；Reader 等为模拟名称。可编译的接口/模块拒绝证据见上面的用例。
    for source in [
        "use laneflow_format as format;",
        "fn bad() { ::laneflow_runtime_snapshot_wire::read(); }",
        "use renamed_format::Reader;",
        "use r#renamed_format::Reader;",
        "macro_rules! bad { () => { laneflow_format::read() } }",
        "macro_rules! bad { () => { r#laneflow_format::read() } }",
        "use crate::Captured;",
    ] {
        let fixture = SourceFixture::new(
            source,
            "pub use admin::snapshot::CapturedSnapshot as Captured;",
        );
        assert!(fixture.check().unwrap_err().contains("禁止依赖"));
    }
    let fixture = SourceFixture::new("", "");
    fixture.admission(&format!("{ADMISSION}\npub(crate) trait Raw {{ fn view(&self) -> laneflow_runtime_snapshot_wire::Reader; }}"));
    assert!(fixture.check().is_err());
}

#[test]
fn test_only_fixtures_are_excluded_and_unknown_production_cfg_is_checked() {
    let fixture = SourceFixture::new(
        "#[cfg(test)] mod tests { use laneflow_format::*; } #[cfg(all(test, feature = \"extra\"))] mod missing;",
        "",
    );
    fixture.admission(&format!(
        "{ADMISSION}\nstruct Private; impl Private {{ #[cfg(test)] pub fn helper() {{}} }}"
    ));
    fixture.compile();
    fixture.check().unwrap();
    for cfg in ["any(test, feature = \"extra\")", "not(test)"] {
        let fixture =
            SourceFixture::new(&format!("#[cfg({cfg})] use laneflow_format::Reader;"), "");
        assert!(fixture.check().is_err());
    }
}

#[test]
fn incomplete_or_unsupported_source_inputs_fail() {
    let local = SourceFixture::new("fn outer() { mod inner {} }", "");
    local.compile();
    assert!(local.check().unwrap_err().contains("模块层级"));
    for source in [
        "mod missing;",
        "fn broken(",
        "include!(\"hidden.rs\");",
        "use crate::admin::*;",
        "#[path = \"elsewhere.rs\"] mod hidden;",
    ] {
        assert!(SourceFixture::new(source, "").check().is_err());
    }
}

#[test]
fn include_macro_loading_is_rejected_through_qualified_paths_and_imports() {
    for source in [
        "std::include!(\"hidden.rs\");",
        "::std::r#include!(\"hidden.rs\");",
        "use std::include as load; load!(\"hidden.rs\");",
        "use std as library; use library::include as load; load!(\"hidden.rs\");",
        "fn outer() { use std::include as load; load!(\"hidden_expr.rs\"); }",
        "macro_rules! load { () => { std::include!(\"hidden.rs\"); } } load!();",
        "macro_rules! load { () => { use std::include as hidden; hidden!(\"hidden.rs\"); } } load!();",
        // 合同保守保留该导入名，不扩展宏命名空间解析。
        "mod util { pub fn include() {} } use self::util::include; fn call() { include(); }",
    ] {
        let fixture = SourceFixture::new(source, "");
        fixture.directory.write(
            "kernel/hidden.rs",
            "fn bad() { let _ = crate::admin::snapshot::CapturedSnapshot; }",
        );
        fixture.directory.write(
            "kernel/hidden_expr.rs",
            "{ let _ = crate::admin::snapshot::CapturedSnapshot; }",
        );
        fixture.compile();
        assert!(fixture.check().unwrap_err().contains("include"));
    }
    let fixture = SourceFixture::new(
        "const TEXT: &str = std::include_str!(\"notes.txt\"); const BYTES: &[u8] = std::include_bytes!(\"notes.txt\");",
        "",
    );
    fixture.directory.write("kernel/notes.txt", "fixture");
    fixture.compile();
    fixture.check().unwrap();
}

#[test]
fn admission_rejects_possible_conditional_macro_exports() {
    for attribute in [
        "#[cfg_attr(not(test), macro_export)]",
        "#[cfg_attr(not(test), cfg_attr(not(test), macro_export))]",
        "#[cfg_attr(feature = \"exports\", macro_export)]",
    ] {
        let fixture = SourceFixture::new("", "");
        fixture.admission(&format!(
            "{ADMISSION}\n{attribute} macro_rules! extra {{ () => {{}} }}"
        ));
        fixture.directory.write("facade/mod.rs", "pub struct TrafficWorld; pub(crate) mod source { pub struct CommittedNetworkSource; } pub fn call_export() { crate::extra!(); }");
        fixture.compile_with(&["--cfg", "feature=\"exports\""]);
        assert!(fixture.check().unwrap_err().contains("可见声明"));
    }
    for attribute in [
        "#[cfg_attr(test, macro_export)]",
        "#[cfg_attr(not(test), allow(unused_macros))]",
    ] {
        let fixture = SourceFixture::new("", "");
        fixture.admission(&format!(
            "{ADMISSION}\n{attribute} macro_rules! helper {{ () => {{}} }}"
        ));
        fixture.compile();
        fixture.check().unwrap();
    }
}

#[test]
fn conditional_imports_reject_distinct_targets_without_selecting_a_configuration() {
    let original = "use crate::admin::snapshot::CapturedSnapshot;";
    let raw = "#[cfg(not(feature = \"expected\"))] use laneflow_runtime_snapshot_wire::Raw as CapturedSnapshot;";
    let expected = "#[cfg(feature = \"expected\")] use crate::admin::snapshot::CapturedSnapshot;";
    for imports in [format!("{raw}\n{expected}"), format!("{expected}\n{raw}")] {
        let fixture = SourceFixture::new("", "");
        fixture.admission(&format!(
            "{}\n#[cfg(not(feature = \"expected\"))] fn prove_raw_signature() {{ let _: fn(&laneflow_runtime_snapshot_wire::Raw) -> Vec<u8> = encode_lfrs; }}",
            ADMISSION.replace(original, &imports)
        ));
        fixture.compile();
        fixture.compile_with(&["--cfg", "feature=\"expected\""]);
        assert!(fixture.check().unwrap_err().contains("别名歧义"));
    }
    let fixture = SourceFixture::new("", "");
    fixture.admission(&ADMISSION.replace(
        original,
        &format!("#[cfg(not(feature = \"expected\"))] {original}\n{expected}"),
    ));
    fixture.compile();
    fixture.compile_with(&["--cfg", "feature=\"expected\""]);
    fixture.check().unwrap();
}

#[test]
fn test_only_nested_declarations_are_excluded_consistently() {
    let declarations = r#"
struct Local;
impl Local {
    #[cfg(test)] const RAW: Option<laneflow_runtime_snapshot_wire::Raw> = None;
    #[cfg(test)] fn helper(_: laneflow_runtime_snapshot_wire::Raw) {}
}
trait Fixture {
    #[cfg(test)] const RAW: Option<laneflow_runtime_snapshot_wire::Raw> = None;
    #[cfg(test)] type Raw: Into<laneflow_runtime_snapshot_wire::Raw>;
    #[cfg(test)] fn helper(_: laneflow_runtime_snapshot_wire::Raw) {}
}
struct Fields { #[cfg(test)] raw: laneflow_runtime_snapshot_wire::Raw, live: u8 }
enum Variants { Live, #[cfg(test)] Raw(laneflow_runtime_snapshot_wire::Raw) }
unsafe extern "C" { #[cfg(test)] fn extra(raw: *const laneflow_runtime_snapshot_wire::Raw); }
"#;
    let fixture = SourceFixture::new(declarations, "");
    fixture.compile();
    fixture.compile_with(&["--cfg", "test"]);
    fixture.check().unwrap();
    // unknown feature 不能像 test-only 一样排除；启用后仍是可编译的生产声明。
    let fixture = SourceFixture::new(
        &declarations.replace("cfg(test)", "cfg(any(test, feature = \"extra\"))"),
        "",
    );
    fixture.compile_with(&["--cfg", "feature=\"extra\""]);
    assert!(fixture.check().unwrap_err().contains("禁止依赖"));
}

struct CargoFixture(Temporary);
impl CargoFixture {
    fn new(extra: &str) -> Self {
        let directory = Temporary::new();
        directory.write("Cargo.toml", "[workspace]\nmembers = [\"runtime\", \"spatial\", \"adapter\"]\nexclude = [\"vendor/helper-a\", \"vendor/helper-b\", \"vendor/bevy\"]\nresolver = \"3\"\n");
        let fixture = Self(directory);
        fixture.package("runtime", "laneflow-runtime", "0.0.0", &format!("[lib]\npath = \"src/entry.rs\"\n[dependencies]\nrenamed_helper = {{ package = \"helper\", path = \"../vendor/helper-a\" }}\n[features]\nbridge = [\"renamed_helper/extra\"]\n{extra}"));
        fixture.0.write("runtime/src/entry.rs", "");
        fixture.package("spatial", "laneflow-spatial", "0.0.0", "");
        fixture.package("adapter", "laneflow-bevy", "0.0.0", "[dependencies]\nlaneflow-runtime = { path = \"../runtime\" }\nlaneflow-spatial = { path = \"../spatial\" }\nother_helper = { package = \"helper\", path = \"../vendor/helper-b\" }\n");
        fixture.package("vendor/helper-a", "helper", "1.0.0", "[dependencies]\nbevy = { path = \"../bevy\", optional = true }\n[features]\nextra = [\"dep:bevy\"]\n");
        fixture.package(
            "vendor/helper-b",
            "helper",
            "2.0.0",
            "[dependencies]\nbevy = { path = \"../bevy\" }\n",
        );
        fixture.package("vendor/bevy", "bevy", "0.0.0", "");
        fixture.cargo(&["generate-lockfile", "--offline"]);
        fixture
    }
    fn package(&self, path: &str, name: &str, version: &str, extra: &str) {
        self.0.write(
            &format!("{path}/Cargo.toml"),
            &format!(
                "[package]\nname = \"{name}\"\nversion = \"{version}\"\nedition = \"2024\"\n{extra}"
            ),
        );
        self.0.write(&format!("{path}/src/lib.rs"), "");
    }
    fn cargo(&self, args: &[&str]) -> Vec<u8> {
        let output = Command::new("cargo")
            .args(args)
            .current_dir(&self.0.0)
            .env("CARGO_TARGET_DIR", self.0.0.join("target"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }
    fn load(&self, all_features: bool) -> dependencies::Metadata {
        dependencies::load(&self.0.0.join("Cargo.toml"), all_features).unwrap()
    }
}

#[test]
fn real_cargo_graph_keeps_package_ids_and_traverses_external_features() {
    let fixture = CargoFixture::new("");
    fixture.cargo(&[
        "check",
        "--workspace",
        "--all-features",
        "--locked",
        "--offline",
    ]);
    let default = fixture.load(false);
    default.check().unwrap(); // helper 2 -> bevy 仅从 Adapter 可达，不得覆盖 helper 1 的节点。
    assert!(
        default
            .source_inputs()
            .unwrap()
            .entry
            .ends_with("src/entry.rs")
    );
    let error = fixture.load(true).check().unwrap_err();
    assert!(
        error.contains("laneflow-runtime -> helper -> bevy"),
        "{error}"
    );
}

#[test]
fn real_cargo_declarations_distinguish_dev_optional_and_target_build_edges() {
    let dev =
        CargoFixture::new("[dev-dependencies]\nlaneflow-spatial = { path = \"../spatial\" }\n");
    dev.load(false).check().unwrap();
    for extra in [
        "[target.'cfg(windows)'.build-dependencies]\nbevy = { path = \"../vendor/bevy\" }\n",
        "[target.'cfg(unix)'.dependencies]\nbevy = { path = \"../vendor/bevy\", optional = true }\n",
    ] {
        let fixture = CargoFixture::new(extra);
        assert!(
            fixture
                .load(false)
                .check()
                .unwrap_err()
                .contains("laneflow-runtime -> bevy")
        );
    }
}

#[test]
fn missing_resolved_nodes_or_resolve_input_are_not_empty_graphs() {
    let fixture = CargoFixture::new("");
    let bytes = fixture.cargo(&["metadata", "--locked", "--offline", "--format-version", "1"]);
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let helper = value["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|package| package["name"] == "helper" && package["version"] == "1.0.0")
        .unwrap()["id"]
        .clone();
    value["resolve"]["nodes"]
        .as_array_mut()
        .unwrap()
        .retain(|node| node["id"] != helper);
    let missing: dependencies::Metadata = serde_json::from_value(value.clone()).unwrap();
    assert!(missing.check().unwrap_err().contains("已解析节点缺失"));
    value["resolve"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<dependencies::Metadata>(value).is_err());
}
