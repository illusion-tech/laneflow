//! 走廊共享里程映射。端点、进度和发布误差以整毫米保存，连续反算才放大分子。

const NM_PER_MM: u64 = 1_000_000;

/// 站位映射分段无法满足发布条件的原因。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StationSpanError {
    /// 毫米字段越过查询算术可表示的范围。
    UnrepresentableLength,
    /// 两端站位重合。
    EmptyStationInterval,
    /// 边内进度没有沿行驶方向严格增加。
    NonIncreasingProgress,
    /// 规范进度误差超过 1 厘米。
    CanonicalError,
    /// 边首累计的原线行程偏差超过 1 毫米。
    SourceTravelError,
    /// 登记的正斜率下界为零或超过插值函数的绝对斜率。
    MinimumRate,
}

/// 由两端锚点与边内累计进度确定的有向线性分段。
///
/// 构造时核对可由制品字段直接检查的条件。连续原线的速度与行程证据由编译器提供。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StationSpan {
    stations: [u64; 2],
    progress: [u64; 2],
    canonical_error_mm: u32,
    source_travel_error_mm: u32,
    min_rate_ppm: u32,
}

impl StationSpan {
    /// 连续反算的向外取整端点以 u64 纳米表示，发布长度须能安全换算。
    pub const MAX_DISTANCE_MM: u64 = u64::MAX / NM_PER_MM;
    /// 规范进度插值允许的最大整毫米误差。
    pub const MAX_CANONICAL_ERROR_MM: u32 = 10;
    /// 原线行程、站位锚点和路幅各自允许的最大整毫米误差。
    pub const MAX_SOURCE_ERROR_MM: u32 = 1;

    /// 从按行驶方向排列的两端值和发布证据构造分段。
    ///
    /// # Errors
    ///
    /// 端点退化、进度倒退、误差超限或斜率下界无效时返回首项违反条件。
    pub fn try_from_millimetres(
        stations: [u64; 2],
        progress: [u64; 2],
        canonical_error_mm: u32,
        source_travel_error_mm: u32,
        min_rate_ppm: u32,
    ) -> Result<Self, StationSpanError> {
        if stations
            .into_iter()
            .chain(progress)
            .any(|mm| mm > Self::MAX_DISTANCE_MM)
        {
            return Err(StationSpanError::UnrepresentableLength);
        }
        let length = stations[0].abs_diff(stations[1]);
        if length == 0 {
            return Err(StationSpanError::EmptyStationInterval);
        }
        if progress[0] >= progress[1] {
            return Err(StationSpanError::NonIncreasingProgress);
        }
        if canonical_error_mm > Self::MAX_CANONICAL_ERROR_MM {
            return Err(StationSpanError::CanonicalError);
        }
        if source_travel_error_mm > Self::MAX_SOURCE_ERROR_MM {
            return Err(StationSpanError::SourceTravelError);
        }
        if min_rate_ppm == 0
            || u128::from(progress[1] - progress[0]) * 1_000_000
                < u128::from(length) * u128::from(min_rate_ppm)
        {
            return Err(StationSpanError::MinimumRate);
        }
        Ok(Self {
            stations,
            progress,
            canonical_error_mm,
            source_travel_error_mm,
            min_rate_ppm,
        })
    }

    /// 返回行驶方向上起、终锚点的走廊站位。
    #[must_use]
    pub const fn stations_mm(self) -> [u64; 2] {
        self.stations
    }

    /// 返回行驶方向上两端的边内进度。
    #[must_use]
    pub const fn progress_mm(self) -> [u64; 2] {
        self.progress
    }

    /// 返回规范进度插值误差上界，以毫米计。
    #[must_use]
    pub const fn canonical_error_mm(self) -> u32 {
        self.canonical_error_mm
    }

    /// 返回原线累计行程相对规范进度的误差上界，以毫米计。
    #[must_use]
    pub const fn source_travel_error_mm(self) -> u32 {
        self.source_travel_error_mm
    }

    /// 返回登记的绝对斜率下界，以百万分数计。
    #[must_use]
    pub const fn min_rate_ppm(self) -> u32 {
        self.min_rate_ppm
    }

    /// 计算指定站位的插值进度并直接舍入到最近偶数毫米。域外站位返回空。
    #[must_use]
    pub fn progress_mm_at(self, station_nm: u64) -> Option<u64> {
        let (whole_nm, fractional) = self.interpolate(station_nm)?;
        Some(round_mm(whole_nm, fractional))
    }

    /// 在指定站位应用本段规范误差，返回实际进度可能舍入到的毫米闭区间。
    /// 调用方另按所属边的交通长度裁剪；切点的精确零进度由边归属确定。
    #[must_use]
    pub fn progress_mm_bounds_at(self, station_nm: u64) -> Option<[u64; 2]> {
        let (whole_nm, fractional) = self.interpolate(station_nm)?;
        let error = u128::from(self.canonical_error_mm) * u128::from(NM_PER_MM);
        Some([
            round_mm(whole_nm.saturating_sub(error), fractional),
            round_mm(whole_nm + error, fractional),
        ])
    }

