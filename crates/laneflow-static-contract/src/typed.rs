//! 稳定标识与致密序号的有类型值封装。
//!
//! [`StableId128`] 跨制品、路网修订和运行时冷边界保存实体身份；[`StableId<K>`]
//! 再用实体种类消除错误混用。与之相对，[`Ordinal<K>`] 只索引某一份已验证致密表，
//! 不能持久化为实体身份，也不能在不同表或路网修订之间比较。所有类型均为无分配的
//! 透明值；本模块不派生或验证 Identity v1 的规范前像。

use core::{fmt, marker::PhantomData, str::FromStr};

use crate::EntityKind;

/// 不携带实体种类的 128 位稳定标识值。
///
/// 字节按规范摘要的原始顺序保存；本类型不验证这些字节是否由 Identity v1 前像正确
/// 派生。需要实体种类约束时应尽早转换为 [`StableId<K>`]。
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct StableId128([u8; 16]);

impl StableId128 {
    /// 全零原始值。
    ///
    /// 该常量只提供值级便利，不表示一个已经通过身份验证的真实实体。
    pub const ZERO: Self = Self([0; 16]);

    /// 原样封装 16 字节，不执行身份前像或摘要验证。
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// 借用稳定标识的 16 个原始字节。
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// 取出稳定标识的 16 个原始字节。
    #[must_use]
    pub const fn into_bytes(self) -> [u8; 16] {
        self.0
    }
}

impl fmt::LowerHex for StableId128 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for StableId128 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "StableId128({self:x})")
    }
}

mod sealed {
    /// 防止外部 crate 为未登记实体种类扩展有类型稳定标识。
    pub trait EntityKindMarker {}
    /// 防止外部 crate 为未登记表种类扩展有类型序号。
    pub trait OrdinalKind {}
}

/// 由本 crate 封闭的 Identity v1 实体种类标记。
///
/// 封闭性保证 `K::KIND` 只能来自 [`EntityKind`] 登记表，调用方不能创建与登记表竞争
/// 的公共实体种类。
pub trait EntityKindMarker: sealed::EntityKindMarker + 'static {
    /// 此标记唯一对应的 Identity v1 实体种类。
    const KIND: EntityKind;
}

/// 允许作为致密表序号类型参数的封闭标记。
///
/// 该 trait 只区分表中实体种类，不证明某个序号落在具体表的边界内。
pub trait OrdinalKind: sealed::OrdinalKind + 'static {}

/// 按实体种类区分的 128 位稳定标识。
///
/// `K` 是编译期种类约束，不增加运行时存储。构造函数不会重新计算摘要；调用者必须
/// 从受信任的身份派生或已验证数据取得原始值。
///
/// # Examples
///
/// 文本解析同时校验版本前缀、实体种类和规范小写十六进制形式：
///
/// ```
/// use laneflow_static_contract::{LaneEdgeId, StableIdTextError};
///
/// let id: LaneEdgeId = "lfid1_lane-edge_0000000000000000000000000000002a".parse()?;
/// assert_eq!(id.to_string(), "lfid1_lane-edge_0000000000000000000000000000002a");
///
/// # Ok::<(), StableIdTextError>(())
/// ```
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct StableId<K: EntityKindMarker> {
    raw: StableId128,
    marker: PhantomData<fn() -> K>,
}

impl<K: EntityKindMarker> StableId<K> {
    /// 把已确认属于 `K` 的原始稳定标识加上类型约束。
    ///
    /// 此函数不验证实体种类或 Identity v1 前像，不能用它把任意摘要“认证”为 `K`。
    #[must_use]
    pub const fn from_untyped(raw: StableId128) -> Self {
        Self {
            raw,
            marker: PhantomData,
        }
    }

    /// 借用去除类型参数后的原始稳定标识。
    #[must_use]
    pub const fn as_untyped(&self) -> &StableId128 {
        &self.raw
    }

    /// 去除类型参数并返回原始稳定标识。
    #[must_use]
    pub const fn into_untyped(self) -> StableId128 {
        self.raw
    }
}

impl<K: EntityKindMarker> fmt::Display for StableId<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "lfid1_{}_{:x}", K::KIND.slug(), self.raw)
    }
}

impl<K: EntityKindMarker> fmt::Debug for StableId<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "StableId<{}>({self})", K::KIND.slug())
    }
}

impl<K: EntityKindMarker> FromStr for StableId<K> {
    type Err = StableIdTextError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        const PREFIX: &str = "lfid1_";
        let remainder = text
            .strip_prefix(PREFIX)
            .ok_or(StableIdTextError::InvalidPrefix)?;
        let hexadecimal = remainder
            .strip_prefix(K::KIND.slug())
            .and_then(|remainder| remainder.strip_prefix('_'))
            .ok_or(StableIdTextError::UnexpectedEntityKind)?;

        if hexadecimal.len() != 32 {
            return Err(StableIdTextError::InvalidHexLength {
                actual: hexadecimal.len(),
            });
        }

