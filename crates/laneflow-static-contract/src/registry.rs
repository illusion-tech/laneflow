//! Identity v1 的闭合登记表。
//!
//! 本模块只登记稳定代码、字段顺序和字段编码，不负责拼接规范身份字节或计算 BLAKE3。
//! 编译器与独立验证器必须分别消费同一登记元数据并独立实现编码，以避免共享算法成为
//! 共同失效点。枚举的数值、保留空位以及 [`EntityKind::required_tags`] 返回的顺序均是
//! Identity v1 契约的一部分，不能按源码美观需要重新编号或排序。

/// Identity envelope 的固定四字节 ASCII 魔数。
pub const IDENTITY_MAGIC: [u8; 4] = *b"LFID";

/// Identity v1 的规范字节编码版本；改变 envelope 或字段编码规则时必须提升。
pub const IDENTITY_ENCODING_VERSION: u16 = 1;

/// Identity v1 的实体种类 / 字段标签登记表修订；登记项增删或身份字段变化时必须提升。
pub const IDENTITY_REGISTRY_REVISION: u16 = 1;

/// 拼接在规范身份字节之前的 Stable ID BLAKE3 输入域分离前缀。
///
/// 末尾 NUL 是常量自身的一部分，编码器不得按 C 字符串规则截断。
pub const STABLE_ID_DOMAIN_PREFIX: &[u8] = b"laneflow.stable-id.v1\0";

/// Identity v1 中实体的身份类别。
///
/// 类别决定身份来自普通来源声明，还是来自不应依赖可选领域角色的可寻址拓扑实体。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EntityCategory {
    /// 由来源模块声明的实体。
    Declaration,
    /// 不依赖可选领域角色、可独立寻址的拓扑实体。
    AddressableTopologyEntity,
}

/// Identity v1 的实体种类登记表。
///
/// `repr(u16)` 数值进入规范身份 envelope。未知代码必须失败关闭，不能映射为相近种类。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u16)]
pub enum EntityKind {
    RoadCorridor = 1,
    RoadSection = 2,
    AuthoringLane = 3,
    LaneEdge = 4,
    Junction = 5,
    Movement = 6,
    ManeuverPath = 7,
    ManeuverGate = 8,
    WaitingZone = 9,
    StopLine = 10,
    SignalGroup = 11,
    SignalController = 12,
    SignalPhase = 13,
    ParkingArea = 14,
    ParkingSpace = 15,
    LaneGroup = 16,
    FacilityBand = 17,
    ParticipantClass = 18,
    AccessRule = 19,
    VehicleProfile = 20,
    StaticRoute = 21,
    CanonicalFrame = 22,
}

impl EntityKind {
    /// Registry revision 1 中按代码升序排列的全部已登记实体种类。
    pub const ALL: [Self; 22] = [
        Self::RoadCorridor,
        Self::RoadSection,
        Self::AuthoringLane,
        Self::LaneEdge,
        Self::Junction,
        Self::Movement,
        Self::ManeuverPath,
        Self::ManeuverGate,
        Self::WaitingZone,
        Self::StopLine,
        Self::SignalGroup,
        Self::SignalController,
        Self::SignalPhase,
        Self::ParkingArea,
        Self::ParkingSpace,
        Self::LaneGroup,
        Self::FacilityBand,
        Self::ParticipantClass,
        Self::AccessRule,
        Self::VehicleProfile,
        Self::StaticRoute,
        Self::CanonicalFrame,
    ];

    /// 返回写入 Identity v1 envelope 的稳定 `u16` 种类代码。
    #[must_use]
    pub const fn code(self) -> u16 {
        self as u16
    }

