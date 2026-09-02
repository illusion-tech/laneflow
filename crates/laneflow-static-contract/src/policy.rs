//! 路权策略的无分配封闭值域；不产生运行时 grant 或 reservation。

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
