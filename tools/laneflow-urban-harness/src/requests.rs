use std::collections::BTreeMap;

use crate::{Result, invalid};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct RequestId(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestState {
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
}