    /// 解析已登记种类代码；未知或其他修订才定义的代码返回 `None`。
    #[must_use]
    pub const fn from_code(code: u16) -> Option<Self> {
        match code {
            1 => Some(Self::RoadCorridor),
            2 => Some(Self::RoadSection),
            3 => Some(Self::AuthoringLane),
            4 => Some(Self::LaneEdge),
            5 => Some(Self::Junction),
            6 => Some(Self::Movement),
            7 => Some(Self::ManeuverPath),
            8 => Some(Self::ManeuverGate),
            9 => Some(Self::WaitingZone),
            10 => Some(Self::StopLine),
            11 => Some(Self::SignalGroup),
            12 => Some(Self::SignalController),
            13 => Some(Self::SignalPhase),
            14 => Some(Self::ParkingArea),
            15 => Some(Self::ParkingSpace),
            16 => Some(Self::LaneGroup),
            17 => Some(Self::FacilityBand),
            18 => Some(Self::ParticipantClass),
            19 => Some(Self::AccessRule),
            20 => Some(Self::VehicleProfile),
            21 => Some(Self::StaticRoute),
            22 => Some(Self::CanonicalFrame),
            _ => None,
        }
    }

    /// 返回该实体种类的身份类别。
    #[must_use]
    pub const fn category(self) -> EntityCategory {
        match self {
            Self::LaneEdge => EntityCategory::AddressableTopologyEntity,
            _ => EntityCategory::Declaration,
        }
    }

    /// 返回用于规范文本表示和诊断的稳定 ASCII slug。
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::RoadCorridor => "corridor",
            Self::RoadSection => "section",
            Self::AuthoringLane => "lane",
            Self::LaneEdge => "lane-edge",
            Self::Junction => "junction",
            Self::Movement => "movement",
            Self::ManeuverPath => "path",
            Self::ManeuverGate => "gate",
            Self::WaitingZone => "waiting-zone",
            Self::StopLine => "stop-line",
            Self::SignalGroup => "signal-group",
            Self::SignalController => "signal-controller",
            Self::SignalPhase => "signal-phase",
            Self::ParkingArea => "parking-area",
            Self::ParkingSpace => "parking-space",
            Self::LaneGroup => "lane-group",
            Self::FacilityBand => "facility-band",
            Self::ParticipantClass => "participant-class",
            Self::AccessRule => "access-rule",
            Self::VehicleProfile => "vehicle-profile",
            Self::StaticRoute => "static-route",
            Self::CanonicalFrame => "canonical-frame",
        }
    }

    /// 返回构造该实体 Identity v1 前像时必需的字段标签规范序列。
    ///
    /// 返回顺序就是编码顺序；调用者不得排序、去重，也不得从 source map 补入缺失字段。
    #[must_use]
    pub const fn required_tags(self) -> &'static [FieldTag] {
        match self {
            Self::RoadCorridor => &[FieldTag::AuthoringNamespaceId, FieldTag::CorridorKey],
            Self::RoadSection => &[
                FieldTag::AuthoringNamespaceId,
                FieldTag::SectionKey,
                FieldTag::RoadCorridorStableId,
            ],
            Self::AuthoringLane => &[
                FieldTag::AuthoringNamespaceId,
                FieldTag::LaneKey,
                FieldTag::RoadSectionStableId,
            ],
            Self::LaneEdge => &[FieldTag::AuthoringNamespaceId, FieldTag::LaneEdgeKey],
            Self::Junction => &[FieldTag::AuthoringNamespaceId, FieldTag::JunctionKey],
            Self::Movement => &[
                FieldTag::AuthoringNamespaceId,
                FieldTag::MovementKey,
                FieldTag::DirectedEntryApproachKey,
                FieldTag::DirectedExitApproachKey,
                FieldTag::JunctionStableId,
            ],
            Self::ManeuverPath => &[
                FieldTag::AuthoringNamespaceId,
                FieldTag::PathKey,
                FieldTag::MovementStableId,
                FieldTag::EntryEdgeStableId,
                FieldTag::ExitEdgeStableId,
            ],
            Self::ManeuverGate => &[
                FieldTag::AuthoringNamespaceId,
                FieldTag::ManeuverPathStableId,
                FieldTag::GateKey,
            ],
            Self::WaitingZone => &[
                FieldTag::AuthoringNamespaceId,
                FieldTag::ManeuverPathStableId,
                FieldTag::WaitingZoneKey,
            ],
            Self::StopLine => &[FieldTag::AuthoringNamespaceId, FieldTag::StopLineKey],
            Self::SignalGroup => &[FieldTag::AuthoringNamespaceId, FieldTag::SignalGroupKey],
            Self::SignalController => &[
                FieldTag::AuthoringNamespaceId,
                FieldTag::SignalControllerKey,
            ],
            Self::SignalPhase => &[
                FieldTag::AuthoringNamespaceId,
                FieldTag::SignalControllerStableId,
                FieldTag::PhaseKey,
            ],
            Self::ParkingArea => &[FieldTag::AuthoringNamespaceId, FieldTag::ParkingAreaKey],
            Self::ParkingSpace => &[FieldTag::AuthoringNamespaceId, FieldTag::ParkingSpaceKey],
            Self::LaneGroup => &[
                FieldTag::AuthoringNamespaceId,
                FieldTag::LaneGroupKey,
                FieldTag::RoadSectionStableId,
            ],
            Self::FacilityBand => &[
                FieldTag::AuthoringNamespaceId,
                FieldTag::FacilityBandKey,
                FieldTag::RoadCorridorStableId,
            ],
            Self::ParticipantClass => &[
                FieldTag::AuthoringNamespaceId,
                FieldTag::ParticipantClassKey,
            ],
            Self::AccessRule => &[FieldTag::AuthoringNamespaceId, FieldTag::AccessRuleKey],
            Self::VehicleProfile => &[FieldTag::AuthoringNamespaceId, FieldTag::VehicleProfileKey],
            Self::StaticRoute => &[FieldTag::AuthoringNamespaceId, FieldTag::RouteKey],
            Self::CanonicalFrame => &[FieldTag::AuthoringNamespaceId, FieldTag::CanonicalFrameKey],
        }
    }
}

