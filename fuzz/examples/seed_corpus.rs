//! 将固定合法种子与控制程序写入独立 fuzz corpus，并输出字节库存供审计。
use std::{error::Error, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("fuzz/corpus"));
    let mut inventory = String::from("target\tseed\tbytes\tsha256\n");
    for (target, name, bytes) in laneflow_fuzz::corpus_seeds() {
        let directory = root.join(target);
        fs::create_dir_all(&directory)?;
        fs::write(directory.join(name), &bytes)?;
        inventory.push_str(&format!(
            "{target}\t{name}\t{}\t{}\n",
            bytes.len(),
            laneflow_fuzz::corpus_digest(&bytes)
        ));
    }
    fs::write(root.join("seed-inventory.tsv"), &inventory)?;
    print!("{inventory}");
    Ok(())
}
