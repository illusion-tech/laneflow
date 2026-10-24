//! sustained 夹具的写时登记、受检复制与解析前核对（#1043），只在测试构建中使用。

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use laneflow_urban_generator::verified_write::{self, SourceRegistration, WritePolicy};
use serde::{Serialize, de::DeserializeOwned};

use crate::{Result, digest::FileDigest, invalid, written::DigestWriter};

const MAX_REGENERATIONS: u32 = 3;

#[derive(Debug, Default)]
pub(super) struct CopyStats {
    pub copies: u32,
    pub source_regenerations: u32,
    pub regeneration_rewrites: u32,
    pub target_rewrites: u32,
}

/// 期望值保存在生成时的内存登记中；包括 result.json，不能从待核对目录重新建立期望。
#[derive(Debug)]
pub(super) struct Fixture {
    directory: PathBuf,
    files: BTreeMap<String, FileDigest>,
    rewrites: u32,
}

impl Fixture {
    pub fn new(directory: &Path) -> Result<Self> {
        fs::create_dir_all(directory)?;
        Ok(Self {
            directory: directory.to_owned(),
            files: BTreeMap::new(),
            rewrites: 0,
        })
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn rewrites(&self) -> u32 {
        self.rewrites
    }

    pub fn record(&mut self, name: &str, digest: FileDigest) {
        self.files.insert(name.into(), digest);
    }

    pub fn result_files(&self) -> BTreeMap<String, FileDigest> {
        self.files
            .iter()
            .filter(|(name, _)| name.as_str() != "result.json")
            .map(|(name, digest)| (name.clone(), digest.clone()))
            .collect()
    }

    pub fn write(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        let expected = FileDigest::of(bytes);
        let written = verified_write::write_verified(
            &self.directory.join(name),
            bytes,
            &SourceRegistration::of(bytes),
            WritePolicy::DEFAULT,
        )
        .map_err(|error| {
            self.rewrites += match &error {
                verified_write::VerifiedWriteError::ReadbackMismatch { rewrites, .. } => *rewrites,
                verified_write::VerifiedWriteError::ReadbackDivergent { attempt, .. } => attempt - 1,
                _ => 0,
            };
            let diagnostic = format!(
                "[digest-probe] site=fixture-copy name={name} phase=target action=fail-closed error={error}"
            );
            eprintln!("{diagnostic}");
            invalid(diagnostic)
        })?;
        self.rewrites += written.rewrites;
        self.record(name, expected);
        Ok(())
    }

    pub fn write_json(&mut self, name: &str, value: &impl Serialize) -> Result<()> {
        let mut bytes = serde_json::to_vec_pretty(value)?;
        bytes.push(b'\n');
        self.write(name, &bytes)
    }

    pub fn stream(
        &mut self,
        name: &str,
        fill: impl FnOnce(&mut DigestWriter) -> Result<()>,
    ) -> Result<()> {
        let mut writer = DigestWriter::create(&self.directory.join(name))?;
        fill(&mut writer)?;
        self.record(name, writer.finish()?);
        Ok(())
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        self.read_checked(name, "before-edit", 1, "fail-closed")
    }

    pub fn read_text(&self, name: &str) -> Result<String> {
        String::from_utf8(self.read(name)?)
            .map_err(|error| invalid(format!("受检夹具 {name} 的编码无效：{error}")))
    }

    pub fn read_json<T: DeserializeOwned>(&self, name: &str) -> Result<T> {
        Ok(serde_json::from_slice(&self.read(name)?)?)
    }

    pub fn verify_all(&self) -> Result<()> {
        for name in self.files.keys() {
            self.read_checked(name, "before-compare", 1, "fail-closed")?;
        }
        Ok(())
    }

    fn read_checked(&self, name: &str, phase: &str, attempt: u32, action: &str) -> Result<Vec<u8>> {
        let expected = self
            .files
            .get(name)
            .ok_or_else(|| invalid(format!("夹具文件未登记：{name}")))?;
        let bytes = fs::read(self.directory.join(name))?;
        let actual = FileDigest::of(&bytes);
        if actual != *expected {
            let diagnostic = format!(
                "[digest-probe] site=fixture-copy name={name} phase={phase} attempt={attempt} expected_digest={} actual_digest={} expected_bytes={} actual_bytes={} actual_head32={:02x?} action={action} path={}",
                expected.digest,
                actual.digest,
                expected.bytes,
                actual.bytes,
                &bytes[..bytes.len().min(32)],
                self.directory.join(name).display(),
            );
            eprintln!("{diagnostic}");
            return Err(invalid(diagnostic));
        }
        Ok(bytes)
    }

    /// 每份源夹具每次复制最多重建三次；重建必须仍匹配首次写出登记，不能采纳新期望。
    /// 目标重写由 verified_write 自身限界；目标失败不触发源重建。
    pub fn copy_to(
        &self,
        destination: &Path,
        mut regenerate: impl FnMut() -> Result<Self>,
        stats: &mut CopyStats,
    ) -> Result<Self> {
        assert_ne!(self.directory, destination, "distinct fixture directories");
        if destination.exists() {
            fs::remove_dir_all(destination)?;
        }
        let mut copied = Self::new(destination)?;
        'attempt: for regeneration in 0..=MAX_REGENERATIONS {
            for name in self.files.keys() {
                let action = if regeneration < MAX_REGENERATIONS {
                    "regenerate"
                } else {
                    "fail-closed"
                };
                let bytes = match self.read_checked(name, "source", regeneration + 1, action) {
                    Ok(bytes) => bytes,
                    // 仅摘要/长度失配触发重建；I/O 错误不靠重试隐藏。
                    Err(crate::Error::Validation(error)) if regeneration < MAX_REGENERATIONS => {
                        stats.source_regenerations += 1;
                        let regenerated = regenerate()?;
                        stats.regeneration_rewrites += regenerated.rewrites;
                        if regenerated.files != self.files {
                            let diagnostic = format!(
                                "[digest-probe] site=fixture-copy name={name} phase=regenerate attempt={} reason=changed-registration action=fail-closed prior={error}",
                                regeneration + 1,
                            );
                            eprintln!("{diagnostic}");
                            return Err(invalid(diagnostic));
                        }
                        continue 'attempt;
                    }
                    Err(error) => return Err(error),
                };
                let before = copied.rewrites;
                let outcome = copied.write(name, &bytes);
                stats.target_rewrites += copied.rewrites - before;
                outcome?;
            }
            stats.copies += 1;
            return Ok(copied);
        }
        unreachable!("last mismatch returns an error")
    }
}