/// Identity v1 字段值的规范编码类别。
///
/// 该枚举只描述值载荷；字段标签、长度和 envelope 仍由 Identity v1 编码规则约束。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FieldEncoding {
    /// 受 Identity v1 文本规则约束的 ASCII 字节。
    Ascii,
    /// 恰好 16 字节的原始 `StableId128`。
    StableId128,
}

/// Identity v1 的字段标签登记表。代码 23 被保留且不得解码为字段。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u16)]
pub enum FieldTag {
    AuthoringNamespaceId = 1,
    CorridorKey = 2,
    SectionKey = 3,
    LaneKey = 4,
    LaneEdgeKey = 5,
    JunctionKey = 6,
    PathKey = 7,
    MovementKey = 8,
    DirectedEntryApproachKey = 9,
    DirectedExitApproachKey = 10,
    MovementStableId = 11,
    EntryEdgeStableId = 12,
    ExitEdgeStableId = 13,
    ManeuverPathStableId = 14,
    GateKey = 15,
    WaitingZoneKey = 16,
    StopLineKey = 17,
    SignalGroupKey = 18,
    SignalControllerKey = 19,
    SignalControllerStableId = 20,
    PhaseKey = 21,
    ParkingAreaKey = 22,
    ParkingSpaceKey = 24,
    LaneGroupKey = 25,
    FacilityBandKey = 26,
    ParticipantClassKey = 27,
    AccessRuleKey = 28,
    VehicleProfileKey = 29,
    RouteKey = 30,
    CanonicalFrameKey = 31,
    RoadSectionStableId = 32,
    RoadCorridorStableId = 33,
    JunctionStableId = 34,
}

impl FieldTag {
    /// Registry revision 1 中按代码升序排列的全部已登记字段标签。
    ///
    /// 保留代码 23 不在此集合中。
    pub const ALL: [Self; 33] = [
        Self::AuthoringNamespaceId,
        Self::CorridorKey,
        Self::SectionKey,
        Self::LaneKey,
        Self::LaneEdgeKey,
        Self::JunctionKey,
        Self::PathKey,
        Self::MovementKey,
        Self::DirectedEntryApproachKey,
        Self::DirectedExitApproachKey,
        Self::MovementStableId,
        Self::EntryEdgeStableId,
        Self::ExitEdgeStableId,
        Self::ManeuverPathStableId,
        Self::GateKey,
        Self::WaitingZoneKey,
        Self::StopLineKey,
        Self::SignalGroupKey,
        Self::SignalControllerKey,
        Self::SignalControllerStableId,
        Self::PhaseKey,
        Self::ParkingAreaKey,
        Self::ParkingSpaceKey,
        Self::LaneGroupKey,
        Self::FacilityBandKey,
        Self::ParticipantClassKey,
        Self::AccessRuleKey,
        Self::VehicleProfileKey,
        Self::RouteKey,
        Self::CanonicalFrameKey,
        Self::RoadSectionStableId,
        Self::RoadCorridorStableId,
        Self::JunctionStableId,
    ];

