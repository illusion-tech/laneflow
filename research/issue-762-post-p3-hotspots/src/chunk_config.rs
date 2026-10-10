//! #801 受控构建拒绝 Cargo 搜索路径中的配置，并封存各位置的缺失状态。
use crate::{Result, io, need};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

const POLICY: &str = "p5-cargo-no-configuration-v1";

fn locations(source: &Path, environment: &Value) -> Result<BTreeSet<String>> {
    need(source.is_absolute(), "Cargo configuration source path")?;
    let mut directories: Vec<PathBuf> = source.ancestors().map(|p| p.join(".cargo")).collect();
    let home = if let Some(home) = environment["CARGO_HOME"].as_str() {
        PathBuf::from(home)
    } else {
        let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        PathBuf::from(environment[key].as_str().ok_or("Cargo home unavailable")?).join(".cargo")
    };
    directories.push(if home.is_absolute() {
        home
    } else {
        source.join(home)
    });
    Ok(directories
        .into_iter()
        .flat_map(|dir| [dir.join("config"), dir.join("config.toml")])
        .map(|p| io::slash(&p))
        .collect())
}

pub(crate) fn snapshot(source: &Path, environment: &Value) -> Result<Value> {
    let mut files = serde_json::Map::new();
    for location in locations(source, environment)? {
        let path = Path::new(&location);
        let value = if path.try_exists()? {
            json!({"sha256":io::sha(path)?})
        } else {
            Value::Null
        };
        files.insert(location, value);
    }
    let value = json!({"policy":POLICY,"files":files});
    validate(&value, source, environment)?;
    Ok(value)
}

pub(crate) fn validate(value: &Value, source: &Path, environment: &Value) -> Result<()> {
    let files = value["files"]
        .as_object()
        .ok_or("Cargo configuration records")?;
    need(
        value["policy"] == POLICY
            && files.keys().cloned().collect::<BTreeSet<_>>() == locations(source, environment)?
            && files.values().all(Value::is_null),
        "ambient Cargo configuration present or unattested",
    )
}

#[cfg(test)]
pub(crate) fn fixture(source: &Path, environment: &Value) -> Value {
    let files: serde_json::Map<_, _> = locations(source, environment)
        .unwrap()
        .into_iter()
        .map(|key| (key, Value::Null))
        .collect();
    json!({"policy":POLICY,"files":files})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn global_ancestor_and_missing_records_fail_closed() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let source = root.join("source");
        fs::create_dir_all(&source).unwrap();
        let environment = json!({"CARGO_HOME":root.join("home")});
        let good = fixture(&source, &environment);
        validate(&good, &source, &environment).unwrap();
        let home = io::slash(&root.join("home/config.toml"));
        let ancestor = io::slash(&root.join(".cargo/config"));
        for location in [home, ancestor] {
            let mut bad = good.clone();
            bad["files"][&location] = json!({"sha256":"changed LTO"});
            assert!(validate(&bad, &source, &environment).is_err());
            bad["files"].as_object_mut().unwrap().remove(&location);
            assert!(validate(&bad, &source, &environment).is_err());
        }
        fs::create_dir(root.join(".cargo")).unwrap();
        fs::write(
            root.join(".cargo/config.toml"),
            "[profile.release]\nlto = false\n",
        )
        .unwrap();
        assert!(snapshot(&source, &environment).is_err());
        fs::remove_file(root.join(".cargo/config.toml")).unwrap();
        fs::remove_dir(root.join(".cargo")).unwrap();
        fs::remove_dir(source).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
