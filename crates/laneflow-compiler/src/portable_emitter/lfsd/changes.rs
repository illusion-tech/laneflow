//! LFSD 变化表：对象组装前同时存续的变化行（资源账本设计第 6.2 节）。
//!
//! 每类变化按两侧行数求得的上界授予连续存储；变化载荷或借用受检制品字节，或写入一段
//! 共用的暂存字节区。对象树由变化表以计数与构建两种模式写出，变化表本身不再复制载荷。

use super::super::tree::{SectionRef, TreeBuilder, Value, field};
use super::*;
use crate::ledger::{BoundedVec, Plan};

/// 变化载荷：受检制品中的原字节，或暂存字节区中的区间。
#[derive(Clone, Copy, Debug)]
pub(super) enum Payload<'a> {
    View(&'a [u8]),
    Staged(Slots),
}

impl<'a> Payload<'a> {
    /// 载荷字节。
    pub(super) fn bytes<'s>(self, staged: &'s [u8]) -> &'s [u8]
    where
        'a: 's,
    {
        match self {
            Self::View(bytes) => bytes,
            Self::Staged(slots) => slots.of(staged),
        }
    }
}

/// 以 `write` 向暂存字节区追加一段载荷，返回其区间。
pub(super) fn stage<E>(
    staged: &mut BoundedVec<u8>,
    write: impl FnOnce(&mut BoundedVec<u8>) -> Result<(), E>,
) -> Result<Slots, E> {
    let start = staged.len();
    write(staged)?;
    Ok(Slots {
        start,
        end: staged.len(),
    })
}

/// 实体、静态规则或几何的一条变化：变化种类、主体、字段标签与前后载荷。
#[derive(Clone, Copy, Debug)]
pub(super) struct FieldChange<'a> {
    pub(super) change_kind: u8,
    pub(super) entity_kind: EntityKind,
    pub(super) stable_id: [u8; 16],
    pub(super) field_tag: u16,
    pub(super) before: Option<Payload<'a>>,
    pub(super) after: Option<Payload<'a>>,
}

impl FieldChange<'_> {
    /// 规范排序键：变化种类、实体种类、稳定标识与字段标签。
    pub(super) const fn key(&self) -> (u8, EntityKind, [u8; 16], u16) {
        (
            self.change_kind,
            self.entity_kind,
            self.stable_id,
            self.field_tag,
        )
    }
}

/// 路权策略局部成员的一条变化。
#[derive(Clone, Copy, Debug)]
pub(super) struct PolicyChange<'a> {
    pub(super) op: u8,
    pub(super) owner: [u8; 16],
    pub(super) kind: u8,
    pub(super) key: &'a str,
    pub(super) before: Option<Slots>,
    pub(super) after: Option<Slots>,
}

/// 空间配置变化：Genesis 只带目标一侧，制品差异带前后两侧。
#[derive(Clone, Copy, Debug)]
pub(super) enum SpatialChange<'a> {
    Genesis(&'a [u8]),
    Changed(&'a [u8], &'a [u8]),
}

/// 一次差异发射在 LFSD 对象组装前同时存续的全部变化表。
pub(super) struct DiffChanges<'a> {
    pub(super) entity: BoundedVec<FieldChange<'a>>,
    pub(super) relation: BoundedVec<RelationChange>,
    pub(super) geometry: BoundedVec<FieldChange<'a>>,
    pub(super) static_rule: BoundedVec<FieldChange<'a>>,
    pub(super) spatial: Option<SpatialChange<'a>>,
    pub(super) policy: BoundedVec<PolicyChange<'a>>,
    pub(super) staged: BoundedVec<u8>,
}

impl DiffChanges<'_> {
    /// 变化表持有的受控请求字节。
    pub(super) const fn granted_bytes(&self) -> u64 {
        self.entity.granted_bytes()
            + self.relation.granted_bytes()
            + self.geometry.granted_bytes()
            + self.static_rule.granted_bytes()
            + self.policy.granted_bytes()
            + self.staged.granted_bytes()
    }
}

/// 各变化表容量的上界。
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ChangeBounds {
    pub(super) entity: usize,
    pub(super) relation: usize,
    pub(super) geometry: usize,
    pub(super) static_rule: usize,
    pub(super) policy: usize,
    pub(super) staged: usize,
}

impl ChangeBounds {
    /// 变化表同时存续的计划峰值。
    pub(super) fn plan(self) -> Plan {
        Plan::ZERO
            .array::<FieldChange<'_>>(self.entity)
            .array::<RelationChange>(self.relation)
            .array::<FieldChange<'_>>(self.geometry)
            .array::<FieldChange<'_>>(self.static_rule)
            .array::<PolicyChange<'_>>(self.policy)
            .array::<u8>(self.staged)
    }

    /// 按上界取得各变化表的授予。
    pub(super) fn grant<'a>(
        self,
        pass: &crate::ledger::PassBudget<'_>,
    ) -> Result<DiffChanges<'a>, PortableEmissionError> {
        Ok(DiffChanges {
            entity: pass.vec(self.entity)?,
            relation: pass.vec(self.relation)?,
            geometry: pass.vec(self.geometry)?,
            static_rule: pass.vec(self.static_rule)?,
            spatial: None,
            policy: pass.vec(self.policy)?,
            staged: pass.vec(self.staged)?,
        })
    }
}

