use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Result, invalid};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct RequestId(u32);

/// 在途请求的状态；结束（成功或耗尽）的请求移出账本，只计入计数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum RequestState {
    Scheduled {
        due: u64,
    },
    Pending {
        attempt: u32,
        next_tick: Option<u64>,
    },
}

struct Entry {
    state: RequestState,
    counted: bool,
}

/// 只保留在途请求；结束的请求即刻移出，账本大小与运行时长无关。
#[derive(Default)]
pub(crate) struct RequestLedger {
    entries: BTreeMap<RequestId, Entry>,
    next_sequence: u32,
    /// 封口后可登记的最小编号；封口前（安装期间）为空，计划请求可按任意序登记。
    /// 封口后登记编号严格递增，结束移出的编号因此不能再次登记。
    registration_floor: Option<u32>,
    pending: usize,
    exhausted: usize,
}

impl RequestLedger {
    pub fn allocate(&mut self) -> Result<u32> {
        let sequence = self.next_sequence;
        self.next_sequence = sequence
            .checked_add(1)
            .ok_or_else(|| invalid("请求编号耗尽"))?;
        Ok(sequence)
    }

    pub fn register(&mut self, sequence: u32, due: u64, counted: bool) -> Result<()> {
        let id = RequestId(sequence);
        if self.entries.contains_key(&id)
            || self
                .registration_floor
                .is_some_and(|floor| sequence < floor)
        {
            return Err(invalid(format!("请求编号已登记：{sequence}")));
        }
        let next = sequence
            .checked_add(1)
            .ok_or_else(|| invalid("请求编号耗尽"))?;
        self.next_sequence = self.next_sequence.max(next);
        if self.registration_floor.is_some() {
            self.registration_floor = Some(next);
        }
        self.entries.insert(
            id,
            Entry {
                state: RequestState::Scheduled { due },
                counted,
            },
        );
        Ok(())
    }

    pub fn begin(&mut self, sequence: u32, tick: u64, attempt: u32) -> Result<()> {
        let entry = self
            .entries
            .get_mut(&RequestId(sequence))
            .ok_or_else(|| invalid("执行未登记的请求"))?;
        let first = match entry.state {
            RequestState::Scheduled { due } if due == tick && attempt == 1 => true,
            RequestState::Pending {
                attempt: expected,
                next_tick: Some(due),
            } if expected == attempt && due == tick => false,
            _ => return Err(invalid("请求尝试与登记状态不匹配")),
        };
        entry.state = RequestState::Pending {
            attempt,
            next_tick: None,
        };
        if first && entry.counted {
            self.pending += 1;
        }
        Ok(())
    }

    pub fn retry(&mut self, sequence: u32, due: u64, next_attempt: u32) -> Result<()> {
        let entry = self
            .entries
            .get_mut(&RequestId(sequence))
            .ok_or_else(|| invalid("重试未登记的请求"))?;
        match entry.state {
            RequestState::Pending {
                attempt,
                next_tick: None,
            } if attempt.checked_add(1) == Some(next_attempt) => {}
            _ => return Err(invalid("请求不能从当前状态重试")),
        }
        entry.state = RequestState::Pending {
            attempt: next_attempt,
            next_tick: Some(due),
        };
        Ok(())
    }

    pub fn finish(&mut self, sequence: u32, exhausted: bool) -> Result<()> {
        let id = RequestId(sequence);
        let entry = self
            .entries
            .get(&id)
            .ok_or_else(|| invalid("结束未登记的请求"))?;
        if !matches!(
            entry.state,
            RequestState::Pending {
                next_tick: None,
                ..
            }
        ) {
            return Err(invalid("请求不能从当前状态结束"));
        }
        let counted = entry.counted;
        self.entries.remove(&id);
        if counted {
            self.pending -= 1;
            if exhausted {
                self.exhausted += 1;
            }
        }
        Ok(())
    }

    pub fn counts(&self) -> (usize, usize) {
        (self.pending, self.exhausted)
    }

    /// 安装结束时封口：此后只接受新分配的编号。
    pub(crate) fn seal(&mut self) {
        self.registration_floor = Some(self.next_sequence);
    }

    /// 存档用的在途登记：按序号升序的每一项、两个编号水位与两项计数。
    pub(crate) fn save(&self) -> SavedLedger {
        SavedLedger {
            entries: self
                .entries
                .iter()
                .map(|(id, entry)| SavedLedgerEntry {
                    sequence: id.0,
                    state: entry.state,
                    counted: entry.counted,
                })
                .collect(),
            next_sequence: self.next_sequence,
            registration_floor: self.registration_floor.unwrap_or(self.next_sequence),
            pending: self.pending,
            exhausted: self.exhausted,
        }
    }

    /// 从存档重建（已封口）并核对自洽：序号严格递增且小于登记水位、登记水位
    /// 不大于下一编号、在途计数与按登记项重算的值相同、只有排队中的请求（执行中
    /// 只出现在单个请求执行过程中，结束的请求已移出）。
    pub(crate) fn restore(saved: &SavedLedger) -> Result<Self> {
        if saved.registration_floor > saved.next_sequence {
            return Err(invalid("存档登记水位大于下一编号"));
        }
        let mut ledger = Self {
            entries: BTreeMap::new(),
            next_sequence: saved.next_sequence,
            registration_floor: Some(saved.registration_floor),
            pending: 0,
            exhausted: saved.exhausted,
        };
        let mut previous = None;
        for entry in &saved.entries {
            if previous.is_some_and(|previous| previous >= entry.sequence)
                || entry.sequence >= saved.registration_floor
            {
                return Err(invalid("存档请求编号不递增或不小于登记水位"));
            }
            previous = Some(entry.sequence);
            match entry.state {
                RequestState::Pending {
                    next_tick: None, ..
                } => return Err(invalid("存档含执行中的请求")),
                RequestState::Pending { .. } if entry.counted => ledger.pending += 1,
                _ => {}
            }
            ledger.entries.insert(
                RequestId(entry.sequence),
                Entry {
                    state: entry.state,
                    counted: entry.counted,
                },
            );
        }
        if ledger.pending != saved.pending {
            return Err(invalid("存档请求计数与登记项不符"));
        }
        Ok(ledger)
    }

