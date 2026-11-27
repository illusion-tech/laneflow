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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_rejects_invalid_domains_and_out_of_range_lengths() {
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
    }
}
