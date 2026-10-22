//! 共享路网修订认证的修订比较阶段：`origin` 逐字段与期望值比对、候选修订与被认证修订
//! 逐 component / 表按值比对（完整性设计 §4.2；比较算子冻结为整数 / 句柄 / ID /
//! 范围按值相等，`f32` 按 `to_bits()` 位相等，不使用浮点 `PartialEq`）。
//!
//! 比对只读两侧对象、逐列早停于首个不等位置，不按布局或地址取字节；
//! `origin` 的字节身份字段（LFCA digest / exact length）不在比对面内
//! （设计 §2.2「发布来源的字节身份」）。

use std::fmt::Debug;

use crate::SharedNetworkRevision;
use crate::attestation::{
    AttestationStage, MismatchEvidence, MismatchSite, OriginField, RootAttestationExpectation,
    RootComponent,
};
use crate::origin::CanonicalNetworkOrigin;
use crate::relations::SharedRelationClosure;
use crate::spatial::{LanePoseNetwork, SharedSpatialNetwork};
use crate::traffic::{SharedManeuverNetwork, SharedTrafficNetwork};

/// 二进制常量组装的期望版本轴：reader / builder 只接受这一组精确版本
/// （完整性设计 §2.2，任何期望值不得取自被认证修订）。
pub(crate) fn expected_static_contract_versions() -> crate::origin::StaticContractVersions {
    use laneflow_static_contract::{
        CANONICAL_ARTIFACT_FORMAT_VERSION, CONSTRAINT_CONTRACT_VERSION, IDENTITY_ENCODING_VERSION,
        IDENTITY_REGISTRY_REVISION, NETWORK_REVISION_DERIVATION_VERSION,
        STATIC_EXECUTION_CONTRACT_VERSION,
    };
    crate::origin::StaticContractVersions::new(
        CANONICAL_ARTIFACT_FORMAT_VERSION,
        IDENTITY_ENCODING_VERSION,
        IDENTITY_REGISTRY_REVISION,
        NETWORK_REVISION_DERIVATION_VERSION,
        CONSTRAINT_CONTRACT_VERSION,
        STATIC_EXECUTION_CONTRACT_VERSION,
    )
}

