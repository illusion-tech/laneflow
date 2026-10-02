//! #814：公共 step 外侧的 Windows 时间锚点；不修改交通计算或按时间比例分配 PMC。
use std::{
    io,
    sync::OnceLock,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const FILETIME_UNIX_EPOCH: u64 = 116_444_736_000_000_000;
const MAX_CLOCK_ERROR_NS: u128 = 100_000;
const ANCHOR_SAMPLES: usize = 3;
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

#[derive(Clone, Copy)]
pub(crate) struct Anchor {
    before: Instant,
    wall: u64,
    after: Instant,
    samples: usize,
    max_span_ns: u64,
    sample_clock_error_ns: u64,
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
        Self::select([Self::sample()?, Self::sample()?, Self::sample()?])
    }

    fn sample() -> io::Result<Self> {
        let before = Instant::now();
        let wall = SystemTime::now();
        let after = Instant::now();
        Ok(Self {
            before,
            wall: filetime(wall)?,
            after,
            samples: 1,
            max_span_ns: u64::try_from(after.duration_since(before).as_nanos()).map_err(invalid)?,
            sample_clock_error_ns: 0,
        })
    }

    // 固定采三组锚点，只选择时钟校准括号最窄的一组；不重跑或筛选任何 step。
    // 所有相邻采样仍检查时钟连续性，并导出最宽括号，不能借选择隐藏跳时。
    fn select(samples: [Self; ANCHOR_SAMPLES]) -> io::Result<Self> {
        let mut selected = samples[0];
        let mut max_span = 0;
        let mut max_error = 0;
        for sample in samples {
            if sample.before > sample.after {
                return Err(invalid("anchor clock order"));
            }
            let span = sample.after.duration_since(sample.before);
            max_span = max_span.max(span.as_nanos());
            if span < selected.after.duration_since(selected.before) {
                selected = sample;
            }
        }
        for pair in samples.windows(2) {
            let (previous, next) = (pair[0], pair[1]);
            if previous.after > next.before || previous.wall > next.wall {
                return Err(invalid("anchor clock order"));
            }
            let utc_ns = u128::from(next.wall - previous.wall) * 100;
            let lo = next.before.duration_since(previous.after).as_nanos();
            let hi = next.after.duration_since(previous.before).as_nanos();
            max_error = max_error.max(lo.saturating_sub(utc_ns).max(utc_ns.saturating_sub(hi)));
        }
        if max_error > MAX_CLOCK_ERROR_NS {
            return Err(invalid("system clock diverged while sampling anchors"));
        }
        // FILETIME 向外取整最多再引入两个 100 ns 单位；在采集时就拒绝不够精确的锚点。
        if selected.after.duration_since(selected.before).as_nanos() + 200 > MAX_CLOCK_ERROR_NS {
            return Err(invalid("Windows anchor uncertainty too large"));
        }
        selected.samples = ANCHOR_SAMPLES;
        selected.max_span_ns = u64::try_from(max_span).map_err(invalid)?;
        selected.sample_clock_error_ns = u64::try_from(max_error).map_err(invalid)?;
        Ok(selected)
    }
}

