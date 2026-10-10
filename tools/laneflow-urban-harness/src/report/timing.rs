use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter},
    path::Path,
};

use serde::{Deserialize, Serialize};

use crate::{Result, invalid};

pub(super) const FILE_NAME: &str = "timings.jsonl";
pub(super) const VERSION: &str = "urban-tick-timings-v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
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

#[derive(Deserialize)]
struct Declaration {
    version: String,
    file: String,
}

#[derive(Deserialize)]
struct Envelope {
    tick_timings: Option<Declaration>,
    window_step_samples_ns: Vec<u64>,
    window_command_samples_ns: Vec<u64>,
    window_observation_samples_ns: Vec<u64>,
}

/// 比较入口逐行核对逐拍计时：声明版本、每个完成拍恰好一行且 tick 连续，
/// 观察窗三个分量与 diagnostics 保存的原始样本逐项相等。
pub(super) fn validate(directory: &Path, completed_ticks: u64, warm_up_ticks: u64) -> Result<()> {
    let envelope: Envelope = serde_json::from_reader(BufReader::new(File::open(
        directory.join("diagnostics.json"),
    )?))?;
    if !envelope
        .tick_timings
        .is_some_and(|declared| declared.version == VERSION && declared.file == FILE_NAME)
    {
        return Err(invalid("逐拍计时声明缺失或版本不符"));
    }
    let mut tick = 0;
    let (mut step, mut command, mut observation) = (Vec::new(), Vec::new(), Vec::new());
    for line in BufReader::new(File::open(directory.join(FILE_NAME))?).lines() {
        let row: TickTiming = serde_json::from_str(&line?)?;
        tick += 1;
        if row.tick != tick {
            return Err(invalid("逐拍计时 tick 不连续"));
        }
        if tick > warm_up_ticks {
            step.push(row.step_ns);
            command.push(row.command_ns);
            observation.push(row.observation_ns);
        }
    }
    if tick != completed_ticks {
        return Err(invalid("逐拍计时行数与完成拍数不符"));
    }
    if step != envelope.window_step_samples_ns
        || command != envelope.window_command_samples_ns
        || observation != envelope.window_observation_samples_ns
    {
        return Err(invalid("逐拍计时与观察窗计时副本不符"));
    }
    Ok(())
}

/// 合成夹具：按给定观察窗分量写出逐拍计时，并把声明与副本写回 diagnostics。
#[cfg(test)]
pub(super) fn write_fixture(
    directory: &Path,
    completed_ticks: u64,
    warm_up_ticks: u64,
    window: [&[u64]; 3],
    diagnostics: &mut serde_json::Value,
) {
    let mut file = BufWriter::new(File::create(directory.join(FILE_NAME)).unwrap());
    let mut copies = [Vec::new(), Vec::new(), Vec::new()];
    for tick in 1..=completed_ticks {
        let value = |component: usize| {
            usize::try_from(tick.saturating_sub(warm_up_ticks + 1))
                .ok()
                .filter(|_| tick > warm_up_ticks)
                .map_or(1, |index| {
                    window[component].get(index).copied().unwrap_or(0)
                })
        };
        let row = TickTiming {
            tick,
            step_ns: value(0),
            command_ns: value(1),
            observation_ns: value(2),
            tick_elapsed_ns: value(0) + value(1) + value(2),
        };
        if tick > warm_up_ticks {
            copies[0].push(row.step_ns);
            copies[1].push(row.command_ns);
            copies[2].push(row.observation_ns);
        }
        super::line(&mut file, &row).unwrap();
    }
    std::io::Write::flush(&mut file).unwrap();
    let [step, command, observation] = copies;
    diagnostics["tick_timings"] = serde_json::json!({"version":VERSION,"file":FILE_NAME});
    diagnostics["window_step_samples_ns"] = serde_json::json!(step);
    diagnostics["window_command_samples_ns"] = serde_json::json!(command);
    diagnostics["window_observation_samples_ns"] = serde_json::json!(observation);
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