/// 首个不等字段的报告三元组：（字段名，期望值文本，实际值文本）。
type FieldDiff = Option<(&'static str, String, String)>;

fn evidence(site: MismatchSite) -> MismatchEvidence {
    MismatchEvidence {
        stage: AttestationStage::RootComparison,
        site,
    }
}

fn u32_len(values: &[impl Sized]) -> u32 {
    u32::try_from(values.len()).expect("format-bounded table length fits u32")
}

/// `f32` 的报告文本：值与位模式并列，覆盖 `-0.0` 与 NaN 位模式注入。
fn f32_text(value: f32) -> String {
    format!("{value:?} (bits 0x{:08X})", value.to_bits())
}

/// 向量浮点证据：取首个位不等的分量并报告其值与位模式（不同 NaN payload
/// 因此产生不同证据文本，复算判等不会把不同失配误认成同一证据）。
/// 分量级字段名：证据必须携带完整逻辑位置（分量轴），同值失配移到
/// 另一分量是不同证据（复算归类据此判 Untrusted 而非确认 Mismatch）。
const TANGENT_FIELDS: [&str; 3] = ["tangent[0]", "tangent[1]", "tangent[2]"];
const UP_FIELDS: [&str; 3] = ["up[0]", "up[1]", "up[2]"];

fn vec3_text(
    fields: [&'static str; 3],
    expected: &[f32; 3],
    actual: &[f32; 3],
) -> (&'static str, String, String) {
    let axis = expected
        .iter()
        .zip(actual)
        .position(|(expected, actual)| expected.to_bits() != actual.to_bits())
        .unwrap_or(0);
    (
        fields[axis],
        f32_text(expected[axis]),
        f32_text(actual[axis]),
    )
}

fn f32_whole(expected: &f32, actual: &f32) -> FieldDiff {
    (expected.to_bits() != actual.to_bits())
        .then(|| ("value", f32_text(*expected), f32_text(*actual)))
}

/// 元素整体按值比较（整数、句柄 ordinal、ID、范围、枚举与 `Option` 载荷；
/// 这些类型不含浮点，`PartialEq` 与按值相等一致）。
fn whole<T: PartialEq + Debug>(expected: &T, actual: &T) -> FieldDiff {
    (expected != actual).then(|| ("value", format!("{expected:?}"), format!("{actual:?}")))
}

/// 具名字段按值比较（复合记录的成员；报告字段身份而非统一的 "value"）。
fn named<T: PartialEq + Debug>(field: &'static str, expected: &T, actual: &T) -> FieldDiff {
    (expected != actual).then(|| (field, format!("{expected:?}"), format!("{actual:?}")))
}

/// 文本列比较（intern token 与策略冷数据字符串，按字节相等）。
fn text_whole(expected: &str, actual: &str) -> FieldDiff {
    (expected != actual).then(|| ("value", expected.to_string(), actual.to_string()))
}

/// 首个失配早停的逐元素列比较：长度失配先报，元素失配取首个不等字段。
fn diff_column<T>(
    component: RootComponent,
    table: &'static str,
    expected: &[T],
    actual: &[T],
    first_field: impl Fn(&T, &T) -> FieldDiff,
) -> Result<(), MismatchEvidence> {
    if expected.len() != actual.len() {
        return Err(evidence(MismatchSite::TableLength {
            component,
            table,
            entity_kind: None,
            expected_len: u32_len(expected),
            actual_len: u32_len(actual),
        }));
    }
    for (index, (expected_item, actual_item)) in expected.iter().zip(actual).enumerate() {
        if let Some((field, expected_text, actual_text)) = first_field(expected_item, actual_item) {
            return Err(evidence(MismatchSite::TableValue {
                component,
                table,
                entity_kind: None,
                index: u32::try_from(index).expect("column index fits u32"),
                field,
                expected: expected_text,
                actual: actual_text,
            }));
        }
    }
    Ok(())
}

/// 声明式列比较：表名取字段名。
macro_rules! cmp_column {
    ($component:expr, $expected:expr, $actual:expr, $field:ident, $cmp:expr) => {
        diff_column(
            $component,
            stringify!($field),
            &$expected.$field,
            &$actual.$field,
            $cmp,
        )?
    };
}

/// 声明式 `OptionalColumn` 比较：表名取字段名。
macro_rules! cmp_optional {
    ($component:expr, $expected:expr, $actual:expr, $field:ident) => {
        diff_column(
            $component,
            stringify!($field),
            &$expected.$field.values,
            &$actual.$field.values,
            option_column,
        )?
    };
}

fn cmp_maneuver_candidate(
    expected: &crate::traffic::ManeuverTransitionCandidate,
    actual: &crate::traffic::ManeuverTransitionCandidate,
) -> FieldDiff {
    if expected.successor != actual.successor {
        return named("successor", &expected.successor, &actual.successor);
    }
    if expected.maneuver_path != actual.maneuver_path {
        return named(
            "maneuver_path",
            &expected.maneuver_path,
            &actual.maneuver_path,
        );
    }
    if expected.transition_index != actual.transition_index {
        return Some((
            "transition_index",
            expected.transition_index.to_string(),
            actual.transition_index.to_string(),
        ));
    }
    named(
        "maneuver_gate",
        &expected.maneuver_gate,
        &actual.maneuver_gate,
    )
}

fn cmp_range(expected: &crate::traffic::RangeU32, actual: &crate::traffic::RangeU32) -> FieldDiff {
    if expected.start != actual.start {
        return Some((
            "start",
            expected.start.to_string(),
            actual.start.to_string(),
        ));
    }
    if expected.len != actual.len {
        return Some(("len", expected.len.to_string(), actual.len.to_string()));
    }
    None
}

fn cmp_identity_reverse_entry(
    expected: &crate::identity::IdentityReverseEntry,
    actual: &crate::identity::IdentityReverseEntry,
) -> FieldDiff {
    if expected.entity_kind != actual.entity_kind {
        return named("entity_kind", &expected.entity_kind, &actual.entity_kind);
    }
    if expected.stable_id != actual.stable_id {
        return named("stable_id", &expected.stable_id, &actual.stable_id);
    }
    if expected.ordinal != actual.ordinal {
        return Some((
            "ordinal",
            expected.ordinal.to_string(),
            actual.ordinal.to_string(),
        ));
    }
    None
}

fn cmp_conflict_passage(
    expected: &crate::conflict::ConflictPassage,
    actual: &crate::conflict::ConflictPassage,
) -> FieldDiff {
    if expected.conflict_zone != actual.conflict_zone {
        return named(
            "conflict_zone",
            &expected.conflict_zone,
            &actual.conflict_zone,
        );
    }
    if expected.entry != actual.entry {
        return Some((
            "entry",
            format!("{:?}", expected.entry),
            format!("{:?}", actual.entry),
        ));
    }
    if expected.exit != actual.exit {
        return Some((
            "exit",
            format!("{:?}", expected.exit),
            format!("{:?}", actual.exit),
        ));
    }
    named(
        "admission_gate",
        &expected.admission_gate,
        &actual.admission_gate,
    )
}

fn cmp_policy_record(
    expected: &crate::policy::PolicyRecord,
    actual: &crate::policy::PolicyRecord,
) -> FieldDiff {
    if expected.id != actual.id {
        return named("id", &expected.id, &actual.id);
    }
    if expected.jurisdiction != actual.jurisdiction {
        return Some((
            "jurisdiction",
            expected.jurisdiction.to_string(),
            actual.jurisdiction.to_string(),
        ));
    }
    if expected.version != actual.version {
        return Some((
            "version",
            expected.version.to_string(),
            actual.version.to_string(),
        ));
    }
    if expected.source != actual.source {
        return Some((
            "source",
            format!("{:?}", expected.source),
            format!("{:?}", actual.source),
        ));
    }
    if expected.gates != actual.gates {
        return Some((
            "gates",
            format!("{:?}", expected.gates),
            format!("{:?}", actual.gates),
        ));
    }
    if expected.streams != actual.streams {
        return Some((
            "streams",
            format!("{:?}", expected.streams),
            format!("{:?}", actual.streams),
        ));
    }
    if expected.gaps != actual.gaps {
        return Some((
            "gaps",
            format!("{:?}", expected.gaps),
            format!("{:?}", actual.gaps),
        ));
    }
    None
}

fn cmp_rule_record(
    expected: &crate::policy::RuleRecord,
    actual: &crate::policy::RuleRecord,
) -> FieldDiff {
    if expected.policy != actual.policy {
        return named("policy", &expected.policy, &actual.policy);
    }
    if expected.kind != actual.kind {
        return named("kind", &expected.kind, &actual.kind);
    }
    if expected.key != actual.key {
        return Some(("key", expected.key.to_string(), actual.key.to_string()));
    }
    if expected.evidence != actual.evidence {
        return Some((
            "evidence",
            format!("{:?}", expected.evidence),
            format!("{:?}", actual.evidence),
        ));
    }
    None
}

fn cmp_policy_evidence(
    expected: &crate::policy::PolicyEvidence,
    actual: &crate::policy::PolicyEvidence,
) -> FieldDiff {
    if expected.key != actual.key {
        return Some(("key", expected.key.to_string(), actual.key.to_string()));
    }
    if expected.locator != actual.locator {
        return Some((
            "locator",
            expected.locator.to_string(),
            actual.locator.to_string(),
        ));
    }
    if expected.description != actual.description {
        return Some((
            "description",
            format!("{:?}", expected.description),
            format!("{:?}", actual.description),
        ));
    }
    None
}

fn cmp_gap_profile(
    expected: &crate::policy::PolicyGapProfile,
    actual: &crate::policy::PolicyGapProfile,
) -> FieldDiff {
    if expected.key != actual.key {
        return Some(("key", expected.key.to_string(), actual.key.to_string()));
    }
    if expected.parameter_version != actual.parameter_version {
        return Some((
            "parameter_version",
            expected.parameter_version.to_string(),
            actual.parameter_version.to_string(),
        ));
    }
    if expected.minimum_lead_ms != actual.minimum_lead_ms {
        return Some((
            "minimum_lead_ms",
            expected.minimum_lead_ms.to_string(),
            actual.minimum_lead_ms.to_string(),
        ));
    }
    if expected.minimum_lag_ms != actual.minimum_lag_ms {
        return Some((
            "minimum_lag_ms",
            expected.minimum_lag_ms.to_string(),
            actual.minimum_lag_ms.to_string(),
        ));
    }
    if expected.clearance_ms != actual.clearance_ms {
        return Some((
            "clearance_ms",
            expected.clearance_ms.to_string(),
            actual.clearance_ms.to_string(),
        ));
    }
    None
}

fn cmp_owner_index(
    component: RootComponent,
    table: &'static str,
    expected: &crate::policy::PolicyOwnerIndex,
    actual: &crate::policy::PolicyOwnerIndex,
) -> Result<(), MismatchEvidence> {
    use crate::policy::PolicyOwnerIndex;
    match (expected, actual) {
        (PolicyOwnerIndex::Dense(expected), PolicyOwnerIndex::Dense(actual)) => {
            diff_column(component, table, expected, actual, whole)
        }
        (PolicyOwnerIndex::Sparse(expected), PolicyOwnerIndex::Sparse(actual)) => {
            diff_column(component, table, expected, actual, cmp_policy_owner)
        }
        (expected, actual) => Err(evidence(MismatchSite::TableValue {
            component,
            table,
            entity_kind: None,
            index: 0,
            field: "variant",
            expected: owner_index_kind(expected).to_string(),
            actual: owner_index_kind(actual).to_string(),
        })),
    }
}

/// `PolicyOwnerIndex` 未实现 `Debug`：以变体名作报告文本。
fn owner_index_kind(index: &crate::policy::PolicyOwnerIndex) -> &'static str {
    match index {
        crate::policy::PolicyOwnerIndex::Dense(_) => "Dense",
        crate::policy::PolicyOwnerIndex::Sparse(_) => "Sparse",
    }
}

fn cmp_policy_owner(
    expected: &crate::policy::PolicyOwner,
    actual: &crate::policy::PolicyOwner,
) -> FieldDiff {
    if expected.owner != actual.owner {
        return Some((
            "owner",
            expected.owner.to_string(),
            actual.owner.to_string(),
        ));
    }
    if expected.cells != actual.cells {
        return Some((
            "cells",
            format!("{:?}", expected.cells),
            format!("{:?}", actual.cells),
        ));
    }
    None
}

fn cmp_resolved_gate_policy(
    expected: &crate::policy::ResolvedGatePolicy,
    actual: &crate::policy::ResolvedGatePolicy,
) -> FieldDiff {
    if expected.class != actual.class {
        return named("class", &expected.class, &actual.class);
    }
    if expected.rule != actual.rule {
        return Some(("rule", expected.rule.to_string(), actual.rule.to_string()));
    }
    if expected.interpretation != actual.interpretation {
        return named(
            "interpretation",
            &expected.interpretation,
            &actual.interpretation,
        );
    }
    named("prohibition", &expected.prohibition, &actual.prohibition)
}

fn cmp_yield_target_cell(
    expected: &crate::policy::YieldTargetCell,
    actual: &crate::policy::YieldTargetCell,
) -> FieldDiff {
    if expected.stream != actual.stream {
        return named("stream", &expected.stream, &actual.stream);
    }
    if expected.passage_local_index != actual.passage_local_index {
        return Some((
            "passage_local_index",
            expected.passage_local_index.to_string(),
            actual.passage_local_index.to_string(),
        ));
    }
    None
}

fn cmp_resolved_stream_policy(
    expected: &crate::policy::ResolvedStreamPolicy,
    actual: &crate::policy::ResolvedStreamPolicy,
) -> FieldDiff {
    if expected.class != actual.class {
        return named("class", &expected.class, &actual.class);
    }
    if expected.rule != actual.rule {
        return Some(("rule", expected.rule.to_string(), actual.rule.to_string()));
    }
    if expected.priority != actual.priority {
        return Some((
            "priority",
            expected.priority.to_string(),
            actual.priority.to_string(),
        ));
    }
    if expected.gap != actual.gap {
        return Some((
            "gap",
            format!("{:?}", expected.gap),
            format!("{:?}", actual.gap),
        ));
    }
    if expected.target_ranges != actual.target_ranges {
        return Some((
            "target_ranges",
            format!("{:?}", expected.target_ranges),
            format!("{:?}", actual.target_ranges),
        ));
    }
    None
}

fn cmp_target_range(
    expected: &crate::policy::TargetRange,
    actual: &crate::policy::TargetRange,
) -> FieldDiff {
    if expected.zone != actual.zone {
        return named("zone", &expected.zone, &actual.zone);
    }
    if expected.targets != actual.targets {
        return Some((
            "targets",
            format!("{:?}", expected.targets),
            format!("{:?}", actual.targets),
        ));
    }
    None
}

fn cmp_parking_anchor(
    expected: &crate::relations::ParkingLaneAnchor,
    actual: &crate::relations::ParkingLaneAnchor,
) -> FieldDiff {
    if expected.lane_edge != actual.lane_edge {
        return named("lane_edge", &expected.lane_edge, &actual.lane_edge);
    }
    if expected.progress_mm != actual.progress_mm {
        return Some((
            "progress_mm",
            expected.progress_mm.to_string(),
            actual.progress_mm.to_string(),
        ));
    }
    None
}

fn cmp_facility_entry(
    expected: &crate::spatial::FacilityGeometryEntry,
    actual: &crate::spatial::FacilityGeometryEntry,
) -> FieldDiff {
    if expected.facility_band != actual.facility_band {
        return named(
            "facility_band",
            &expected.facility_band,
            &actual.facility_band,
        );
    }
    if expected.canonical_frame != actual.canonical_frame {
        return named(
            "canonical_frame",
            &expected.canonical_frame,
            &actual.canonical_frame,
        );
    }
    if expected.point_range != actual.point_range {
        return Some((
            "point_range",
            format!("{:?}", expected.point_range),
            format!("{:?}", actual.point_range),
        ));
    }
    None
}

fn cmp_conflict_region_entry(
    expected: &crate::spatial::ConflictZoneRegionEntry,
    actual: &crate::spatial::ConflictZoneRegionEntry,
) -> FieldDiff {
    if expected.conflict_zone != actual.conflict_zone {
        return named(
            "conflict_zone",
            &expected.conflict_zone,
            &actual.conflict_zone,
        );
    }
    if expected.canonical_frame != actual.canonical_frame {
        return named(
            "canonical_frame",
            &expected.canonical_frame,
            &actual.canonical_frame,
        );
    }
    if expected.min_y.to_bits() != actual.min_y.to_bits() {
        return Some(("min_y", f32_text(expected.min_y), f32_text(actual.min_y)));
    }
    if expected.max_y.to_bits() != actual.max_y.to_bits() {
        return Some(("max_y", f32_text(expected.max_y), f32_text(actual.max_y)));
    }
    if expected.ring_range != actual.ring_range {
        return Some((
            "ring_range",
            format!("{:?}", expected.ring_range),
            format!("{:?}", actual.ring_range),
        ));
    }
    None
}

fn cmp_point(
    expected: &crate::spatial::CanonicalPoint,
    actual: &crate::spatial::CanonicalPoint,
) -> FieldDiff {
    if expected.x.to_bits() != actual.x.to_bits() {
        return Some(("x", f32_text(expected.x), f32_text(actual.x)));
    }
    if expected.y.to_bits() != actual.y.to_bits() {
        return Some(("y", f32_text(expected.y), f32_text(actual.y)));
    }
    (expected.z.to_bits() != actual.z.to_bits())
        .then(|| ("z", f32_text(expected.z), f32_text(actual.z)))
}

fn cmp_point_xz(
    expected: &crate::spatial::CanonicalPointXZ,
    actual: &crate::spatial::CanonicalPointXZ,
) -> FieldDiff {
    if expected.x.to_bits() != actual.x.to_bits() {
        return Some(("x", f32_text(expected.x), f32_text(actual.x)));
    }
    (expected.z.to_bits() != actual.z.to_bits())
        .then(|| ("z", f32_text(expected.z), f32_text(actual.z)))
}

fn cmp_segment(
    expected: &crate::spatial::SegmentGeometry,
    actual: &crate::spatial::SegmentGeometry,
) -> FieldDiff {
    if expected.length_meters.to_bits() != actual.length_meters.to_bits() {
        return Some((
            "length_meters",
            f32_text(expected.length_meters),
            f32_text(actual.length_meters),
        ));
    }
    if expected.cumulative_end_meters.to_bits() != actual.cumulative_end_meters.to_bits() {
        return Some((
            "cumulative_end_meters",
            f32_text(expected.cumulative_end_meters),
            f32_text(actual.cumulative_end_meters),
        ));
    }
    if expected.tangent.map(f32::to_bits) != actual.tangent.map(f32::to_bits) {
        let (field, expected_text, actual_text) =
            vec3_text(TANGENT_FIELDS, &expected.tangent, &actual.tangent);
        return Some((field, expected_text, actual_text));
    }
    if expected.up.map(f32::to_bits) != actual.up.map(f32::to_bits) {
        let (field, expected_text, actual_text) = vec3_text(UP_FIELDS, &expected.up, &actual.up);
        return Some((field, expected_text, actual_text));
    }
    None
}

fn f32_bits_whole(expected: &f32, actual: &f32) -> FieldDiff {
    f32_whole(expected, actual)
}

/// `origin` 与期望值逐字段比对（字节身份字段除外，设计 §2.2 / §4.2）。
pub(crate) fn compare_origin(
    origin: &CanonicalNetworkOrigin,
    expectation: &RootAttestationExpectation,
) -> Result<(), MismatchEvidence> {
    let field = |name, expected: String, actual: String| {
        Err(evidence(MismatchSite::OriginField {
            field: name,
            expected,
            actual,
        }))
    };
    if origin.network_revision() != expectation.network_revision() {
        let f = OriginField::NetworkRevision;
        return field(
            f,
            format!("{:?}", expectation.network_revision()),
            format!("{:?}", origin.network_revision()),
        );
    }
    let expected_contracts = expected_static_contract_versions();
    let origin_field = |name: OriginField, expected: u16, actual: u16| {
        field(name, expected.to_string(), actual.to_string())
    };
    let contracts = origin.static_contract_versions();
    if contracts.canonical_format_version() != expected_contracts.canonical_format_version() {
        return origin_field(
            OriginField::CanonicalFormatVersion,
            expected_contracts.canonical_format_version(),
            contracts.canonical_format_version(),
        );
    }
    if contracts.identity_encoding_version() != expected_contracts.identity_encoding_version() {
        return origin_field(
            OriginField::IdentityEncodingVersion,
            expected_contracts.identity_encoding_version(),
            contracts.identity_encoding_version(),
        );
    }
    if contracts.identity_registry_revision() != expected_contracts.identity_registry_revision() {
        return origin_field(
            OriginField::IdentityRegistryRevision,
            expected_contracts.identity_registry_revision(),
            contracts.identity_registry_revision(),
        );
    }
    if contracts.network_revision_derivation_version()
        != expected_contracts.network_revision_derivation_version()
    {
        return origin_field(
            OriginField::NetworkRevisionDerivationVersion,
            expected_contracts.network_revision_derivation_version(),
            contracts.network_revision_derivation_version(),
        );
    }
    if contracts.constraint_contract_version() != expected_contracts.constraint_contract_version() {
        return origin_field(
            OriginField::ConstraintContractVersion,
            expected_contracts.constraint_contract_version(),
            contracts.constraint_contract_version(),
        );
    }
    if contracts.static_execution_contract_version()
        != expected_contracts.static_execution_contract_version()
    {
        return origin_field(
            OriginField::StaticExecutionContractVersion,
            expected_contracts.static_execution_contract_version(),
            contracts.static_execution_contract_version(),
        );
    }
    if origin.partition_planning_hints_derivation_version()
        != crate::origin::PARTITION_PLANNING_HINTS_DERIVATION_VERSION
    {
        return origin_field(
            OriginField::PartitionPlanningHintsDerivationVersion,
            crate::origin::PARTITION_PLANNING_HINTS_DERIVATION_VERSION,
            origin.partition_planning_hints_derivation_version(),
        );
    }
    Ok(())
}

/// 候选修订（期望侧）与被认证修订（实际侧）逐 component / 表按值比对。
pub(crate) fn compare_revisions_by_value(
    candidate: &SharedNetworkRevision,
    root: &SharedNetworkRevision,
) -> Result<(), MismatchEvidence> {
    compare_traffic(candidate.traffic(), root.traffic())?;
    compare_identity(candidate.identity(), root.identity())?;
    compare_planning_hints(candidate.planning_hints(), root.planning_hints())?;
    compare_conflict(candidate.conflict(), root.conflict())?;
    compare_policy(candidate.policy(), root.policy())?;
    compare_spatial(candidate.spatial(), root.spatial())
}

fn compare_traffic(
    expected: &SharedTrafficNetwork,
    actual: &SharedTrafficNetwork,
) -> Result<(), MismatchEvidence> {
    use laneflow_static_contract::EntityKind;
    let component = RootComponent::Traffic;
    for (index, kind) in EntityKind::ALL.iter().enumerate() {
        let expected_count = expected.entity_counts.counts[index];
        let actual_count = actual.entity_counts.counts[index];
        if expected_count != actual_count {
            return Err(evidence(MismatchSite::TableValue {
                component,
                table: "entity_counts",
                entity_kind: Some(*kind),
                index: u32::try_from(index).expect("entity kind index fits u32"),
                field: "count",
                expected: format!("{kind:?}={expected_count}"),
                actual: format!("{kind:?}={actual_count}"),
            }));
        }
    }
    diff_column(
        component,
        "lane_lengths_millimetres",
        expected.lane_lengths_millimetres(),
        actual.lane_lengths_millimetres(),
        whole,
    )?;
    diff_column(
        component,
        "lane_speed_limits_millimetres_per_second",
        expected.lane_speed_limits_millimetres_per_second(),
        actual.lane_speed_limits_millimetres_per_second(),
        whole,
    )?;
    cmp_column!(component, expected, actual, successor_ranges, cmp_range);
    cmp_column!(component, expected, actual, successors, whole);
    cmp_column!(component, expected, actual, predecessor_ranges, cmp_range);
    cmp_column!(component, expected, actual, predecessors, whole);
    compare_maneuvers(expected.maneuvers(), actual.maneuvers())?;
    compare_relations(expected.relations(), actual.relations())
}

fn compare_maneuvers(
    expected: &SharedManeuverNetwork,
    actual: &SharedManeuverNetwork,
) -> Result<(), MismatchEvidence> {
    let component = RootComponent::Maneuver;
    cmp_column!(component, expected, actual, movements, whole);
    cmp_column!(component, expected, actual, edge_ranges, cmp_range);
    cmp_column!(component, expected, actual, edges, whole);
    cmp_column!(component, expected, actual, maneuver_gate_ranges, cmp_range);
    cmp_column!(component, expected, actual, maneuver_gates, whole);
    cmp_column!(component, expected, actual, waiting_zone_ranges, cmp_range);
    cmp_column!(component, expected, actual, waiting_zones, whole);
    cmp_column!(component, expected, actual, candidate_ranges, cmp_range);
    cmp_column!(
        component,
        expected,
        actual,
        candidates,
        cmp_maneuver_candidate
    );
    Ok(())
}

fn compare_relations(
    expected: &SharedRelationClosure,
    actual: &SharedRelationClosure,
) -> Result<(), MismatchEvidence> {
    let component = RootComponent::Relations;
    diff_column(
        component,
        "intern",
        &expected.intern,
        &actual.intern,
        |expected, actual| text_whole(expected, actual),
    )?;
    cmp_column!(
        component,
        expected,
        actual,
        corridor_reference_section,
        whole
    );
    cmp_column!(
        component,
        expected,
        actual,
        corridor_element_ranges,
        cmp_range
    );
    cmp_column!(component, expected, actual, corridor_elements, whole);
    cmp_column!(component, expected, actual, section_corridor, whole);
    cmp_column!(component, expected, actual, section_kind, whole);
    cmp_column!(component, expected, actual, section_lane_ranges, cmp_range);
    cmp_column!(component, expected, actual, section_lanes, whole);
    cmp_column!(component, expected, actual, authoring_section, whole);
    cmp_column!(
        component,
        expected,
        actual,
        authoring_edge_ranges,
        cmp_range
    );
    cmp_column!(component, expected, actual, authoring_edges, whole);
    cmp_optional!(component, expected, actual, authoring_group);
    cmp_optional!(component, expected, actual, edge_authoring_lane);
    cmp_optional!(component, expected, actual, edge_junction);
    cmp_optional!(component, expected, actual, edge_stop_line);
    cmp_column!(
        component,
        expected,
        actual,
        junction_movement_ranges,
        cmp_range
    );
    cmp_column!(component, expected, actual, junction_movements, whole);
    cmp_column!(component, expected, actual, movement_junction, whole);
    cmp_column!(component, expected, actual, movement_path_ranges, cmp_range);
    cmp_column!(component, expected, actual, movement_paths, whole);
    cmp_column!(component, expected, actual, stop_line_edge, whole);
    cmp_column!(
        component,
        expected,
        actual,
        stop_line_gate_ranges,
        cmp_range
    );
    cmp_column!(component, expected, actual, stop_line_gates, whole);
    cmp_column!(component, expected, actual, gate_path, whole);
    cmp_column!(component, expected, actual, gate_transition_index, whole);
    cmp_column!(component, expected, actual, gate_stop_line, whole);
    cmp_optional!(component, expected, actual, gate_signal_group);
    cmp_column!(component, expected, actual, waiting_path, whole);
    cmp_column!(component, expected, actual, waiting_entry_gate, whole);
    cmp_column!(component, expected, actual, waiting_release_gate, whole);
    cmp_column!(component, expected, actual, waiting_max_occupancy, whole);
    cmp_column!(component, expected, actual, group_controller, whole);
    cmp_column!(component, expected, actual, group_gate_ranges, cmp_range);
    cmp_column!(component, expected, actual, group_gates, whole);
    cmp_column!(component, expected, actual, controller_offset_ms, whole);
    cmp_column!(component, expected, actual, controller_cycle_ms, whole);
    cmp_column!(
        component,
        expected,
        actual,
        controller_group_ranges,
        cmp_range
    );
    cmp_column!(component, expected, actual, controller_groups, whole);
    cmp_column!(
        component,
        expected,
        actual,
        controller_phase_ranges,
        cmp_range
    );
    cmp_column!(component, expected, actual, controller_phases, whole);
    cmp_column!(component, expected, actual, phase_controller, whole);
    cmp_column!(component, expected, actual, phase_duration_ms, whole);
    cmp_column!(component, expected, actual, phase_end_offset_ms, whole);
    cmp_column!(component, expected, actual, phase_state_ranges, cmp_range);
    cmp_column!(component, expected, actual, phase_state_groups, whole);
    cmp_column!(component, expected, actual, phase_state_aspects, whole);
    cmp_column!(component, expected, actual, parking_space_ranges, cmp_range);
    cmp_column!(component, expected, actual, parking_spaces, whole);
    cmp_column!(component, expected, actual, parking_virtual_capacity, whole);
    cmp_column!(
        component,
        expected,
        actual,
        parking_virtual_entry_ranges,
        cmp_range
    );
    cmp_column!(
        component,
        expected,
        actual,
        parking_virtual_entries,
        cmp_parking_anchor
    );
    cmp_column!(
        component,
        expected,
        actual,
        parking_virtual_exit_ranges,
        cmp_range
    );
    cmp_column!(
        component,
        expected,
        actual,
        parking_virtual_exits,
        cmp_parking_anchor
    );
    cmp_optional!(component, expected, actual, space_area);
    cmp_column!(component, expected, actual, space_entry_edge, whole);
    cmp_column!(component, expected, actual, space_entry_progress, whole);
    cmp_column!(component, expected, actual, space_exit_edge, whole);
    cmp_column!(component, expected, actual, space_exit_progress, whole);
    cmp_column!(component, expected, actual, space_lateral, whole);
    cmp_column!(component, expected, actual, space_heading, f32_bits_whole);
    cmp_column!(component, expected, actual, space_length, whole);
    cmp_column!(component, expected, actual, space_width, whole);
    cmp_column!(component, expected, actual, lane_group_section, whole);
    cmp_column!(
        component,
        expected,
        actual,
        lane_group_member_ranges,
        cmp_range
    );
    cmp_column!(component, expected, actual, lane_group_members, whole);
    cmp_column!(component, expected, actual, band_corridor, whole);
    cmp_column!(component, expected, actual, band_kind, whole);
    cmp_optional!(component, expected, actual, class_parent);
    cmp_column!(component, expected, actual, class_depth, whole);
    cmp_column!(component, expected, actual, class_subtree_enter, whole);
    cmp_column!(component, expected, actual, class_subtree_exit, whole);
    cmp_column!(component, expected, actual, rule_target, whole);
    cmp_column!(component, expected, actual, rule_effect, whole);
    cmp_column!(component, expected, actual, rule_class_ranges, cmp_range);
    cmp_column!(component, expected, actual, rule_classes, whole);
    cmp_column!(component, expected, actual, rule_priority, whole);
    cmp_column!(component, expected, actual, profile_class, whole);
    cmp_column!(component, expected, actual, profile_length, whole);
    cmp_column!(component, expected, actual, profile_desired_speed, whole);
    cmp_column!(component, expected, actual, profile_min_gap, whole);
    cmp_column!(
        component,
        expected,
        actual,
        profile_time_headway,
        f32_bits_whole
    );
    cmp_column!(
        component,
        expected,
        actual,
        profile_max_accel,
        f32_bits_whole
    );
    cmp_column!(
        component,
        expected,
        actual,
        profile_comfort_decel,
        f32_bits_whole
    );
    cmp_column!(
        component,
        expected,
        actual,
        profile_emergency_decel,
        f32_bits_whole
    );
    if expected.access_class_count != actual.access_class_count {
        return Err(evidence(MismatchSite::TableValue {
            component,
            table: "access_class_count",
            entity_kind: None,
            index: 0,
            field: "count",
            expected: expected.access_class_count.to_string(),
            actual: actual.access_class_count.to_string(),
        }));
    }
    cmp_column!(component, expected, actual, edge_row_starts, whole);
    cmp_column!(component, expected, actual, edge_cells, whole);
    cmp_column!(component, expected, actual, path_row_starts, whole);
    cmp_column!(component, expected, actual, path_cells, whole);
    Ok(())
}

/// `OptionalColumn` 元素（`Option<T>`）：`Some` / `None` 判别与载荷按值比较。
fn option_column<T: PartialEq + Debug>(expected: &Option<T>, actual: &Option<T>) -> FieldDiff {
    whole(expected, actual)
}

fn compare_identity(
    expected: &crate::identity::SharedIdentityIndex,
    actual: &crate::identity::SharedIdentityIndex,
) -> Result<(), MismatchEvidence> {
    use laneflow_static_contract::EntityKind;
    let component = RootComponent::Identity;
    // forward 按 EntityKind 展开为多张表；kind 进入证据身份——同一失配移到
    // 另一个 kind 的表是不同证据（复算归类不得判为同一失配）。
    for (kind_index, kind) in EntityKind::ALL.iter().enumerate() {
        let expected_column = &expected.forward[kind_index];
        let actual_column = &actual.forward[kind_index];
        if expected_column.len() != actual_column.len() {
            return Err(evidence(MismatchSite::TableLength {
                component,
                table: "forward",
                entity_kind: Some(*kind),
                expected_len: u32_len(expected_column),
                actual_len: u32_len(actual_column),
            }));
        }
        for (index, (expected_id, actual_id)) in
            expected_column.iter().zip(actual_column).enumerate()
        {
            if expected_id != actual_id {
                return Err(evidence(MismatchSite::TableValue {
                    component,
                    table: "forward",
                    entity_kind: Some(*kind),
                    index: u32::try_from(index).expect("column index fits u32"),
                    field: "stable_id",
                    expected: format!("{expected_id:?}"),
                    actual: format!("{actual_id:?}"),
                }));
            }
        }
    }
    diff_column(
        component,
        "reverse",
        &expected.reverse,
        &actual.reverse,
        cmp_identity_reverse_entry,
    )
}

fn compare_planning_hints(
    expected: &crate::traffic::PartitionPlanningHints,
    actual: &crate::traffic::PartitionPlanningHints,
) -> Result<(), MismatchEvidence> {
    diff_column(
        RootComponent::PlanningHints,
        "edge_boundary_weights",
        expected.edge_boundary_weights(),
        actual.edge_boundary_weights(),
        whole,
    )
}

fn compare_conflict(
    expected: &crate::conflict::SharedConflictNetwork,
    actual: &crate::conflict::SharedConflictNetwork,
) -> Result<(), MismatchEvidence> {
    let component = RootComponent::Conflict;
    cmp_column!(component, expected, actual, zone_junctions, whole);
    cmp_column!(component, expected, actual, junction_zone_ranges, cmp_range);
    cmp_column!(component, expected, actual, junction_zones, whole);
    cmp_column!(component, expected, actual, zone_stream_ranges, cmp_range);
    cmp_column!(component, expected, actual, zone_streams, whole);
    cmp_column!(component, expected, actual, stream_junctions, whole);
    cmp_column!(
        component,
        expected,
        actual,
        junction_stream_ranges,
        cmp_range
    );
    cmp_column!(component, expected, actual, junction_streams, whole);
    cmp_column!(component, expected, actual, stream_paths, whole);
    cmp_column!(component, expected, actual, path_stream_ranges, cmp_range);
    cmp_column!(component, expected, actual, path_streams, whole);
    cmp_column!(
        component,
        expected,
        actual,
        stream_passage_ranges,
        cmp_range
    );
    cmp_column!(component, expected, actual, passages, cmp_conflict_passage);
    Ok(())
}

fn compare_policy(
    expected: &crate::policy::SharedPolicyNetwork,
    actual: &crate::policy::SharedPolicyNetwork,
) -> Result<(), MismatchEvidence> {
    let component = RootComponent::Policy;
    cmp_column!(component, expected, actual, policies, cmp_policy_record);
    cmp_column!(component, expected, actual, rules, cmp_rule_record);
    cmp_column!(component, expected, actual, evidence, cmp_policy_evidence);
    cmp_column!(component, expected, actual, evidence_refs, whole);
    cmp_column!(component, expected, actual, gaps, cmp_gap_profile);
    cmp_owner_index(
        component,
        "gate_owners",
        &expected.gate_owners,
        &actual.gate_owners,
    )?;
    cmp_owner_index(
        component,
        "stream_owners",
        &expected.stream_owners,
        &actual.stream_owners,
    )?;
    cmp_column!(component, expected, actual, gates, cmp_resolved_gate_policy);
    cmp_column!(
        component,
        expected,
        actual,
        streams,
        cmp_resolved_stream_policy
    );
    cmp_column!(component, expected, actual, target_ranges, cmp_target_range);
    cmp_column!(component, expected, actual, targets, cmp_yield_target_cell);
    Ok(())
}

fn compare_spatial(
    expected: Option<&SharedSpatialNetwork>,
    actual: Option<&SharedSpatialNetwork>,
) -> Result<(), MismatchEvidence> {
    if expected.is_some() != actual.is_some() {
        return Err(evidence(MismatchSite::SpatialPresence {
            expected: expected.is_some(),
            actual: actual.is_some(),
        }));
    }
    let (Some(expected), Some(actual)) = (expected, actual) else {
        return Ok(());
    };
    let component = RootComponent::Spatial;
    if expected.direction_profile() != actual.direction_profile() {
        return Err(evidence(MismatchSite::TableValue {
            component,
            table: "direction_profile",
            entity_kind: None,
            index: 0,
            field: "value",
            expected: expected.direction_profile().to_string(),
            actual: actual.direction_profile().to_string(),
        }));
    }
    match (&expected.lane_pose, &actual.lane_pose) {
        (Some(expected_pose), Some(actual_pose)) => {
            compare_lane_pose(expected_pose, actual_pose)?;
        }
        (None, None) => {}
        (expected_pose, actual_pose) => {
            return Err(evidence(MismatchSite::TableValue {
                component,
                table: "lane_pose",
                entity_kind: None,
                index: 0,
                field: "presence",
                expected: format!("{:?}", expected_pose.is_some()),
                actual: format!("{:?}", actual_pose.is_some()),
            }));
        }
    }
    cmp_column!(
        component,
        expected,
        actual,
        facility_entries,
        cmp_facility_entry
    );
    cmp_column!(component, expected, actual, facility_points, cmp_point);
    cmp_column!(
        component,
        expected,
        actual,
        conflict_region_entries,
        cmp_conflict_region_entry
    );
    cmp_column!(
        component,
        expected,
        actual,
        conflict_region_points,
        cmp_point_xz
    );
    Ok(())
}

fn compare_lane_pose(
    expected: &LanePoseNetwork,
    actual: &LanePoseNetwork,
) -> Result<(), MismatchEvidence> {
    let component = RootComponent::Spatial;
    cmp_column!(component, expected, actual, canonical_frames, whole);
    cmp_column!(
        component,
        expected,
        actual,
        arc_lengths_meters,
        f32_bits_whole
    );
    cmp_column!(component, expected, actual, point_ranges, cmp_range);
    cmp_column!(component, expected, actual, points, cmp_point);
    cmp_column!(component, expected, actual, segment_ranges, cmp_range);
    cmp_column!(component, expected, actual, segments, cmp_segment);
    Ok(())
}