    /// 返回写入规范身份字节的稳定 `u16` 字段代码。
    #[must_use]
    pub const fn code(self) -> u16 {
        self as u16
    }

    /// 解析已登记字段代码；未知代码和保留代码 23 返回 `None`。
    #[must_use]
    pub const fn from_code(code: u16) -> Option<Self> {
        match code {
            1 => Some(Self::AuthoringNamespaceId),
            2 => Some(Self::CorridorKey),
            3 => Some(Self::SectionKey),
            4 => Some(Self::LaneKey),
            5 => Some(Self::LaneEdgeKey),
            6 => Some(Self::JunctionKey),
            7 => Some(Self::PathKey),
            8 => Some(Self::MovementKey),
            9 => Some(Self::DirectedEntryApproachKey),
            10 => Some(Self::DirectedExitApproachKey),
            11 => Some(Self::MovementStableId),
            12 => Some(Self::EntryEdgeStableId),
            13 => Some(Self::ExitEdgeStableId),
            14 => Some(Self::ManeuverPathStableId),
            15 => Some(Self::GateKey),
            16 => Some(Self::WaitingZoneKey),
            17 => Some(Self::StopLineKey),
            18 => Some(Self::SignalGroupKey),
            19 => Some(Self::SignalControllerKey),
            20 => Some(Self::SignalControllerStableId),
            21 => Some(Self::PhaseKey),
            22 => Some(Self::ParkingAreaKey),
            24 => Some(Self::ParkingSpaceKey),
            25 => Some(Self::LaneGroupKey),
            26 => Some(Self::FacilityBandKey),
            27 => Some(Self::ParticipantClassKey),
            28 => Some(Self::AccessRuleKey),
            29 => Some(Self::VehicleProfileKey),
            30 => Some(Self::RouteKey),
            31 => Some(Self::CanonicalFrameKey),
            32 => Some(Self::RoadSectionStableId),
            33 => Some(Self::RoadCorridorStableId),
            34 => Some(Self::JunctionStableId),
            _ => None,
        }
    }

