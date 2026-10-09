use std::{fs::File, io::BufWriter, path::Path};

use serde::{Deserialize, Serialize};

use crate::{Result, invalid};

pub(super) const FILE_NAME: &str = "timings.jsonl";
pub(super) const VERSION: &str = "urban-tick-timings-v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct TickTiming {
    pub tick: u64,
    pub step_ns: u64,
    pub command_ns: u64,
    pub observation_ns: u64,
    pub tick_elapsed_ns: u64,
}

pub(super) struct Timings {
    rows: Vec<TickTiming>,
}

impl Timings {
    pub(super) fn new(ticks: u64) -> Result<Self> {
        let capacity = usize::try_from(ticks).map_err(|_| invalid("逐拍计时容量超限"))?;
        let mut rows = Vec::new();
        rows.try_reserve_exact(capacity)
            .map_err(|error| invalid(format!("无法预分配逐拍计时记录：{error}")))?;
        Ok(Self { rows })
    }

    pub(super) fn push(&mut self, row: TickTiming) {
        debug_assert!(self.rows.len() < self.rows.capacity());
        self.rows.push(row);
    }

    pub(super) fn write(&self, output: &Path) -> Result<()> {
        let mut writer = BufWriter::with_capacity(
            super::log_io::BUFFER_BYTES_PER_LOG,
            File::create(output.join(FILE_NAME))?,
        );
        for row in &self.rows {
            super::line(&mut writer, row)?;
        }
        std::io::Write::flush(&mut writer)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_keep_tick_order_and_independent_elapsed_time() {
        let output = tempfile::tempdir().unwrap();
        let mut timings = Timings::new(3).unwrap();
        let capacity = timings.rows.capacity();
        for (index, step_ns) in [90, 10, 70].into_iter().enumerate() {
            timings.push(TickTiming {
                tick: index as u64 + 1,
                step_ns,
                command_ns: 2,
                observation_ns: 3,
                tick_elapsed_ns: 200 + index as u64,
            });
        }
        assert_eq!(timings.rows.capacity(), capacity);
        assert!(!output.path().join(FILE_NAME).exists());
        timings.write(output.path()).unwrap();
        let rows = std::fs::read_to_string(output.path().join(FILE_NAME))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<TickTiming>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(rows, timings.rows);
        assert_eq!(
            rows.iter().map(|row| row.step_ns).collect::<Vec<_>>(),
            [90, 10, 70]
        );
        assert_eq!(rows[0].tick_elapsed_ns, 200);
    }

    #[test]
    fn impossible_capacity_is_rejected_before_ticking() {
        assert!(Timings::new(u64::MAX).is_err());
    }
}
