use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::{Result, validation};

/// Compare the complete v1 canonical delivery, excluding nondeterministic measurements.
pub fn compare_artifacts(a: &Path, b: &Path) -> Result<()> {
    for name in [
        "config.toml",
        "common.lfre",
        "topology.lfre",
        "network.lfca",
        "source-map.lfsm",
        "genesis.lfsd",
        "routes.toml",
        "manifest.toml",
    ] {
        let mut left = File::open(a.join(name))?;
        let mut right = File::open(b.join(name))?;
        let mut remaining = left.metadata()?.len();
        if remaining != right.metadata()?.len() {
            return Err(validation("artifact comparison", name));
        }
        let mut x = [0; 65_536];
        let mut y = [0; 65_536];
        while remaining != 0 {
            let length = remaining.min(x.len() as u64) as usize;
            left.read_exact(&mut x[..length])?;
            right.read_exact(&mut y[..length])?;
            if x[..length] != y[..length] {
                return Err(validation("artifact comparison", name));
            }
            remaining -= length as u64;
        }
    }
    Ok(())
}
