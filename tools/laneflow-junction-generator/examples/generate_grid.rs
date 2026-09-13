//! 冻结 #285 规模制品；每个输出目录只写入一次。
use std::{error::Error, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: generate_grid <config.toml> <cells> <new-output-directory>".into());
    }
    let config = laneflow_junction_generator::load_config(std::path::Path::new(&args[0]))?;
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
