use std::path::Path;

use serde::Serialize;

use crate::{Result, digest::FileDigest, written::DigestWriter};

pub(super) const BUFFER_BYTES_PER_LOG: usize = 10 * 1_024 * 1_024;
pub(super) const FILE_NAMES: [&str; 3] = ["ticks.jsonl", "commands.jsonl", "events.jsonl"];

pub(super) struct Logs {
    writers: [DigestWriter; 3],
}

impl Logs {
    pub(super) fn new(output: &Path) -> Result<Self> {
        let make =
            |name: &str| DigestWriter::with_capacity(&output.join(name), BUFFER_BYTES_PER_LOG);
        let [ticks, commands, events] = FILE_NAMES;
        Ok(Self {
            writers: [make(ticks)?, make(commands)?, make(events)?],
        })
    }

    pub(super) fn write_tick(
        &mut self,
        record: &impl Serialize,
        commands: &[impl Serialize],
        events: &[impl Serialize],
    ) -> Result<()> {
        super::line(&mut self.writers[0], record)?;
        for command in commands {
            super::line(&mut self.writers[1], command)?;
        }
        for event in events {
            super::line(&mut self.writers[2], event)?;
        }
        Ok(())
    }

    /// 刷盘、读回核对后随所有权释放缓冲，后续输出不与日志缓冲同时驻留；
    /// 返回按 [`FILE_NAMES`] 顺序的写出摘要（#1042）。
    pub(super) fn finish(self) -> Result<[FileDigest; 3]> {
        let [ticks, commands, events] = self.writers;
        Ok([ticks.finish()?, commands.finish()?, events.finish()?])
    }
}
