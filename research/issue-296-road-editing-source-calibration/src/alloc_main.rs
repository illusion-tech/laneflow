use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use issue_296_road_editing_source_calibration::{
    AllocatorProbeRequest, AllocatorProbeRole, run_allocator_probe,
};

#[global_allocator]
static ALLOCATOR: dhat::Alloc = dhat::Alloc;

fn main() {
    if let Err(error) = run() {
        eprintln!("road-editing allocator probe failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let repository_root = repository_root();
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let [role, output] = arguments.as_slice() else {
        return Err(
            "calibrate-alloc requires an allocator probe role and repository-relative JSON output"
                .into(),
        );
    };
    let role = AllocatorProbeRole::parse(role)
        .ok_or_else(|| format!("unknown allocator probe role {role:?}"))?;
    let output = checked_repository_json_path(&repository_root, output)?;
    let probe = run_allocator_probe(
        &repository_root,
        &AllocatorProbeRequest {
            role,
            argv: std::env::args().collect(),
        },
    )?;
    write_new_json(&output, &probe)?;
    println!("wrote allocator probe {}", output.display());
    Ok(())
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("research crate is two levels below repository root")
        .to_path_buf()
}

fn checked_repository_json_path(
    repository_root: &Path,
    relative: &str,
) -> Result<PathBuf, Box<dyn Error>> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path.extension().and_then(|extension| extension.to_str()) != Some("json")
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err(
            "allocator probe output must be a repository-relative .json path without traversal"
                .into(),
        );
    }
    Ok(repository_root.join(path))
}

fn write_new_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}