/// 受检和的累加。
pub(super) fn add(total: &mut usize, value: usize) -> Result<(), PortableEmissionError> {
    *total = total
        .checked_add(value)
        .ok_or(PortableEmissionError::ArithmeticOverflow)?;
    Ok(())
}

/// LFSD 第 1 节差异绑定。
pub(super) enum DiffHeader {
    Genesis {
        network_revision: NetworkRevisionId,
        artifact_digest: Sha256Digest,
        artifact_length: u64,
    },
    Artifact {
        base_network_revision: NetworkRevisionId,
        base_digest: Sha256Digest,
        base_length: u64,
        network_revision: NetworkRevisionId,
        artifact_digest: Sha256Digest,
        artifact_length: u64,
    },
}

#[derive(Clone, Copy)]
enum TagMode {
    Never,
    Modify,
    Always,
}

fn field_change_table(
    t: &TreeBuilder,
    changes: &[FieldChange<'_>],
    staged: &[u8],
    tag: TagMode,
    change_kind: Option<u8>,
) -> super::super::tree::TableRef {
    t.table(
        1,
        changes.iter().map(|change| {
            let with_tag = match tag {
                TagMode::Never => false,
                TagMode::Modify => change.change_kind == 2,
                TagMode::Always => true,
            };
            t.row([
                Some(field(
                    1,
                    Value::U8(change_kind.unwrap_or(change.change_kind)),
                )),
                Some(field(2, Value::U16(change.entity_kind.code()))),
                Some(field(4, Value::StableId128(change.stable_id))),
                with_tag.then(|| field(6, Value::U16(change.field_tag))),
                change
                    .before
                    .map(|before| field(9, t.bytes(before.bytes(staged)))),
                change
                    .after
                    .map(|after| field(10, t.bytes(after.bytes(staged)))),
            ])
        }),
    )
}

/// 由变化表写出 LFSD 对象的七节。
pub(super) fn emit_lfsd(
    t: &TreeBuilder,
    header: &DiffHeader,
    changes: &DiffChanges<'_>,
) -> [SectionRef; 7] {
    let staged = &changes.staged[..];
    let binding = match *header {
        DiffHeader::Genesis {
            network_revision,
            artifact_digest,
            artifact_length,
        } => t.row([
            field(1, Value::U8(0)),
            field(2, Value::U16(0)),
            field(3, t.sha256([0; 32])),
            field(4, t.sha256([0; 32])),
            field(5, Value::U64(0)),
            field(6, Value::U16(NETWORK_REVISION_DERIVATION_VERSION)),
            field(7, t.sha256(network_revision.into_digest().into_bytes())),
            field(8, t.sha256(artifact_digest.into_bytes())),
            field(9, Value::U64(artifact_length)),
        ]),
        DiffHeader::Artifact {
            base_network_revision,
            base_digest,
            base_length,
            network_revision,
            artifact_digest,
            artifact_length,
        } => t.row([
            field(1, Value::U8(1)),
            field(2, Value::U16(NETWORK_REVISION_DERIVATION_VERSION)),
            field(
                3,
                t.sha256(base_network_revision.into_digest().into_bytes()),
            ),
            field(4, t.sha256(base_digest.into_bytes())),
            field(5, Value::U64(base_length)),
            field(6, Value::U16(NETWORK_REVISION_DERIVATION_VERSION)),
            field(7, t.sha256(network_revision.into_digest().into_bytes())),
            field(8, t.sha256(artifact_digest.into_bytes())),
            field(9, Value::U64(artifact_length)),
        ]),
    };
    [
        t.section(1, [t.table(1, [binding])]),
        t.section(
            2,
            [field_change_table(
                t,
                &changes.entity,
                staged,
                TagMode::Modify,
                None,
            )],
        ),
        t.section(
            3,
            [t.table(
                1,
                changes
                    .relation
                    .iter()
                    .map(|change| relation::relation_change_row(t, *change)),
            )],
        ),
        t.section(
            4,
            [field_change_table(
                t,
                &changes.geometry,
                staged,
                TagMode::Never,
                None,
            )],
        ),
        t.section(
            5,
            [field_change_table(
                t,
                &changes.static_rule,
                staged,
                TagMode::Always,
                Some(0),
            )],
        ),
        t.section(
            6,
            [t.table(
                1,
                changes.spatial.map(|spatial| match spatial {
                    SpatialChange::Genesis(presence) => {
                        t.row([field(1, Value::U8(0)), field(3, t.bytes(presence))])
                    }
                    SpatialChange::Changed(before, after) => t.row([
                        field(1, Value::U8(1)),
                        field(2, t.bytes(before)),
                        field(3, t.bytes(after)),
                    ]),
                }),
            )],
        ),
        t.section(
            7,
            [t.table(
                1,
                changes.policy.iter().map(|change| {
                    t.row([
                        Some(field(1, Value::U8(change.op))),
                        Some(field(2, Value::StableId128(change.owner))),
                        Some(field(3, Value::U8(change.kind))),
                        Some(field(4, t.utf8(change.key))),
                        change
                            .before
                            .map(|before| field(5, t.bytes(before.of(staged)))),
                        change
                            .after
                            .map(|after| field(6, t.bytes(after.of(staged)))),
                    ])
                }),
            )],
        ),
    ]
}
