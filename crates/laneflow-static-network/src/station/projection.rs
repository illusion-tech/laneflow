//! 在候选边归属确定前保留线性分段的连续插值与反查包络。
use super::{NM_PER_MM, Segment};

impl Segment {
    /// 计算指定站位的插值进度并直接舍入到最近偶数毫米。域外站位返回空。
    #[must_use]
    #[cfg(test)]
    fn progress_mm_at(self, station_nm: u64) -> Option<u64> {
        let (whole_nm, fractional) = self.interpolate(station_nm)?;
        Some(round_mm(whole_nm, fractional))
    }

    /// 在指定站位应用本段规范误差，返回实际进度可能舍入到的毫米闭区间。
    /// 调用方另按所属边的交通长度裁剪；切点的精确零进度由边归属确定。
    #[must_use]
    pub(super) fn progress_mm_bounds_at(self, station_nm: u64) -> Option<[u64; 2]> {
        let (whole_nm, fractional) = self.interpolate(station_nm)?;
        let error = u128::from(self.map.canonical_error_mm()) * u128::from(NM_PER_MM);
        Some([
            round_mm(whole_nm.saturating_sub(error), fractional),
            round_mm(whole_nm + error, fractional),
        ])
    }

    fn interpolate(self, station: u64) -> Option<(u128, bool)> {
        let [a, b] = self.stations_nm();
        if !(a.min(b)..=a.max(b)).contains(&station) {
            return None;
        }
        let length = u128::from(a.abs_diff(b));
        let numerator = u128::from(a.abs_diff(station))
            * u128::from((self.map.progress_mm()[1] - self.map.progress_mm()[0]) * NM_PER_MM);
        Some((
            u128::from(self.map.progress_mm()[0] * NM_PER_MM) + numerator / length,
            !numerator.is_multiple_of(length),
        ))
    }

    /// 从边内整数毫米观测反查本段的可能站位，端点按纳米向外取整。
    /// 即使观测为零，也须包含能舍入为零的内部位置。无交集时返回空。
    #[must_use]
    pub(super) fn stations_for_progress_mm(self, observed_mm: u32) -> Option<[u64; 2]> {
        let center = u64::from(observed_mm) * NM_PER_MM;
        let radius = NM_PER_MM / 2 + u64::from(self.map.canonical_error_mm()) * NM_PER_MM;
        let [p, q] = self.map.progress_mm().map(|mm| mm * NM_PER_MM);
        let low = p.max(center.saturating_sub(radius));
        let high = q.min(center + radius);
        // 奇数格的两端归相邻偶数。仅在相交部分恰好只有这个点时可排除该段。
        if low > high || (low == high && observed_mm % 2 == 1 && center.abs_diff(low) == radius) {
            return None;
        }
        let [a, b] = self.stations_nm();
        let length = u128::from(a.abs_diff(b));
        let delta = u128::from(q - p);
        let first = (u128::from(low - p) * length / delta) as u64;
        let last = (u128::from(high - p) * length).div_ceil(delta) as u64;
        Some(if a < b {
            [a + first, a + last]
        } else {
            [a - last, a - first]
        })
    }
}