    /// 应当恰好对应一个排队请求的登记项：(序号, 到期拍, 尝试次数)。
    pub(crate) fn queued(&self) -> impl Iterator<Item = (u32, u64, u32)> + '_ {
        self.entries
            .iter()
            .filter_map(|(id, entry)| match entry.state {
                RequestState::Scheduled { due } => Some((id.0, due, 1)),
                RequestState::Pending {
                    attempt,
                    next_tick: Some(due),
                } => Some((id.0, due, attempt)),
                _ => None,
            })
    }

    /// 在途登记项数。
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SavedLedgerEntry {
    pub sequence: u32,
    pub state: RequestState,
    pub counted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SavedLedger {
    pub entries: Vec<SavedLedgerEntry>,
    pub next_sequence: u32,
    pub registration_floor: u32,
    pub pending: usize,
    pub exhausted: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_have_one_identity_and_terminal_requests_cannot_restart() {
        let mut ledger = RequestLedger::default();
        ledger.seal();
        let sequence = ledger.allocate().unwrap();
        ledger.register(sequence, 0, true).unwrap();
        assert!(ledger.register(sequence, 0, true).is_err());
        ledger.begin(sequence, 0, 1).unwrap();
        assert!(ledger.begin(sequence, 0, 1).is_err());
        assert_eq!(ledger.counts(), (1, 0));
        ledger.retry(sequence, 8, 2).unwrap();
        assert!(ledger.begin(sequence, 7, 2).is_err());
        ledger.begin(sequence, 8, 2).unwrap();
        ledger.finish(sequence, false).unwrap();
        assert_eq!(ledger.counts(), (0, 0));
        assert_eq!(ledger.len(), 0);
        assert!(ledger.begin(sequence, 16, 3).is_err());
        assert!(ledger.finish(sequence, true).is_err());
        assert!(ledger.register(sequence, 16, true).is_err());
        assert!(ledger.allocate().unwrap() > sequence);
    }

    #[test]
    fn exhaustion_requires_registration_and_does_not_subtract_unrelated_requests() {
        let mut ledger = RequestLedger::default();
        assert!(ledger.finish(99, true).is_err());
        for sequence in 0..3 {
            ledger.register(sequence, 0, true).unwrap();
            ledger.begin(sequence, 0, 1).unwrap();
            ledger.finish(sequence, true).unwrap();
        }
        assert_eq!(ledger.counts(), (0, 3));
        ledger.register(3, 0, false).unwrap();
        ledger.begin(3, 0, 1).unwrap();
        ledger.finish(3, false).unwrap();
        assert_eq!(ledger.counts(), (0, 3));
    }

    #[test]
    fn sealed_ledger_rejects_out_of_order_registration() {
        let mut ledger = RequestLedger::default();
        // 安装期间计划请求可按任意序登记。
        ledger.register(7, 0, true).unwrap();
        ledger.register(2, 0, true).unwrap();
        ledger.seal();
        assert!(ledger.register(5, 0, true).is_err());
        let first = ledger.allocate().unwrap();
        let second = ledger.allocate().unwrap();
        assert_eq!((first, second), (8, 9));
        ledger.register(first, 1, true).unwrap();
        ledger.register(second, 1, true).unwrap();
        assert!(ledger.register(first, 1, true).is_err());
    }

    #[test]
    fn saved_ledger_round_trips_and_rejects_inconsistent_saves() {
        let mut ledger = RequestLedger::default();
        for sequence in 0..4 {
            ledger.register(sequence, 5, sequence != 3).unwrap();
        }
        ledger.begin(0, 5, 1).unwrap();
        ledger.retry(0, 9, 2).unwrap();
        ledger.begin(1, 5, 1).unwrap();
        ledger.finish(1, true).unwrap();
        ledger.seal();
        let saved = ledger.save();
        assert_eq!(saved.entries.len(), 3);
        let restored = RequestLedger::restore(&saved).unwrap();
        assert_eq!(restored.save(), saved);
        assert_eq!(restored.counts(), ledger.counts());
        assert_eq!(
            restored.queued().collect::<Vec<_>>(),
            vec![(0, 9, 2), (2, 5, 1), (3, 5, 1)]
        );

        let mut wrong_count = saved.clone();
        wrong_count.pending += 1;
        assert!(RequestLedger::restore(&wrong_count).is_err());
        let mut running = saved.clone();
        running.entries[0].state = RequestState::Pending {
            attempt: 2,
            next_tick: None,
        };
        assert!(RequestLedger::restore(&running).is_err());
        // 结束的请求不进存档：终态不是可读的状态。
        assert!(serde_json::from_str::<RequestState>(r#"{"state":"succeeded"}"#).is_err());
        let mut unordered = saved.clone();
        unordered.entries.swap(0, 1);
        assert!(RequestLedger::restore(&unordered).is_err());
        let mut stale_floor = saved.clone();
        stale_floor.registration_floor = 3;
        assert!(RequestLedger::restore(&stale_floor).is_err());
        let mut stale_next = saved;
        stale_next.next_sequence = 3;
        assert!(RequestLedger::restore(&stale_next).is_err());
        // 恢复的账本已封口：结束移出的编号不能再登记。
        let mut restored = restored;
        assert!(restored.register(1, 9, true).is_err());
    }
}