        let mut bytes = [0_u8; 16];
        let (pairs, remainder) = hexadecimal.as_bytes().as_chunks::<2>();
        debug_assert!(remainder.is_empty());
        for (index, pair) in pairs.iter().enumerate() {
            let high = decode_hex_digit(pair[0], index * 2)?;
            let low = decode_hex_digit(pair[1], index * 2 + 1)?;
            bytes[index] = (high << 4) | low;
        }

        Ok(Self::from_untyped(StableId128::from_bytes(bytes)))
    }
}

fn decode_hex_digit(byte: u8, index: usize) -> Result<u8, StableIdTextError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(StableIdTextError::InvalidHexCharacter { index, byte }),
    }
}

/// 有类型稳定标识文本解析错误。
///
/// 解析器只接受 `Display` 产生的规范小写形式
/// `lfid1_<entity-kind>_<32 lowercase hex digits>`；成功解析只证明文本编码与种类
/// slug 合法，不证明摘要前像存在于任何制品。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StableIdTextError {
    /// 文本缺少 `lfid1_` 版本前缀。
    InvalidPrefix,
    /// 文本中的实体种类 slug 与类型参数 `K` 不一致。
    UnexpectedEntityKind,
    /// 十六进制正文不是恰好 32 个 ASCII 字节。
    InvalidHexLength { actual: usize },
    /// 正文包含非小写十六进制字符；`index` 是正文内的零基字节位置。
    InvalidHexCharacter { index: usize, byte: u8 },
}

impl fmt::Display for StableIdTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPrefix => formatter.write_str("稳定标识必须以 lfid1_ 开头"),
            Self::UnexpectedEntityKind => formatter.write_str("稳定标识的实体种类不匹配"),
            Self::InvalidHexLength { actual } => write!(
                formatter,
                "稳定标识的十六进制正文必须为 32 个 ASCII 字符，实际为 {actual}"
            ),
            Self::InvalidHexCharacter { index, byte } => write!(
                formatter,
                "稳定标识在字节位置 {index} 包含非法小写十六进制字符 0x{byte:02x}"
            ),
        }
    }
}

/// 仅用于已验证致密表的有类型 `u32` 序号。
///
/// 序号没有路网修订或表实例身份；同一个原始值在另一份镜像中可能指向不同实体。
/// 因此它只能在建立它的已验证表及其借用生命周期内使用，持久化边界必须改用
/// [`StableId<K>`]。
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct Ordinal<K: OrdinalKind> {
    raw: u32,
    marker: PhantomData<fn() -> K>,
}

impl<K: OrdinalKind> Ordinal<K> {
    /// 从已完成表边界验证的原始序号建立有类型值。
    ///
    /// 本函数不检查具体表长度，解析不可信输入时调用者必须先完成边界验证。
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Self {
            raw,
            marker: PhantomData,
        }
    }

    /// 返回表内原始序号。
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.raw
    }

    /// 转为当前进程的切片下标。
    ///
    /// # Panics
    ///
    /// 仅当目标平台的 `usize` 无法容纳 `u32` 时 panic；LaneFlow 支持的目标均满足该
    /// 条件。此方法不检查具体表长度。
    #[must_use]
    pub fn index(self) -> usize {
        usize::try_from(self.raw).expect("u32 ordinal must fit usize on supported targets")
    }

    /// 把切片下标转换为有类型序号，并拒绝超出 `u32` 表示范围的值。
    pub fn try_from_usize(index: usize) -> Result<Self, core::num::TryFromIntError> {
        u32::try_from(index).map(Self::from_raw)
    }
}

impl<K: OrdinalKind> fmt::Display for Ordinal<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.raw.fmt(formatter)
    }
}

impl<K: OrdinalKind> fmt::Debug for Ordinal<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Ordinal({})", self.raw)
    }
}

impl<K: OrdinalKind> From<Ordinal<K>> for u32 {
    fn from(ordinal: Ordinal<K>) -> Self {
        ordinal.raw
    }
}

macro_rules! define_entity_markers {
    ($(($marker:ident, $kind:ident, $id:ident, $ordinal:ident)),+ $(,)?) => {
        $(
            /// Identity v1 登记实体种类的零尺寸类型标记。
            #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
            pub struct $marker;

            impl sealed::EntityKindMarker for $marker {}
            impl sealed::OrdinalKind for $marker {}

            impl EntityKindMarker for $marker {
                const KIND: EntityKind = EntityKind::$kind;
            }

            impl OrdinalKind for $marker {}

            /// 该登记实体种类的有类型稳定标识。
            pub type $id = StableId<$marker>;
            /// 该登记实体种类在已验证致密表中的有类型序号。
            pub type $ordinal = Ordinal<$marker>;
        )+
    };
}

