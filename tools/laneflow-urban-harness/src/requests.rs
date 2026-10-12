use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Result, invalid};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct RequestId(u32);

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
    Succeeded,
    Exhausted,
}

struct Entry {
    state: RequestState,
    counted: bool,
}

#[derive(Default)]
pub(crate) struct RequestLedger {
    entries: BTreeMap<RequestId, Entry>,
    next_sequence: u32,
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
        if self.entries.contains_key(&id) {
            return Err(invalid(format!("请求编号已登记：{sequence}")));
        }
        let next = sequence
            .checked_add(1)
            .ok_or_else(|| invalid("请求编号耗尽"))?;
        self.next_sequence = self.next_sequence.max(next);
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
        let entry = self
            .entries
            .get_mut(&RequestId(sequence))
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
        entry.state = if exhausted {
            RequestState::Exhausted
        } else {
            RequestState::Succeeded
        };
        if entry.counted {
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

    /// 存档用的完整登记：按序号升序的每一项、下一编号与两项计数。
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
            pending: self.pending,
            exhausted: self.exhausted,
        }
    }

    /// 从存档重建并核对自洽：序号严格递增、下一编号大于全部序号、两项计数与
    /// 按登记项重算的值相同、没有执行中的请求（只出现在单个请求执行过程中）。
    pub(crate) fn restore(saved: &SavedLedger) -> Result<Self> {
        let mut ledger = Self {
            entries: BTreeMap::new(),
            next_sequence: saved.next_sequence,
            pending: 0,
            exhausted: 0,
        };
        let mut previous = None;
        for entry in &saved.entries {
            if previous.is_some_and(|previous| previous >= entry.sequence)
                || entry.sequence >= saved.next_sequence
            {
                return Err(invalid("存档请求编号不递增或不小于下一编号"));
            }
            previous = Some(entry.sequence);
            match entry.state {
                RequestState::Pending {
                    next_tick: None, ..
                } => return Err(invalid("存档含执行中的请求")),
                RequestState::Pending { .. } if entry.counted => ledger.pending += 1,
                RequestState::Exhausted if entry.counted => ledger.exhausted += 1,
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
        if (ledger.pending, ledger.exhausted) != (saved.pending, saved.exhausted) {
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
    pub pending: usize,
    pub exhausted: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_have_one_identity_and_terminal_requests_cannot_restart() {
        let mut ledger = RequestLedger::default();
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
    fn saved_ledger_round_trips_and_rejects_inconsistent_saves() {
        let mut ledger = RequestLedger::default();
        for sequence in 0..4 {
            ledger.register(sequence, 5, sequence != 3).unwrap();
        }
        ledger.begin(0, 5, 1).unwrap();
        ledger.retry(0, 9, 2).unwrap();
        ledger.begin(1, 5, 1).unwrap();
        ledger.finish(1, true).unwrap();
        let saved = ledger.save();
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
        let mut unordered = saved.clone();
        unordered.entries.swap(0, 1);
        assert!(RequestLedger::restore(&unordered).is_err());
        let mut stale_next = saved;
        stale_next.next_sequence = 3;
        assert!(RequestLedger::restore(&stale_next).is_err());
    }
}
