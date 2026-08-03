use core::{fmt, marker::PhantomData, str::FromStr};

use crate::EntityKind;

/// 不携带实体种类的 128 位稳定标识值。
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct StableId128([u8; 16]);

impl StableId128 {
    pub const ZERO: Self = Self([0; 16]);

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

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
    pub trait EntityKindMarker {}
    pub trait OrdinalKind {}
}

/// 由本 crate 封闭的 Identity v1 实体种类标记。
pub trait EntityKindMarker: sealed::EntityKindMarker + 'static {
    const KIND: EntityKind;
}

/// 允许作为致密表序号类型参数的封闭标记。
pub trait OrdinalKind: sealed::OrdinalKind + 'static {}

/// 按实体种类区分的 128 位稳定标识。
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct StableId<K: EntityKindMarker> {
    raw: StableId128,
    marker: PhantomData<fn() -> K>,
}

impl<K: EntityKindMarker> StableId<K> {
    #[must_use]
    pub const fn from_untyped(raw: StableId128) -> Self {
        Self {
            raw,
            marker: PhantomData,
        }
    }

    #[must_use]
    pub const fn as_untyped(&self) -> &StableId128 {
        &self.raw
    }

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
        for (index, pair) in hexadecimal.as_bytes().chunks_exact(2).enumerate() {
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StableIdTextError {
    InvalidPrefix,
    UnexpectedEntityKind,
    InvalidHexLength { actual: usize },
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
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct Ordinal<K: OrdinalKind> {
    raw: u32,
    marker: PhantomData<fn() -> K>,
}

impl<K: OrdinalKind> Ordinal<K> {
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Self {
            raw,
            marker: PhantomData,
        }
    }

    #[must_use]
    pub const fn raw(self) -> u32 {
        self.raw
    }

    #[must_use]
    pub fn index(self) -> usize {
        usize::try_from(self.raw).expect("u32 ordinal must fit usize on supported targets")
    }

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
            #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
            pub struct $marker;

            impl sealed::EntityKindMarker for $marker {}
            impl sealed::OrdinalKind for $marker {}

            impl EntityKindMarker for $marker {
                const KIND: EntityKind = EntityKind::$kind;
            }

            impl OrdinalKind for $marker {}

            pub type $id = StableId<$marker>;
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
        ParkingAreaKind,
        ParkingArea,
        ParkingAreaId,
        ParkingAreaOrdinal
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
        StaticRouteKind,
        StaticRoute,
        StaticRouteId,
        StaticRouteOrdinal
    ),
    (
        CanonicalFrameKind,
        CanonicalFrame,
        CanonicalFrameId,
        CanonicalFrameOrdinal
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