define_entity_markers!(
    (
        RoadCorridorKind,
        RoadCorridor,
        RoadCorridorId,
        RoadCorridorOrdinal
    ),
    (
        RoadSectionKind,
        RoadSection,
        RoadSectionId,
        RoadSectionOrdinal
    ),
    (
        AuthoringLaneKind,
        AuthoringLane,
        AuthoringLaneId,
        AuthoringLaneOrdinal
    ),
    (LaneEdgeKind, LaneEdge, LaneEdgeId, LaneEdgeOrdinal),
    (JunctionKind, Junction, JunctionId, JunctionOrdinal),
    (MovementKind, Movement, MovementId, MovementOrdinal),
    (
        ManeuverPathKind,
        ManeuverPath,
        ManeuverPathId,
        ManeuverPathOrdinal
    ),
    (
        ManeuverGateKind,
        ManeuverGate,
        ManeuverGateId,
        ManeuverGateOrdinal
    ),
    (
        WaitingZoneKind,
        WaitingZone,
        WaitingZoneId,
        WaitingZoneOrdinal
    ),
    (StopLineKind, StopLine, StopLineId, StopLineOrdinal),
    (
        SignalGroupKind,
        SignalGroup,
        SignalGroupId,
        SignalGroupOrdinal
    ),
    (
        SignalControllerKind,
        SignalController,
        SignalControllerId,
        SignalControllerOrdinal
    ),
    (
        SignalPhaseKind,
        SignalPhase,
        SignalPhaseId,
        SignalPhaseOrdinal
    ),
    (
        ParkingFacilityKind,
        ParkingFacility,
        ParkingFacilityId,
        ParkingFacilityOrdinal
    ),
    (
        ParkingSpaceKind,
        ParkingSpace,
        ParkingSpaceId,
        ParkingSpaceOrdinal
    ),
    (LaneGroupKind, LaneGroup, LaneGroupId, LaneGroupOrdinal),
    (
        FacilityBandKind,
        FacilityBand,
        FacilityBandId,
        FacilityBandOrdinal
    ),
    (
        ParticipantClassKind,
        ParticipantClass,
        ParticipantClassId,
        ParticipantClassOrdinal
    ),
    (AccessRuleKind, AccessRule, AccessRuleId, AccessRuleOrdinal),
    (
        VehicleProfileKind,
        VehicleProfile,
        VehicleProfileId,
        VehicleProfileOrdinal
    ),
    (
        ConflictZoneKind,
        ConflictZone,
        ConflictZoneId,
        ConflictZoneOrdinal
    ),
    (
        CanonicalFrameKind,
        CanonicalFrame,
        CanonicalFrameId,
        CanonicalFrameOrdinal
    ),
    (
        ParticipantStreamKind,
        ParticipantStream,
        ParticipantStreamId,
        ParticipantStreamOrdinal
    ),
);

#[cfg(test)]
mod tests {
    use core::mem::size_of;
    use std::string::ToString;

    use super::*;

    #[test]
    fn stable_id_value_types_have_contract_sizes() {
        assert_eq!(size_of::<StableId128>(), 16);
        assert_eq!(size_of::<RoadCorridorId>(), 16);
        assert_eq!(size_of::<LaneEdgeId>(), 16);
        assert_eq!(size_of::<LaneEdgeOrdinal>(), size_of::<u32>());
    }

    #[test]
    fn typed_stable_id_text_round_trips() {
        let raw = StableId128::from_bytes([
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ]);
        let id = LaneEdgeId::from_untyped(raw);
        let text = "lfid1_lane-edge_00112233445566778899aabbccddeeff";

        assert_eq!(id.to_string(), text);
        assert_eq!(text.parse::<LaneEdgeId>(), Ok(id));
        assert_eq!(id.into_untyped(), raw);
    }

    #[test]
    fn typed_stable_id_text_rejects_noncanonical_forms() {
        assert_eq!(
            "lane-edge_00112233445566778899aabbccddeeff".parse::<LaneEdgeId>(),
            Err(StableIdTextError::InvalidPrefix)
        );
        assert_eq!(
            "lfid1_junction_00112233445566778899aabbccddeeff".parse::<LaneEdgeId>(),
            Err(StableIdTextError::UnexpectedEntityKind)
        );
        assert_eq!(
            "lfid1_lane-edge_0011".parse::<LaneEdgeId>(),
            Err(StableIdTextError::InvalidHexLength { actual: 4 })
        );
        assert_eq!(
            "lfid1_lane-edge_00112233445566778899AABBCCDDEEFF".parse::<LaneEdgeId>(),
            Err(StableIdTextError::InvalidHexCharacter {
                index: 20,
                byte: b'A'
            })
        );
    }

    #[test]
    fn ordinal_conversions_are_checked() {
        let ordinal = LaneEdgeOrdinal::try_from_usize(42).unwrap();
        assert_eq!(ordinal.raw(), 42);
        assert_eq!(ordinal.index(), 42);
        assert_eq!(u32::from(ordinal), 42);

        if usize::BITS > u32::BITS {
            assert!(LaneEdgeOrdinal::try_from_usize(u32::MAX as usize + 1).is_err());
        }
    }
}
