//! #814：公共 step 外侧的 Windows 时间锚点；不修改交通计算或按时间比例分配 PMC。
use std::{
    io,
    sync::OnceLock,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const FILETIME_UNIX_EPOCH: u64 = 116_444_736_000_000_000;
const MAX_CLOCK_ERROR_NS: u128 = 100_000;
static ENABLED: OnceLock<Option<bool>> = OnceLock::new();

fn invalid(message: impl std::fmt::Display) -> io::Error {
    io::Error::other(message.to_string())
}

fn filetime(time: SystemTime) -> io::Result<u64> {
    let ticks = time.duration_since(UNIX_EPOCH).map_err(invalid)?.as_nanos() / 100;
    u64::try_from(ticks)
        .ok()
        .and_then(|value| value.checked_add(FILETIME_UNIX_EPOCH))
        .ok_or_else(|| invalid("FILETIME overflow"))
}

pub(crate) struct Anchor {
    before: Instant,
    wall: u64,
    after: Instant,
}

impl Anchor {
    pub(crate) fn begin() -> io::Result<Option<Self>> {
        match ENABLED.get_or_init(|| match std::env::var("LF814_WPR_WINDOWS").as_deref() {
            Ok("1") => Some(true),
            Ok("0") | Err(std::env::VarError::NotPresent) => Some(false),
            _ => None,
        }) {
            Some(true) => Self::now().map(Some),
            Some(false) => Ok(None),
            None => Err(invalid("LF814_WPR_WINDOWS must be 0 or 1")),
        }
    }

    pub(crate) fn now() -> io::Result<Self> {
        let before = Instant::now();
        let wall = SystemTime::now();
        let after = Instant::now();
        Ok(Self {
            before,
            wall: filetime(wall)?,
            after,
        })
    }
}

pub(crate) struct Window {
    pub(crate) start_lower: u64,
    pub(crate) start_upper: u64,
    pub(crate) end_lower: u64,
    pub(crate) end_upper: u64,
    pub(crate) step_ns: u64,
    pub(crate) clock_error_ns: u64,
}

fn floor_ticks(n: u128) -> io::Result<u64> {
    u64::try_from(n / 100).map_err(invalid)
}
fn ceil_ticks(n: u128) -> io::Result<u64> {
    u64::try_from(n.div_ceil(100)).map_err(invalid)
}

impl Window {
    pub(crate) fn new(
        begin: Anchor,
        end: Anchor,
        started: Instant,
        finished: Instant,
    ) -> io::Result<Self> {
        if begin.before > begin.after
            || begin.after > started
            || started > finished
            || finished > end.before
            || end.before > end.after
            || begin.wall > end.wall
        {
            return Err(invalid("clock order"));
        }
        let utc_ns = u128::from(end.wall - begin.wall) * 100;
        let lo = end.before.duration_since(begin.after).as_nanos();
        let hi = end.after.duration_since(begin.before).as_nanos();
        let error = lo.saturating_sub(utc_ns).max(utc_ns.saturating_sub(hi));
        if error > MAX_CLOCK_ERROR_NS {
            return Err(invalid("system clock diverged from monotonic clock"));
        }
        let start_lower = begin
            .wall
            .checked_add(floor_ticks(started.duration_since(begin.after).as_nanos())?)
            .ok_or_else(|| invalid("start overflow"))?;
        let start_upper = begin
            .wall
            .checked_add(ceil_ticks(started.duration_since(begin.before).as_nanos())?)
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| invalid("start overflow"))?;
        let end_lower = end
            .wall
            .checked_sub(ceil_ticks(end.after.duration_since(finished).as_nanos())?)
            .ok_or_else(|| invalid("end underflow"))?;
        let end_upper = end
            .wall
            .checked_sub(floor_ticks(end.before.duration_since(finished).as_nanos())?)
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| invalid("end overflow"))?;
        Ok(Self {
            start_lower,
            start_upper,
            end_lower,
            end_upper,
            step_ns: u64::try_from(finished.duration_since(started).as_nanos()).map_err(invalid)?,
            clock_error_ns: u64::try_from(error).map_err(invalid)?,
        })
    }

    pub(crate) fn emit(&self, tick: u64) {
        eprintln!(
            "LF814_WPR {{\"pid\":{},\"tick\":{},\"step_ns\":{},\"start_lower_filetime\":{},\"start_upper_filetime\":{},\"end_lower_filetime\":{},\"end_upper_filetime\":{},\"clock_error_ns\":{}}}",
            std::process::id(),
            tick,
            self.step_ns,
            self.start_lower,
            self.start_upper,
            self.end_lower,
            self.end_upper,
            self.clock_error_ns
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn anchor_bounds_enclose_step_and_preserve_monotonic_latency() {
        let t = Instant::now();
        let at = |n| t + Duration::from_nanos(n);
        let begin = Anchor {
            before: at(0),
            wall: 10_000,
            after: at(200),
        };
        let end = Anchor {
            before: at(10_000),
            wall: 10_101,
            after: at(10_200),
        };
        let w = Window::new(begin, end, at(300), at(9_900)).unwrap();
        assert_eq!((w.start_lower, w.start_upper), (10_001, 10_004));
        assert_eq!(
            (w.end_lower, w.end_upper, w.step_ns),
            (10_098, 10_101, 9_600)
        );
        assert_eq!(w.clock_error_ns, 0);
    }

    #[test]
    fn backward_or_divergent_system_clock_is_rejected() {
        let t = Instant::now();
        let begin = || Anchor {
            before: t,
            wall: 10_000,
            after: t,
        };
        let after = t + Duration::from_millis(1);
        for wall in [9_999, 10_001, 40_000] {
            assert!(
                Window::new(
                    begin(),
                    Anchor {
                        before: after,
                        wall,
                        after
                    },
                    t,
                    after
                )
                .is_err()
            );
        }
    }

    #[test]
    fn filetime_conversion_retains_integer_precision() {
        assert_eq!(filetime(UNIX_EPOCH).unwrap(), FILETIME_UNIX_EPOCH);
        assert_eq!(
            filetime(UNIX_EPOCH + Duration::from_nanos(299)).unwrap(),
            FILETIME_UNIX_EPOCH + 2
        );
        assert!(filetime(UNIX_EPOCH - Duration::from_secs(1)).is_err());
    }
}
