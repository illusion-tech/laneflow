//! 冻结 #285 规模制品；每个输出目录只写入一次。
use std::{error::Error, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: generate_grid <config.toml> <cells> <new-output-directory>".into());
    }
    let config = laneflow_junction_generator::load_config(std::path::Path::new(&args[0]))?;
    // 此入口服务于固定的 #285 取证协议；普通 generate_grid API 仍按其配置工作。
    if config.fixed_delta_ms != 16 || config.signal_cycle_ms()? != 4_503 * 16 {
        return Err("#285 scale protocol requires fixed_delta_ms=16 and signal cycle=72048ms; update the protocol before changing cadence".into());
    }
    let output = PathBuf::from(&args[2]);
    if output.exists() {
        return Err("output directory already exists".into());
    }
    let cells = args[1].parse()?;
    let generated = laneflow_junction_generator::generate_grid(&config, cells)?;
    std::fs::create_dir_all(&output)?;
    std::fs::write(output.join("network.lfca"), generated.lfca)?;
    std::fs::write(
        output.join("grid.catalog.toml"),
        toml::to_string(&generated.catalog)?,
    )?;
    std::fs::copy(&args[0], output.join("source-config.toml"))?;
    eprintln!(
        "generated {cells} cells in one network: {}",
        output.display()
    );
    Ok(())
}
