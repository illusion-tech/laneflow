use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::encoding::{canonical_json, config_bytes, digest, hex};
use crate::state::initial_state;
use crate::{Case, Error, Identity, network, plan};

/// 调用者声明的设计来源与生成实现提交，不参与交通输入摘要。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub source_commit: String,
    pub generator_commit: String,
}

impl Provenance {
    fn validate(&self) -> Result<(), Error> {
        for commit in [&self.source_commit, &self.generator_commit] {
            if commit.len() != 40
                || !commit
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            {
                return Err(Error::Invalid("提交标识必须为 40 位小写十六进制".into()));
            }
        }
        Ok(())
    }
}

/// 文件完整性记录；摘要覆盖精确文件字节。
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileRecord {
    pub name: String,
    pub byte_length: u64,
    pub digest: String,
}

/// 规范输入包的身份、计数与三个文件记录。
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: String,
    pub identity: Identity,
    pub source_commit: String,
    pub generator_commit: String,
    pub config_digest: String,
    pub counts: BTreeMap<String, u32>,
    pub files: Vec<FileRecord>,
}

/// 已从冻结合同构造的四文件输入包。
#[derive(Debug)]
pub struct Bundle {
    manifest: Manifest,
    files: BTreeMap<String, Vec<u8>>,
}

impl Bundle {
    #[must_use]
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// 返回精确文件名与精确字节，不包含本机路径。
    #[must_use]
    pub fn files(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.files
    }

    /// 写入一个全新目录，逐文件回读比较，manifest 最后写入。
    ///
    /// # Errors
    /// 目录已存在、写入失败或回读不符时返回错误；保留不完整目录供诊断，不覆盖旧文件。
    pub fn write_new(&self, directory: &Path) -> Result<(), Error> {
        fs::create_dir(directory)?;
        for name in [
            "network.lfca",
            "state.json",
            "input-plan.json",
            "manifest.json",
        ] {
            let bytes = &self.files[name];
            let path = directory.join(name);
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            if read_bounded(&path, limit(name))? != *bytes {
                return Err(Error::Invalid(format!("写后回读不符：{name}")));
            }
        }
        Ok(())
    }
}

/// 从正式身份生成已通过编译器后发射检查的规范输入包。
///
/// # Errors
/// 非法身份、来源格式、编译/发射/受检验证或编码失败时返回错误。
pub fn generate(identity: Identity, provenance: Provenance) -> Result<Bundle, Error> {
    identity.validate()?;
    provenance.validate()?;
    generate_inputs(identity, provenance)
}

pub(crate) fn generate_inputs(identity: Identity, provenance: Provenance) -> Result<Bundle, Error> {
    let mut files = BTreeMap::new();
    files.insert("network.lfca".into(), network::compile(&identity)?);
    files.insert(
        "state.json".into(),
        canonical_json(&initial_state(&identity)?)?,
    );
    files.insert(
        "input-plan.json".into(),
        canonical_json(&plan::input_plan(&identity))?,
    );
    let cells = identity.cells();
    let active = match identity.case {
        Case::W1 | Case::W4 => identity.scale * 3 / 4,
        Case::W2 => identity.scale,
        Case::W3 => identity.scale / 4,
    };
    let counts = [
        ("cells", cells),
        ("individuals", identity.scale),
        ("routes", cells * 4),
        ("edges", cells * 8),
        ("controllers", cells),
        ("groups", cells * 4),
        ("gates", cells * 4),
        ("stop_lines", cells * 4),
        ("junctions", cells * 4),
        ("movements", cells * 4),
        ("maneuver_paths", cells * 4),
        ("facilities", cells),
        ("spaces", identity.scale),
        ("geometry_points", cells * 416),
        ("active", active),
        ("parked", identity.scale - active),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value))
    .collect();
    let manifest = Manifest {
        format: "lf-synth-manifest-1".into(),
        config_digest: hex(&digest(&config_bytes(&identity)?)),
        identity,
        source_commit: provenance.source_commit,
        generator_commit: provenance.generator_commit,
        counts,
        files: files
            .iter()
            .map(|(name, bytes): (&String, &Vec<u8>)| FileRecord {
                name: name.clone(),
                byte_length: bytes.len() as u64,
                digest: hex(&digest(bytes)),
            })
            .collect(),
    };
    files.insert("manifest.json".into(), canonical_json(&manifest)?);
    Ok(Bundle { manifest, files })
}

fn limit(name: &str) -> u64 {
    match name {
        "manifest.json" => 65_536,
        "input-plan.json" => 4 * 1_024 * 1_024,
        "state.json" => 64 * 1_024 * 1_024,
        "network.lfca" => 256 * 1_024 * 1_024,
        _ => 0,
    }
}

fn read_bounded(path: &Path, max_bytes: u64) -> Result<Vec<u8>, Error> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.len() > max_bytes {
        return Err(Error::Invalid(format!(
            "非普通文件或超限：{}",
            path.display()
        )));
    }
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(max_bytes + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(Error::Invalid(format!("读取期间超限：{}", path.display())));
    }
    Ok(bytes)
}

/// 检查目录、精确文件摘要和规范编码，再按同身份重新编译比较全部字节。
///
/// # Errors
/// 目录非法、文件篡改/缺失/超限、编码不规范或不符合冻结生成合同都会被拒绝。
pub fn verify_directory(directory: &Path) -> Result<Bundle, Error> {
    let mut names = fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    if names
        != [
            "input-plan.json",
            "manifest.json",
            "network.lfca",
            "state.json",
        ]
    {
        return Err(Error::Invalid("目录必须恰有四个规范文件".into()));
    }
    let manifest_bytes = read_bounded(&directory.join("manifest.json"), limit("manifest.json"))?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)?;
    manifest.identity.validate()?;
    let provenance = Provenance {
        source_commit: manifest.source_commit.clone(),
        generator_commit: manifest.generator_commit.clone(),
    };
    provenance.validate()?;
    if manifest.format != "lf-synth-manifest-1"
        || canonical_json(&manifest)? != manifest_bytes
        || manifest
            .files
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>()
            != ["input-plan.json", "network.lfca", "state.json"]
    {
        return Err(Error::Invalid("manifest 格式、编码或文件清单不符".into()));
    }
    let mut actual = BTreeMap::new();
    for record in &manifest.files {
        let bytes = read_bounded(&directory.join(&record.name), limit(&record.name))?;
        if bytes.len() as u64 != record.byte_length || hex(&digest(&bytes)) != record.digest {
            return Err(Error::Invalid(format!(
                "文件长度或摘要不符：{}",
                record.name
            )));
        }
        actual.insert(record.name.clone(), bytes);
    }
    actual.insert("manifest.json".into(), manifest_bytes);
    let expected = generate(manifest.identity, provenance)?;
    for (name, bytes) in expected.files() {
        if actual.get(name) != Some(bytes) {
            return Err(Error::Invalid(format!("与规范重建不符：{name}")));
        }
    }
    Ok(expected)
}
