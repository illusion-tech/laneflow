//! 生成 §9.2 27 行 workload manifest：从冻结 fixture 容器逐行编译，Draft 2020-12
//! schema 校验通过后写入 `docs/reference/`，并打印字节身份（byte length + SHA-256）。

use std::path::PathBuf;

use issue_296_geometry_frontend_calibration::container::sha256_hex;
use issue_296_geometry_frontend_calibration::manifest::{
    CORRIDOR_FIXTURE_PATH, CORRIDOR_WORKLOAD_ID, MIN_FIXTURE_PATH, MIN_WORKLOAD_ID,
    P100_FIXTURE_PATH, P100_WORKLOAD_ID, build_manifest, load_fixture, validate_manifest,
};

const MANIFEST_PATH: &str =
    "docs/reference/geometry-frontend-calibration-workload-manifest-v1.json";
const MANIFEST_SCHEMA_PATH: &str =
    "docs/reference/geometry-frontend-calibration-workload-manifest-v1.schema.json";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo 根必须存在")
}

fn main() {
    let root = repo_root();
    let fixtures = [
        load_fixture(&root, MIN_FIXTURE_PATH, MIN_WORKLOAD_ID),
        load_fixture(&root, CORRIDOR_FIXTURE_PATH, CORRIDOR_WORKLOAD_ID),
        load_fixture(&root, P100_FIXTURE_PATH, P100_WORKLOAD_ID),
    ];
    let manifest = build_manifest(&fixtures);
    eprintln!("27 行编译完成");
    let schema_bytes =
        std::fs::read(root.join(MANIFEST_SCHEMA_PATH)).expect("读取 manifest schema 失败");
    validate_manifest(&schema_bytes, manifest.as_bytes());
    std::fs::write(root.join(MANIFEST_PATH), &manifest).expect("写 manifest 失败");
    println!(
        "{MANIFEST_PATH}\t{}\t{}",
        manifest.len(),
        sha256_hex(manifest.as_bytes())
    );
    eprintln!("manifest 生成并通过 Draft 2020-12 校验");
}
