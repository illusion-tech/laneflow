//! Runtime 的显式模块、有限格式接口及 Cargo 生产依赖规则。
//! 完整合同见 docs/design/traffic-runtime-module-boundary.md；不提供调用图证明。

use std::collections::BTreeSet;
use std::path::PathBuf;

mod dependencies;
mod sources;
#[cfg(test)]
mod tests;

const RUNTIME: &str = "laneflow-runtime";
const SPATIAL: &str = "laneflow-spatial";
const ADAPTER: &str = "laneflow-bevy";
const FORMAT: &str = "laneflow-format";
const WIRE: &str = "laneflow-runtime-snapshot-wire";

struct SourceInputs {
    entry: PathBuf,
    package_root: PathBuf,
    externals: BTreeSet<String>,
    formats: BTreeSet<String>,
}

/// Runtime 架构检查入口：默认特性集与全特性集各加载一次 Cargo 依赖图校验生产依赖方向，默认特性集下另校验生产源码的显式模块与有限格式接口。
pub(crate) fn run() -> Result<(), String> {
    let manifest = std::env::current_dir()
        .map_err(|error| error.to_string())?
        .join("Cargo.toml");
    for all_features in [false, true] {
        let metadata = dependencies::load(&manifest, all_features)?;
        metadata.check()?;
        if !all_features {
            sources::check(&metadata.source_inputs()?)?;
        }
    }
    println!("Runtime 架构检查通过：显式模块、格式接口与默认/全特性生产依赖规则通过。");
    Ok(())
}
