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

/// 策略局部成员的种类；LFSD 的成员键使用此代码区分同名成员。
///
/// 这是独立于 Identity 实体种类、LFSM 来源角色和道路编辑来源关系的闭合登记。
/// 局部成员不因此获得独立 StableId。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PolicyLocalMemberKind {
    /// 策略依据。
    Evidence,
    /// 间隙参数。
    GapProfile,
    /// 通行流规则。
    StreamRule,
    /// 门规则。
    GateRule,
}

impl PolicyLocalMemberKind {
    /// 返回 LFSD PolicyLocalChange 的 memberKind 代码。
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Evidence => 0,
            Self::GapProfile => 1,
            Self::StreamRule => 2,
            Self::GateRule => 3,
        }
    }

    /// 解码局部成员种类；来源角色或其他未知代码不能充当成员种类。
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Evidence),
            1 => Some(Self::GapProfile),
            2 => Some(Self::StreamRule),
            3 => Some(Self::GateRule),
            _ => None,
        }
    }
}

/// 以同一策略局部成员键配对后得到的变更操作。
///
/// 前后规范值相同或两侧均无该成员时没有变更行；同键修改只有 Modify，
/// 不能用 Remove 加 Add 代替。此值只登记代码与载荷存在性，不比较两端内容。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PolicyLocalChangeKind {
    /// 仅目标侧存在该成员。
    Add,
    /// 仅基础侧存在该成员。
    Remove,
    /// 两侧都有该成员，且完整规范值不同。
    Modify,
}

impl PolicyLocalChangeKind {
    /// 返回 LFSD PolicyLocalChange 的 changeKind 代码。
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Add => 0,
            Self::Remove => 1,
            Self::Modify => 2,
        }
    }

    /// 解码已登记的操作；未知代码不能默认成新增或无变化。
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Add),
            1 => Some(Self::Remove),
            2 => Some(Self::Modify),
            _ => None,
        }
    }

    /// 基础侧完整规范值是否必需；返回 false 时该字段禁止存在。
    #[must_use]
    pub const fn requires_before_value(self) -> bool {
        matches!(self, Self::Remove | Self::Modify)
    }

    /// 目标侧完整规范值是否必需；返回 false 时该字段禁止存在。
    #[must_use]
    pub const fn requires_after_value(self) -> bool {
        matches!(self, Self::Add | Self::Modify)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_member_codes_match_lfsd_without_accepting_lfsm_roles() {
        // 实施合同 §4.3.1 的固定 wire 登记；不是根据 enum 顺序生成期望值。
        for (code, member) in [
            (0, PolicyLocalMemberKind::Evidence),
            (1, PolicyLocalMemberKind::GapProfile),
            (2, PolicyLocalMemberKind::StreamRule),
            (3, PolicyLocalMemberKind::GateRule),
        ] {
            assert_eq!(PolicyLocalMemberKind::from_code(code), Some(member));
            assert_eq!(member.code(), code);
        }
        // 其中包含 Road Editing relation 16–19 与 LFSM role 33–36。
        for code in 4..=u8::MAX {
            assert_eq!(PolicyLocalMemberKind::from_code(code), None);
        }
    }

    #[test]
    fn local_change_codes_require_the_frozen_payload_sides() {
        for (code, change, before, after) in [
            (0, PolicyLocalChangeKind::Add, false, true),
            (1, PolicyLocalChangeKind::Remove, true, false),
            (2, PolicyLocalChangeKind::Modify, true, true),
        ] {
            assert_eq!(PolicyLocalChangeKind::from_code(code), Some(change));
            assert_eq!(change.code(), code);
            assert_eq!(change.requires_before_value(), before);
            assert_eq!(change.requires_after_value(), after);
        }
        for code in 3..=u8::MAX {
            assert_eq!(PolicyLocalChangeKind::from_code(code), None);
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
