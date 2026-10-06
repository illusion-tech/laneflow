//! 构建期捕获实际 rustc 版本：build provenance 的 `rust_version` 必须记录
//! 产出该二进制的工具链。`rust-version = "1.98"` 只是 MSRV，且仓库无
//! pinned rust-toolchain——硬编码会让任何兼容新编译器的构建都谎报同一
//! 版本，不同工具链的产物共享同一份 provenance。

use std::{env, process::Command};

fn main() {
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let output = Command::new(&rustc)
        .arg("--version")
        .output()
        .unwrap_or_else(|error| panic!("invoke {rustc} --version for build provenance: {error}"));
    assert!(
        output.status.success(),
        "{rustc} --version failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("rustc --version is utf-8");
    let version = stdout.trim();
    assert!(!version.is_empty(), "rustc --version produced no output");
    println!("cargo:rustc-env=LANEFLOW_BUILD_RUSTC_VERSION={version}");
}