// whole_nm 是精确有理值的下取整。余下不足一纳米，仅在恰逢半毫米时影响取偶。
fn round_mm(whole_nm: u128, fractional: bool) -> u64 {
    let unit = u128::from(NM_PER_MM);
    let mm = whole_nm / unit;
    let rem = whole_nm % unit;
    (mm + u128::from(rem > unit / 2 || (rem == unit / 2 && (fractional || mm % 2 == 1)))) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use laneflow_static_contract::{LaneEdgeOrdinal, StationSpan, StationSpanError};

    fn segment(
        stations: [u64; 2],
        progress: [u64; 2],
        canonical_error: u32,
        source_error: u32,
        min_rate: u32,
    ) -> Result<Segment, StationSpanError> {
        Ok(Segment {
            edge: LaneEdgeOrdinal::from_raw(0),
            map: StationSpan::try_from_millimetres(
                stations,
                progress,
                canonical_error,
                source_error,
                min_rate,
            )?,
        })
    }

    #[test]
    fn graded_bend_observation_straddles_the_other_lanes_cut() {
        // #1516 带坡弯道 A 车道 59→68 米边的一条实际规范弦，按发布的毫米格。
        let span = segment([59_861, 60_426], [565, 1_131], 1, 1, 900_000).unwrap();
        let cut = 60_301_000_000;
        for station in [cut - 1_005, cut, cut + 1_005] {
            assert_eq!(span.progress_mm_at(station), Some(1_006));
        }
        assert_eq!(
            span.stations_for_progress_mm(1_005),
            Some([60_298_725_265, 60_301_719_965])
        );

        let wide = segment(
            span.map.stations_mm(),
            span.map.progress_mm(),
            10,
            span.map.source_travel_error_mm(),
            span.map.min_rate_ppm(),
        )
        .unwrap();
        assert_eq!(
            wide.stations_for_progress_mm(1_005),
            Some([60_289_741_166, 60_310_704_064])
        );
        for observed_mm in 994..=1_016 {
            for (mapping, overlaps_cut) in [
                (span, (1_005..=1_007).contains(&observed_mm)),
                (wide, (996..=1_016).contains(&observed_mm)),
            ] {
                let [lo, hi] = mapping.stations_for_progress_mm(observed_mm).unwrap();
                assert_eq!(lo < cut && hi >= cut, overlaps_cut);
            }
        }
    }

    #[test]
    fn nearest_even_uses_the_exact_fraction_before_rounding() {
        for endpoints in [[0, 4], [4, 0]] {
            let span = segment(endpoints, [0, 2], 0, 0, 500_000).unwrap();
            let station = |offset| (endpoints[0] * NM_PER_MM).abs_diff(offset);
            for (offset, mm) in [
                (999_999, 0),
                (1_000_000, 0),
                (1_000_001, 1),
                (2_999_999, 1),
                (3_000_000, 2),
                (3_000_001, 2),
            ] {
                assert_eq!(span.progress_mm_at(station(offset)), Some(mm));
                let [low, high] = span.stations_for_progress_mm(mm as u32).unwrap();
                assert!((low..=high).contains(&station(offset)));
            }
            assert_eq!(span.progress_mm_at(4_000_001), None);
            assert_eq!(span.stations_for_progress_mm(3), None);
        }
    }

    #[test]
    fn half_millimetre_observations_and_per_span_error() {
        let exact = segment([12, 13], [0, 1], 0, 0, 1_000_000).unwrap();
        assert_eq!(
            exact.stations_for_progress_mm(0),
            Some([12_000_000, 12_500_000])
        );
        assert_eq!(exact.progress_mm_at(12_500_000), Some(0));
        let expanded = segment([13, 12], [0, 1], 1, 0, 1_000_000).unwrap();
        assert_eq!(
            expanded.stations_for_progress_mm(1),
            Some([12_000_000, 13_000_000])
        );
        assert_eq!(expanded.progress_mm_bounds_at(12_000_000), Some([0, 2]));
        assert_eq!(expanded.progress_mm_bounds_at(13_000_000), Some([0, 1]));
    }

    #[test]
    fn interpolation_covers_the_integer_limits() {
        let max = StationSpan::MAX_DISTANCE_MM;
        let full = segment([max, 0], [0, max], 10, 1, 1_000_000).unwrap();
        assert_eq!(full.progress_mm_at(0), Some(max));
        assert_eq!(
            full.stations_for_progress_mm(0),
            Some([max * NM_PER_MM - 10_500_000, max * NM_PER_MM])
        );
        assert_eq!(full.progress_mm_bounds_at(0), Some([max - 10, max + 10]));
    }
}
