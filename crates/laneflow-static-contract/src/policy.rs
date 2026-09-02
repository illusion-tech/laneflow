//! 路权策略的无分配封闭值域；不产生运行时 grant 或 reservation。

use core::{fmt, str::FromStr};

/// 法规适用公历日期；规范数值为 YYYYMMDD，年份范围为 0001..=9999。
///
/// 不读取宿主日期，不表示 Unix 时间，未知日期不能用零代替。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RegulationDate(u32);

impl RegulationDate {
    /// 验证规范整数中的年月日，包括公历世纪闰年规则。
    #[must_use]
    pub const fn from_yyyymmdd(value: u32) -> Option<Self> {
        let year = value / 10_000;
        let month = value / 100 % 100;
        let day = value % 100;
        if year == 0 || year > 9999 {
            return None;
        }
        let leap =
            year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let last_day = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => return None,
        };
        if day == 0 || day > last_day {
            return None;
        }
        Some(Self(value))
    }

    /// 返回已验证的 YYYYMMDD 规范整数。
    #[must_use]
    pub const fn yyyymmdd(self) -> u32 {
        self.0
    }
}

/// 法规日期文本的封闭解析错误。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegulationDateError {
    /// 输入不是十个 ASCII 字节的 YYYY-MM-DD。
    InvalidFormat,
    /// 年份越界或对应公历日期不存在。
    InvalidDate,
}

impl fmt::Display for RegulationDateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidFormat => "expected YYYY-MM-DD with ASCII digits",
            Self::InvalidDate => "invalid Gregorian date in years 0001..=9999",
        })
    }
}

impl core::error::Error for RegulationDateError {}

impl FromStr for RegulationDate {
    type Err = RegulationDateError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bytes = text.as_bytes();
        if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
            return Err(RegulationDateError::InvalidFormat);
        }
        let mut value = 0_u32;
        for (index, &byte) in bytes.iter().enumerate() {
            if index == 4 || index == 7 {
                continue;
            }
            if !byte.is_ascii_digit() {
                return Err(RegulationDateError::InvalidFormat);
            }
            // 恰好八个十进制数字，其最大值可由 u32 精确表示。
            value = value * 10 + u32::from(byte - b'0');
        }
        Self::from_yyyymmdd(value).ok_or(RegulationDateError::InvalidDate)
    }
}

impl fmt::Display for RegulationDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02}",
            self.0 / 10_000,
            self.0 / 100 % 100,
            self.0 % 100
        )
    }
}

/// 编制来源明确声明的机动方向；缺失由外层 Option 表达。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ManeuverDirection {
    /// 直行。
    Straight,
    /// 左转。
    Left,
    /// 右转。
    Right,
    /// 掉头。
    UTurn,
}

impl ManeuverDirection {
    /// 返回实施合同登记的方向代码，不依赖 Rust enum 内存布局。
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Straight => 0,
            Self::Left => 1,
            Self::Right => 2,
            Self::UTurn => 3,
        }
    }

    /// 解码已登记的方向；未知值必须由调用方报告，不能默认成直行。
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Straight),
            1 => Some(Self::Left),
            2 => Some(Self::Right),
            3 => Some(Self::UTurn),
            _ => None,
        }
    }
}

/// 门规则的封闭灯态解释声明；不是最终通行授权。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GateInterpretation {
    /// 无信号绑定的无控制候选。
    Uncontrolled,
    /// 信号组绿色产生受保护候选。
    ProtectedGroup,
    /// 信号组绿色产生许可候选。
    PermissiveGroup,
    /// 中国普通圆形灯右转解释，红色仍只能产生有条件候选。
    CnCircularRightTurn,
    /// 右转方向灯的受保护解释。
    DirectionalRightProtected,
    /// 右转方向灯的许可解释。
    DirectionalRightPermissive,
}

impl GateInterpretation {
    /// 返回实施合同登记的解释代码。
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Uncontrolled => 0,
            Self::ProtectedGroup => 1,
            Self::PermissiveGroup => 2,
            Self::CnCircularRightTurn => 3,
            Self::DirectionalRightProtected => 4,
            Self::DirectionalRightPermissive => 5,
        }
    }

    /// 解码已登记的解释；不接受自由表达式或未知代码。
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Uncontrolled),
            1 => Some(Self::ProtectedGroup),
            2 => Some(Self::PermissiveGroup),
            3 => Some(Self::CnCircularRightTurn),
            4 => Some(Self::DirectionalRightProtected),
            5 => Some(Self::DirectionalRightPermissive),
            _ => None,
        }
    }
}