    fn interpolate(self, station: u64) -> Option<(u128, bool)> {
        let [a, b] = self.stations.map(|mm| mm * NM_PER_MM);
        if !(a.min(b)..=a.max(b)).contains(&station) {
            return None;
        }
        let length = u128::from(a.abs_diff(b));
        let numerator = u128::from(a.abs_diff(station))
            * u128::from((self.progress[1] - self.progress[0]) * NM_PER_MM);
        Some((
            u128::from(self.progress[0] * NM_PER_MM) + numerator / length,
            !numerator.is_multiple_of(length),
        ))
    }

    /// 从边内整数毫米观测反查本段的可能站位，端点按纳米向外取整。
    /// 即使观测为零，也须包含能舍入为零的内部位置。无交集时返回空。
    #[must_use]
    pub fn stations_for_progress_mm(self, observed_mm: u32) -> Option<[u64; 2]> {
        let center = u64::from(observed_mm) * NM_PER_MM;
        let radius = NM_PER_MM / 2 + u64::from(self.canonical_error_mm) * NM_PER_MM;
        let [p, q] = self.progress.map(|mm| mm * NM_PER_MM);
        let low = p.max(center.saturating_sub(radius));
        let high = q.min(center + radius);
        // 奇数格的两端归相邻偶数。仅在相交部分恰好只有这个点时可排除该段。
        if low > high || (low == high && observed_mm % 2 == 1 && center.abs_diff(low) == radius) {
            return None;
        }
        let [a, b] = self.stations.map(|mm| mm * NM_PER_MM);
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

    #[test]
    fn graded_bend_observation_straddles_the_other_lanes_cut() {
        // #1516 带坡弯道 A 车道 59→68 米边的一条实际规范弦，按发布的毫米格。
        let span = StationSpan::try_from_millimetres([59_861, 60_426], [565, 1_131], 1, 1, 900_000)
            .unwrap();
        let cut = 60_301_000_000;
        for station in [cut - 1_005, cut, cut + 1_005] {
            assert_eq!(span.progress_mm_at(station), Some(1_006));
        }
        assert_eq!(
            span.stations_for_progress_mm(1_005),
            Some([60_298_725_265, 60_301_719_965])
        );

        let wide = StationSpan::try_from_millimetres(
            span.stations_mm(),
            span.progress_mm(),
            10,
            span.source_travel_error_mm(),
            span.min_rate_ppm(),
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
            let span = StationSpan::try_from_millimetres(endpoints, [0, 2], 0, 0, 500_000).unwrap();
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
        let exact = StationSpan::try_from_millimetres([12, 13], [0, 1], 0, 0, 1_000_000).unwrap();
        assert_eq!(
            exact.stations_for_progress_mm(0),
            Some([12_000_000, 12_500_000])
        );
        assert_eq!(exact.progress_mm_at(12_500_000), Some(0));
        let expanded =
            StationSpan::try_from_millimetres([13, 12], [0, 1], 1, 0, 1_000_000).unwrap();
        assert_eq!(
            expanded.stations_for_progress_mm(1),
            Some([12_000_000, 13_000_000])
        );
        assert_eq!(expanded.progress_mm_bounds_at(12_000_000), Some([0, 2]));
        assert_eq!(expanded.progress_mm_bounds_at(13_000_000), Some([0, 1]));
    }

    #[test]
    fn validation_and_products_cover_the_integer_limits() {
        assert_eq!(
            StationSpan::try_from_millimetres([0, 0], [9, 9], 0, 0, 1),
            Err(StationSpanError::EmptyStationInterval)
        );
        assert_eq!(
            StationSpan::try_from_millimetres([0, 2], [9, 9], 0, 0, 1),
            Err(StationSpanError::NonIncreasingProgress)
        );
        for (error, source, rate, expected) in [
            (11, 0, 1, StationSpanError::CanonicalError),
            (0, 2, 1, StationSpanError::SourceTravelError),
            (0, 0, 0, StationSpanError::MinimumRate),
            (0, 0, 500_001, StationSpanError::MinimumRate),
        ] {
            assert_eq!(
                StationSpan::try_from_millimetres([1, 3], [3, 4], error, source, rate),
                Err(expected)
            );
        }
        let max = u64::MAX / NM_PER_MM;
        assert_eq!(
            StationSpan::try_from_millimetres([max + 1, 0], [0, 1], 0, 0, 1),
            Err(StationSpanError::UnrepresentableLength)
        );
        let full = StationSpan::try_from_millimetres([max, 0], [0, max], 10, 1, 1_000_000).unwrap();
        assert_eq!(full.progress_mm_at(0), Some(max));
        assert_eq!(
            full.stations_for_progress_mm(0),
            Some([max * NM_PER_MM - 10_500_000, max * NM_PER_MM])
        );
        assert_eq!(full.progress_mm_bounds_at(0), Some([max - 10, max + 10]));
    }
}
