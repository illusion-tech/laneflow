//! §9.2 cross-record validator 入口：trusted contract → 四件工件 exact bytes →
//! Draft 2020-12 → oracle 重编译，全部通过时打印各工件字节身份。

use std::path::PathBuf;

use issue_296_geometry_frontend_calibration::validator::validate_manifest_with_contract;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo 根必须存在")
}

fn main() {
    validate_manifest_with_contract(&repo_root());
}
