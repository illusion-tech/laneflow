use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

use serde::Serialize;

use crate::Result;

pub(super) const BUFFER_BYTES_PER_LOG: usize = 10 * 1_024 * 1_024;

pub(super) struct Logs {
    writers: [BufWriter<File>; 3],
}

impl Logs {
    pub(super) fn new(output: &Path) -> Result<Self> {
        let make = |name: &str| -> Result<_> {
            Ok(BufWriter::with_capacity(
                BUFFER_BYTES_PER_LOG,
                File::create(output.join(name))?,
            ))
        };
        Ok(Self {
            writers: [
                make("ticks.jsonl")?,
                make("commands.jsonl")?,
                make("events.jsonl")?,
            ],
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

    /// 刷盘后随所有权释放缓冲，后续输出不与日志缓冲同时驻留。
    pub(super) fn finish(self) -> Result<()> {
        for mut writer in self.writers {
            writer.flush()?;
        }
        Ok(())
    }
}