/// 独立于灯型声明的显式门禁令。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GateProhibition {
    /// 没有本层禁令，不代表可以覆盖其他安全拒绝。
    None,
    /// 始终拒绝入口候选。
    Always,
    /// 红色信号指示时拒绝入口候选。
    OnRed,
}

impl GateProhibition {
    /// 返回实施合同登记的禁令代码。
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Always => 1,
            Self::OnRed => 2,
        }
    }

    /// 解码显式禁令；未知代码不能默认成 None。
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::None),
            1 => Some(Self::Always),
            2 => Some(Self::OnRed),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::string::ToString;

    #[test]
    fn regulation_dates_preserve_calendar_and_canonical_encoding() {
        for (text, encoded) in [
            ("0001-01-01", 10_101),
            ("1900-02-28", 19_000_228),
            ("2000-02-29", 20_000_229),
            ("2024-02-29", 20_240_229),
            ("2026-09-03", 20_260_903),
            ("9999-12-31", 99_991_231),
        ] {
            let date: RegulationDate = text.parse().unwrap();
            assert_eq!(date.yyyymmdd(), encoded);
            assert_eq!(RegulationDate::from_yyyymmdd(encoded), Some(date));
            assert_eq!(date.to_string(), text);
        }
        assert!(
            "2026-09-03".parse::<RegulationDate>().unwrap()
                < "2027-01-01".parse::<RegulationDate>().unwrap()
        );
    }

    #[test]
    fn invalid_dates_and_noncanonical_text_are_rejected() {
        for text in [
            "0000-01-01",
            "1900-02-29",
            "2100-02-29",
            "2026-04-31",
            "2026-00-01",
            "2026-13-01",
            "2026-01-00",
            "2026-01-32",
        ] {
            assert_eq!(
                text.parse::<RegulationDate>(),
                Err(RegulationDateError::InvalidDate),
                "{text}"
            );
        }
        for text in [
            "",
            "2026-9-03",
            "2026/09/03",
            "2026-09-03 ",
            " 2026-09-03",
            "+026-09-03",
            "２０２６-09-03",
            "2026-09-03T00:00:00",
            "10000-01-01",
        ] {
            assert_eq!(
                text.parse::<RegulationDate>(),
                Err(RegulationDateError::InvalidFormat),
                "{text}"
            );
        }
        for value in [0, 101, 20_260_229, 100_000_101, u32::MAX] {
            assert_eq!(RegulationDate::from_yyyymmdd(value), None);
        }
    }

    #[test]
    fn policy_codes_match_the_frozen_wire_registry() {
        for (value, code) in [
            (ManeuverDirection::Straight, 0),
            (ManeuverDirection::Left, 1),
            (ManeuverDirection::Right, 2),
            (ManeuverDirection::UTurn, 3),
        ] {
            assert_eq!(value.code(), code);
            assert_eq!(ManeuverDirection::from_code(code), Some(value));
        }
        for (value, code) in [
            (GateInterpretation::Uncontrolled, 0),
            (GateInterpretation::ProtectedGroup, 1),
            (GateInterpretation::PermissiveGroup, 2),
            (GateInterpretation::CnCircularRightTurn, 3),
            (GateInterpretation::DirectionalRightProtected, 4),
            (GateInterpretation::DirectionalRightPermissive, 5),
        ] {
            assert_eq!(value.code(), code);
            assert_eq!(GateInterpretation::from_code(code), Some(value));
        }
        for (value, code) in [
            (GateProhibition::None, 0),
            (GateProhibition::Always, 1),
            (GateProhibition::OnRed, 2),
        ] {
            assert_eq!(value.code(), code);
            assert_eq!(GateProhibition::from_code(code), Some(value));
        }
        for code in 0..=u8::MAX {
            assert_eq!(ManeuverDirection::from_code(code).is_some(), code <= 3);
            assert_eq!(GateInterpretation::from_code(code).is_some(), code <= 5);
            assert_eq!(GateProhibition::from_code(code).is_some(), code <= 2);
        }
    }
}