    /// 返回登记表中的稳定 lowerCamelCase 字段名，供描述符和诊断使用。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::AuthoringNamespaceId => "authoringNamespaceId",
            Self::CorridorKey => "corridorKey",
            Self::SectionKey => "sectionKey",
            Self::LaneKey => "laneKey",
            Self::LaneEdgeKey => "laneEdgeKey",
            Self::JunctionKey => "junctionKey",
            Self::PathKey => "pathKey",
            Self::MovementKey => "movementKey",
            Self::DirectedEntryApproachKey => "directedEntryApproachKey",
            Self::DirectedExitApproachKey => "directedExitApproachKey",
            Self::MovementStableId => "movementStableId",
            Self::EntryEdgeStableId => "entryEdgeStableId",
            Self::ExitEdgeStableId => "exitEdgeStableId",
            Self::ManeuverPathStableId => "maneuverPathStableId",
            Self::GateKey => "gateKey",
            Self::WaitingZoneKey => "waitingZoneKey",
            Self::StopLineKey => "stopLineKey",
            Self::SignalGroupKey => "signalGroupKey",
            Self::SignalControllerKey => "signalControllerKey",
            Self::SignalControllerStableId => "signalControllerStableId",
            Self::PhaseKey => "phaseKey",
            Self::ParkingAreaKey => "parkingAreaKey",
            Self::ParkingSpaceKey => "parkingSpaceKey",
            Self::LaneGroupKey => "laneGroupKey",
            Self::FacilityBandKey => "facilityBandKey",
            Self::ParticipantClassKey => "participantClassKey",
            Self::AccessRuleKey => "accessRuleKey",
            Self::VehicleProfileKey => "vehicleProfileKey",
            Self::RouteKey => "routeKey",
            Self::CanonicalFrameKey => "canonicalFrameKey",
            Self::RoadSectionStableId => "roadSectionStableId",
            Self::RoadCorridorStableId => "roadCorridorStableId",
            Self::JunctionStableId => "junctionStableId",
        }
    }

    /// 返回该字段值在 Identity v1 中必须采用的编码类别。
    #[must_use]
    pub const fn encoding(self) -> FieldEncoding {
        match self {
            Self::MovementStableId
            | Self::EntryEdgeStableId
            | Self::ExitEdgeStableId
            | Self::ManeuverPathStableId
            | Self::SignalControllerStableId
            | Self::RoadSectionStableId
            | Self::RoadCorridorStableId
            | Self::JunctionStableId => FieldEncoding::StableId128,
            _ => FieldEncoding::Ascii,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_v1_constants_match_contract() {
        assert_eq!(IDENTITY_MAGIC, *b"LFID");
        assert_eq!(IDENTITY_ENCODING_VERSION, 1);
        assert_eq!(IDENTITY_REGISTRY_REVISION, 1);
        assert_eq!(STABLE_ID_DOMAIN_PREFIX, b"laneflow.stable-id.v1\0");
    }

    #[test]
    fn entity_registry_matches_identity_v1_contract() {
        let expected = [
            (EntityKind::RoadCorridor, "corridor", &[1, 2][..]),
            (EntityKind::RoadSection, "section", &[1, 3, 33][..]),
            (EntityKind::AuthoringLane, "lane", &[1, 4, 32][..]),
            (EntityKind::LaneEdge, "lane-edge", &[1, 5][..]),
            (EntityKind::Junction, "junction", &[1, 6][..]),
            (EntityKind::Movement, "movement", &[1, 8, 9, 10, 34][..]),
            (EntityKind::ManeuverPath, "path", &[1, 7, 11, 12, 13][..]),
            (EntityKind::ManeuverGate, "gate", &[1, 14, 15][..]),
            (EntityKind::WaitingZone, "waiting-zone", &[1, 14, 16][..]),
            (EntityKind::StopLine, "stop-line", &[1, 17][..]),
            (EntityKind::SignalGroup, "signal-group", &[1, 18][..]),
            (
                EntityKind::SignalController,
                "signal-controller",
                &[1, 19][..],
            ),
            (EntityKind::SignalPhase, "signal-phase", &[1, 20, 21][..]),
            (EntityKind::ParkingArea, "parking-area", &[1, 22][..]),
            (EntityKind::ParkingSpace, "parking-space", &[1, 24][..]),
            (EntityKind::LaneGroup, "lane-group", &[1, 25, 32][..]),
            (EntityKind::FacilityBand, "facility-band", &[1, 26, 33][..]),
            (
                EntityKind::ParticipantClass,
                "participant-class",
                &[1, 27][..],
            ),
            (EntityKind::AccessRule, "access-rule", &[1, 28][..]),
            (EntityKind::VehicleProfile, "vehicle-profile", &[1, 29][..]),
            (EntityKind::StaticRoute, "static-route", &[1, 30][..]),
            (EntityKind::CanonicalFrame, "canonical-frame", &[1, 31][..]),
        ];

        assert_eq!(EntityKind::ALL.len(), expected.len());
        for (index, (kind, slug, required_tag_codes)) in expected.into_iter().enumerate() {
            assert_eq!(EntityKind::ALL[index], kind);
            assert_eq!(kind.code(), u16::try_from(index + 1).unwrap());
            assert_eq!(EntityKind::from_code(kind.code()), Some(kind));
            assert_eq!(kind.slug(), slug);
            assert!(kind.slug().bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
            }));
            assert_eq!(kind.required_tags().len(), required_tag_codes.len());
            for (tag, expected_code) in kind.required_tags().iter().zip(required_tag_codes) {
                assert_eq!(tag.code(), *expected_code);
            }

            for other in EntityKind::ALL.iter().copied().skip(index + 1) {
                assert_ne!(kind.code(), other.code());
                assert_ne!(kind.slug(), other.slug());
            }
        }
        assert_eq!(EntityKind::from_code(0), None);
        assert_eq!(EntityKind::from_code(23), None);
        assert_eq!(
            EntityKind::LaneEdge.category(),
            EntityCategory::AddressableTopologyEntity
        );
        assert!(
            EntityKind::ALL
                .into_iter()
                .filter(|kind| *kind != EntityKind::LaneEdge)
                .all(|kind| kind.category() == EntityCategory::Declaration)
        );
    }

    #[test]
    fn required_tags_are_registered_and_strictly_increasing() {
        for kind in EntityKind::ALL {
            let tags = kind.required_tags();
            assert!(!tags.is_empty(), "{}", kind.slug());
            for tag in tags {
                assert_eq!(FieldTag::from_code(tag.code()), Some(*tag));
            }
            assert!(
                tags.windows(2).all(|pair| pair[0].code() < pair[1].code()),
                "{}",
                kind.slug()
            );
        }
    }

    #[test]
    fn field_registry_matches_identity_v1_contract() {
        let expected = [
            (1, "authoringNamespaceId", FieldEncoding::Ascii),
            (2, "corridorKey", FieldEncoding::Ascii),
            (3, "sectionKey", FieldEncoding::Ascii),
            (4, "laneKey", FieldEncoding::Ascii),
            (5, "laneEdgeKey", FieldEncoding::Ascii),
            (6, "junctionKey", FieldEncoding::Ascii),
            (7, "pathKey", FieldEncoding::Ascii),
            (8, "movementKey", FieldEncoding::Ascii),
            (9, "directedEntryApproachKey", FieldEncoding::Ascii),
            (10, "directedExitApproachKey", FieldEncoding::Ascii),
            (11, "movementStableId", FieldEncoding::StableId128),
            (12, "entryEdgeStableId", FieldEncoding::StableId128),
            (13, "exitEdgeStableId", FieldEncoding::StableId128),
            (14, "maneuverPathStableId", FieldEncoding::StableId128),
            (15, "gateKey", FieldEncoding::Ascii),
            (16, "waitingZoneKey", FieldEncoding::Ascii),
            (17, "stopLineKey", FieldEncoding::Ascii),
            (18, "signalGroupKey", FieldEncoding::Ascii),
            (19, "signalControllerKey", FieldEncoding::Ascii),
            (20, "signalControllerStableId", FieldEncoding::StableId128),
            (21, "phaseKey", FieldEncoding::Ascii),
            (22, "parkingAreaKey", FieldEncoding::Ascii),
            (24, "parkingSpaceKey", FieldEncoding::Ascii),
            (25, "laneGroupKey", FieldEncoding::Ascii),
            (26, "facilityBandKey", FieldEncoding::Ascii),
            (27, "participantClassKey", FieldEncoding::Ascii),
            (28, "accessRuleKey", FieldEncoding::Ascii),
            (29, "vehicleProfileKey", FieldEncoding::Ascii),
            (30, "routeKey", FieldEncoding::Ascii),
            (31, "canonicalFrameKey", FieldEncoding::Ascii),
            (32, "roadSectionStableId", FieldEncoding::StableId128),
            (33, "roadCorridorStableId", FieldEncoding::StableId128),
            (34, "junctionStableId", FieldEncoding::StableId128),
        ];

        assert_eq!(FieldTag::ALL.len(), expected.len());
        assert_eq!(FieldTag::from_code(23), None);

        for (tag, (code, name, encoding)) in FieldTag::ALL.into_iter().zip(expected) {
            assert_eq!(tag.code(), code);
            assert_eq!(FieldTag::from_code(tag.code()), Some(tag));
            assert_eq!(tag.name(), name);
            assert_eq!(tag.encoding(), encoding);
        }
    }
}