/// 非预期 panic 时保留实际夹具目录，避免 TempDir 清理唯一的失配现场。
pub(super) struct FixtureDirectory(Option<tempfile::TempDir>);

impl FixtureDirectory {
    pub fn new() -> Self {
        Self(Some(tempfile::tempdir().expect("fixture directory")))
    }

    pub fn path(&self) -> &Path {
        self.0.as_ref().expect("live fixture directory").path()
    }
}

impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        if std::thread::panicking()
            && let Some(directory) = self.0.take()
        {
            eprintln!(
                "[digest-probe] site=fixture-copy action=preserve path={}",
                directory.keep().display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use laneflow_urban_generator::verified_write::test_support::with_after_write;

    use super::*;

    fn small_fixture(directory: &Path) -> Result<Fixture> {
        let mut fixture = Fixture::new(directory)?;
        fixture.write("result.json", b"{\"status\":\"fixture\"}\n")?;
        fixture.write("ticks.jsonl", b"{\"tick\":1}\n{\"tick\":2}\n")?;
        Ok(fixture)
    }

    #[test]
    fn source_corruption_regenerates_with_original_registration_including_receipt() {
        for name in ["result.json", "ticks.jsonl"] {
            let temp = tempfile::tempdir().unwrap();
            let source = small_fixture(&temp.path().join("source")).unwrap();
            fs::write(source.directory.join(name), [0xFF]).unwrap();
            let mut stats = CopyStats::default();
            let copied = source
                .copy_to(
                    &temp.path().join("target"),
                    || small_fixture(source.directory()),
                    &mut stats,
                )
                .unwrap();
            assert_eq!(stats.source_regenerations, 1);
            assert_eq!(stats.target_rewrites, 0);
            assert_eq!(stats.copies, 1);
            for name in source.files.keys() {
                assert_eq!(copied.read(name).unwrap(), source.read(name).unwrap());
            }
        }
    }

    #[test]
    fn persistent_source_corruption_exhausts_without_adopting_readback_digest() {
        let temp = tempfile::tempdir().unwrap();
        let source = small_fixture(&temp.path().join("source")).unwrap();
        fs::write(source.directory.join("ticks.jsonl"), [0xFF]).unwrap();
        let mut stats = CopyStats::default();
        let error = source
            .copy_to(
                &temp.path().join("target"),
                || {
                    let regenerated = small_fixture(source.directory())?;
                    fs::write(source.directory.join("ticks.jsonl"), [0xFF])?;
                    Ok(regenerated)
                },
                &mut stats,
            )
            .unwrap_err();
        assert_eq!(stats.source_regenerations, 3);
        assert_eq!(stats.copies, 0);
        let text = error.to_string();
        for expected in [
            "site=fixture-copy",
            "name=ticks.jsonl",
            "phase=source",
            "attempt=4",
            "action=fail-closed",
            "actual_head32=[ff]",
        ] {
            assert!(text.contains(expected), "{text}");
        }
    }

    #[test]
    fn regeneration_cannot_replace_the_expected_content() {
        let temp = tempfile::tempdir().unwrap();
        let source = small_fixture(&temp.path().join("source")).unwrap();
        fs::write(source.directory.join("ticks.jsonl"), [0xFF]).unwrap();
        let mut stats = CopyStats::default();
        let error = source
            .copy_to(
                &temp.path().join("target"),
                || {
                    let mut regenerated = small_fixture(source.directory())?;
                    regenerated.write("ticks.jsonl", b"{\"tick\":3}\n")?;
                    Ok(regenerated)
                },
                &mut stats,
            )
            .unwrap_err();
        assert_eq!(stats.source_regenerations, 1);
        assert!(error.to_string().contains("reason=changed-registration"));
    }

    #[test]
    fn target_corruption_uses_verified_write_retry_and_exhaustion() {
        for persistent in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let source = small_fixture(&temp.path().join("source")).unwrap();
            let destination = temp.path().join("target");
            let target_file = destination.join("ticks.jsonl");
            let hook_target = target_file.clone();
            let writes = Rc::new(Cell::new(0));
            let hook_writes = Rc::clone(&writes);
            let mut stats = CopyStats::default();
            let outcome = with_after_write(
                move |path| {
                    if path == hook_target {
                        let count = hook_writes.get();
                        hook_writes.set(count + 1);
                        if persistent || count == 0 {
                            fs::write(path, [0xFF])?;
                        }
                    }
                    Ok(())
                },
                || {
                    source.copy_to(
                        &destination,
                        || panic!("target failure must not regenerate source"),
                        &mut stats,
                    )
                },
            );
            assert_eq!(stats.source_regenerations, 0);
            if persistent {
                let error = outcome.unwrap_err().to_string();
                assert_eq!(writes.get(), 4);
                assert_eq!(stats.target_rewrites, 3);
                assert_eq!(stats.copies, 0);
                assert!(!target_file.exists());
                assert!(error.contains("site=fixture-copy"), "{error}");
                assert!(error.contains("phase=target"), "{error}");
                assert!(error.contains("重写 3 次耗尽"), "{error}");
            } else {
                let copied = outcome.unwrap();
                assert_eq!(writes.get(), 2);
                assert_eq!(stats.target_rewrites, 1);
                assert_eq!(stats.copies, 1);
                assert_eq!(
                    copied.read("ticks.jsonl").unwrap(),
                    source.read("ticks.jsonl").unwrap(),
                );
            }
        }
    }

    #[test]
    fn before_edit_checks_digest_before_utf8_or_json_parsing() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = small_fixture(temp.path()).unwrap();
        for name in ["result.json", "ticks.jsonl"] {
            fs::write(fixture.directory.join(name), [0xFF]).unwrap();
            let error = fixture.read_text(name).unwrap_err();
            assert!(matches!(error, crate::Error::Validation(_)));
            assert!(error.to_string().contains("phase=before-edit"));
        }
        let error = fixture
            .read_json::<serde_json::Value>("result.json")
            .unwrap_err();
        assert!(matches!(error, crate::Error::Validation(_)));
        assert!(error.to_string().contains("site=fixture-copy"));
    }

    #[test]
    fn unexpected_failure_keeps_the_fixture_bytes() {
        let directory = FixtureDirectory::new();
        let path = directory.path().to_owned();
        fs::write(path.join("ticks.jsonl"), [0xFF]).unwrap();
        let outcome = std::panic::catch_unwind(move || {
            let _directory = directory;
            panic!("injected fixture failure");
        });
        assert!(outcome.is_err());
        assert_eq!(fs::read(path.join("ticks.jsonl")).unwrap(), [0xFF]);
        fs::remove_dir_all(path).unwrap();
    }
}