pub(crate) struct Window {
    pub(crate) start_lower: u64,
    pub(crate) start_upper: u64,
    pub(crate) end_lower: u64,
    pub(crate) end_upper: u64,
    pub(crate) step_ns: u64,
    pub(crate) clock_error_ns: u64,
    start_anchor_samples: usize,
    end_anchor_samples: usize,
    start_anchor_max_span_ns: u64,
    end_anchor_max_span_ns: u64,
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
        let error = lo
            .saturating_sub(utc_ns)
            .max(utc_ns.saturating_sub(hi))
            .max(u128::from(begin.sample_clock_error_ns))
            .max(u128::from(end.sample_clock_error_ns));
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
        if u128::from((start_upper - start_lower).max(end_upper - end_lower)) * 100
            > MAX_CLOCK_ERROR_NS
        {
            return Err(invalid("Windows anchor uncertainty too large"));
        }
        Ok(Self {
            start_lower,
            start_upper,
            end_lower,
            end_upper,
            step_ns: u64::try_from(finished.duration_since(started).as_nanos()).map_err(invalid)?,
            clock_error_ns: u64::try_from(error).map_err(invalid)?,
            start_anchor_samples: begin.samples,
            end_anchor_samples: end.samples,
            start_anchor_max_span_ns: begin.max_span_ns,
            end_anchor_max_span_ns: end.max_span_ns,
        })
    }

    pub(crate) fn emit(&self, tick: u64) {
        eprintln!(
            "LF814_WPR {{\"pid\":{},\"tick\":{},\"step_ns\":{},\"start_lower_filetime\":{},\"start_upper_filetime\":{},\"end_lower_filetime\":{},\"end_upper_filetime\":{},\"clock_error_ns\":{},\"start_anchor_samples\":{},\"end_anchor_samples\":{},\"start_anchor_max_span_ns\":{},\"end_anchor_max_span_ns\":{}}}",
            std::process::id(),
            tick,
            self.step_ns,
            self.start_lower,
            self.start_upper,
            self.end_lower,
            self.end_upper,
            self.clock_error_ns,
            self.start_anchor_samples,
            self.end_anchor_samples,
            self.start_anchor_max_span_ns,
            self.end_anchor_max_span_ns
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn anchor(before: Instant, wall: u64, after: Instant) -> Anchor {
        Anchor {
            before,
            wall,
            after,
            samples: 1,
            max_span_ns: after.duration_since(before).as_nanos() as u64,
            sample_clock_error_ns: 0,
        }
    }

    #[test]
    fn anchor_bounds_enclose_step_and_preserve_monotonic_latency() {
        let t = Instant::now();
        let at = |n| t + Duration::from_nanos(n);
        let begin = anchor(at(0), 10_000, at(200));
        let end = anchor(at(10_000), 10_101, at(10_200));
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
        let begin = || anchor(t, 10_000, t);
        let after = t + Duration::from_millis(1);
        for wall in [9_999, 10_001, 40_000] {
            assert!(Window::new(begin(), anchor(after, wall, after), t, after).is_err());
        }
    }

    #[test]
    fn fixed_anchor_samples_keep_wide_observation_and_monotonic_step() {
        let t = Instant::now();
        let at = |n| t + Duration::from_nanos(n);
        let begin = Anchor::select([
            anchor(at(0), 10_100, at(140_000)),
            anchor(at(141_000), 11_411, at(141_200)),
            anchor(at(142_000), 11_421, at(142_100)),
        ])
        .unwrap();
        assert_eq!(begin.samples, 3);
        assert_eq!(begin.max_span_ns, 140_000);
        assert_eq!(begin.before, at(142_000));
        let end = anchor(at(200_000), 12_001, at(200_200));
        let window = Window::new(begin, end, at(143_000), at(199_000)).unwrap();
        assert_eq!(window.step_ns, 56_000);
        assert!(window.start_upper - window.start_lower <= 3);
        assert_eq!(window.start_anchor_max_span_ns, 140_000);
    }

    #[test]
    fn anchor_selection_rejects_jump_in_an_unselected_sample() {
        let t = Instant::now();
        let at = |n| t + Duration::from_nanos(n);
        assert!(
            Anchor::select([
                anchor(at(0), 10_000, at(100)),
                anchor(at(1_000), 12_011, at(1_200)),
                anchor(at(2_000), 12_021, at(2_200)),
            ])
            .is_err()
        );
    }

    #[test]
    fn all_wide_anchors_and_manual_wide_window_are_rejected() {
        let t = Instant::now();
        let at = |n| t + Duration::from_nanos(n);
        assert!(
            Anchor::select([
                anchor(at(0), 10_000, at(110_000)),
                anchor(at(120_000), 11_200, at(230_000)),
                anchor(at(240_000), 12_400, at(350_000)),
            ])
            .is_err()
        );
        assert!(
            Window::new(
                anchor(at(0), 10_000, at(110_000)),
                anchor(at(130_000), 11_300, at(130_100)),
                at(120_000),
                at(129_000)
            )
            .is_err()
        );
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
