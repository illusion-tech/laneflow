use super::*;

pub(super) const MAPPING_TABLES: &[PortableTableSchema] = &[
    table(
        1,
        "StationCorridor",
        &PortableRowSchema {
            fields: &[
                field(1, "roadCorridor", PortableFieldType::U32, R),
                field(2, "originSNm", PortableFieldType::U64, R),
                field(3, "lengthNm", PortableFieldType::U64, R),
                field(4, "originErrorNm", PortableFieldType::U32, R),
                field(5, "firstAnchor", PortableFieldType::U32, R),
                field(6, "anchorCount", PortableFieldType::U32, R),
            ],
            shape: PortableRowShape::Uniform,
        },
        ANY,
    ),
    table(
        2,
        "StationAnchor",
        &PortableRowSchema {
            fields: &[
                field(1, "stationNm", PortableFieldType::U64, R),
                field(2, "stationErrorNm", PortableFieldType::U32, R),
            ],
            shape: PortableRowShape::Uniform,
        },
        ANY,
    ),
    table(
        3,
        "StationLane",
        &PortableRowSchema {
            fields: &[
                field(1, "authoringLane", PortableFieldType::U32, R),
                field(2, "direction", PortableFieldType::U8, R),
                field(3, "firstEdge", PortableFieldType::U32, R),
                field(4, "edgeCount", PortableFieldType::U32, R),
            ],
            shape: PortableRowShape::Uniform,
        },
        ANY,
    ),
    table(
        4,
        "StationEdge",
        &PortableRowSchema {
            fields: &[
                field(1, "laneEdge", PortableFieldType::U32, R),
                field(2, "startAnchor", PortableFieldType::U32, R),
                field(3, "firstSpan", PortableFieldType::U32, R),
                field(4, "spanCount", PortableFieldType::U32, R),
            ],
            shape: PortableRowShape::Uniform,
        },
        ANY,
    ),
    table(
        5,
        "StationSpan",
        &PortableRowSchema {
            fields: &[
                field(1, "endAnchor", PortableFieldType::U32, R),
                field(2, "endProgressNm", PortableFieldType::U64, R),
                field(3, "canonicalErrorNm", PortableFieldType::U32, R),
                field(4, "sourceTravelErrorNm", PortableFieldType::U32, R),
                field(5, "minRatePpm", PortableFieldType::U32, R),
            ],
            shape: PortableRowShape::Uniform,
        },
        ANY,
    ),
    table(
        6,
        "StationBand",
        &PortableRowSchema {
            fields: &[
                field(1, "facilityBand", PortableFieldType::U32, R),
                field(2, "startAnchor", PortableFieldType::U32, R),
                field(3, "endAnchor", PortableFieldType::U32, R),
                field(4, "leftStartUm", PortableFieldType::I32, R),
                field(5, "rightStartUm", PortableFieldType::I32, R),
                field(6, "leftEndUm", PortableFieldType::I32, R),
                field(7, "rightEndUm", PortableFieldType::I32, R),
                field(8, "lateralErrorNm", PortableFieldType::U32, R),
            ],
            shape: PortableRowShape::Uniform,
        },
        ANY,
    ),
    table(
        7,
        "BoundaryExtent",
        &PortableRowSchema {
            fields: &[
                field(1, "ownerKind", PortableFieldType::U16, R),
                field(2, "ownerOrdinal", PortableFieldType::U32, R),
                field(3, "leftIndex", PortableFieldType::U32, R),
                field(4, "extentKind", PortableFieldType::U8, R),
                field(5, "localKey", PortableFieldType::Utf8, R),
                field(6, "lowAnchor", PortableFieldType::U32, R),
                field(7, "highAnchor", PortableFieldType::U32, R),
            ],
            shape: PortableRowShape::Uniform,
        },
        ANY,
    ),
    table(
        8,
        "LaneRoadway",
        &PortableRowSchema {
            fields: &[
                field(1, "lane", PortableFieldType::U32, R),
                field(2, "leftStartUm", PortableFieldType::I32, R),
                field(3, "rightStartUm", PortableFieldType::I32, R),
                field(4, "leftEndUm", PortableFieldType::I32, R),
                field(5, "rightEndUm", PortableFieldType::I32, R),
                field(6, "lateralErrorNm", PortableFieldType::U32, R),
                field(7, "supportsWheel", PortableFieldType::U8, R),
            ],
            shape: PortableRowShape::Uniform,
        },
        ANY,
    ),
    table(
        9,
        "SurfaceSpan",
        &PortableRowSchema {
            fields: &[
                field(1, "corridor", PortableFieldType::U32, R),
                field(2, "frame", PortableFieldType::U32, R),
                field(3, "lowAnchor", PortableFieldType::U32, R),
                field(4, "highAnchor", PortableFieldType::U32, R),
                field(5, "startJet", PortableFieldType::Bytes, R),
                field(6, "endJet", PortableFieldType::Bytes, R),
                field(7, "derivativeErrors", PortableFieldType::Bytes, R),
            ],
            shape: PortableRowShape::Uniform,
        },
        ANY,
    ),
    table(
        10,
        "BoundaryRule",
        &PortableRowSchema {
            fields: &[
                field(1, "extent", PortableFieldType::U32, R),
                field(2, "directions", PortableFieldType::U8, R),
                field(3, "effect", PortableFieldType::U8, R),
            ],
            shape: PortableRowShape::Uniform,
        },
        ANY,
    ),
    table(
        11,
        "BoundaryObstacle",
        &PortableRowSchema {
            fields: &[
                field(1, "extent", PortableFieldType::U32, R),
                field(2, "shape", PortableFieldType::U8, R),
                field(3, "parameters", PortableFieldType::Bytes, R),
            ],
            shape: PortableRowShape::Uniform,
        },
        ANY,
    ),
];

pub(super) const CHANGE_TABLES: &[PortableTableSchema] = &[table(
    1,
    "StationChange",
    &PortableRowSchema {
        fields: &[
            field(1, "changeKind", PortableFieldType::U8, V),
            field(2, "memberKind", PortableFieldType::U8, V),
            field(3, "memberKey", PortableFieldType::Bytes, V),
            field(4, "beforeValue", PortableFieldType::Bytes, V),
            field(5, "afterValue", PortableFieldType::Bytes, V),
        ],
        shape: PortableRowShape::DiscriminatedU8 {
            tag: 1,
            variants: &[
                PortableRowVariant {
                    discriminant: 0,
                    required_fields: 0b101110,
                    allowed_fields: 0b101110,
                    at_least_one_field: 0,
                },
                PortableRowVariant {
                    discriminant: 1,
                    required_fields: 0b11110,
                    allowed_fields: 0b11110,
                    at_least_one_field: 0,
                },
                PortableRowVariant {
                    discriminant: 2,
                    required_fields: 0b111110,
                    allowed_fields: 0b111110,
                    at_least_one_field: 0,
                },
            ],
        },
    },
    ANY,
)];
