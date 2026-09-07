//! 按流和冲突区索引 passage；不为不相交区域枚举 passage 笛卡尔积。
use super::work::WorkBudget;
use super::*;

/// 冲突通行段索引行：定位某参与者流在某冲突区内的一段冲突通行段。
#[derive(Clone, Copy)]
pub(super) struct Entry {
    stream: MirParticipantStreamKey,
    zone: MirConflictZoneKey,
    pub passage: u32,
}

/// 冲突通行段索引：按（参与者流, 冲突区, 通行段）排序，支持区间二分查询。
pub(super) struct PassageIndex {
    entries: Vec<Entry>,
}

impl PassageIndex {
    /// 收集全部冲突通行段并排序构建索引；先校验临时内存与记录数上限。
    pub(super) fn build(unit: &CompilationUnit, mir: &MirUnit) -> Result<Self, DiagnosticBundle> {
        let count = mir.conflict_passages.len() as u64;
        super::validation::budget(
            unit,
            mir,
            count.saturating_mul(size_of::<Entry>() as u64),
            count,
        )?;
        let mut entries = Vec::with_capacity(mir.conflict_passages.len());
        for (stream, value) in mir.participant_streams.iter().enumerate() {
            for passage in value.passages.as_usize_range() {
                entries.push(Entry {
                    stream: MirParticipantStreamKey::from_raw(stream as u32),
                    zone: mir.conflict_passages[passage].conflict_zone,
                    passage: passage as u32,
                });
            }
        }
        entries.sort_unstable_by_key(|v| (v.stream, v.zone, v.passage));
        Ok(Self { entries })
    }

    /// 返回索引占用的字节数。
    pub(super) fn bytes(&self) -> u64 {
        (self.entries.capacity() as u64).saturating_mul(size_of::<Entry>() as u64)
    }

    /// 返回指定参与者流在指定冲突区内的通行段条目。
    pub(super) fn in_zone(
        &self,
        stream: MirParticipantStreamKey,
        zone: MirConflictZoneKey,
    ) -> &[Entry] {
        let key = (stream, zone);
        &self.entries[self.entries.partition_point(|v| (v.stream, v.zone) < key)
            ..self.entries.partition_point(|v| (v.stream, v.zone) <= key)]
    }

    fn stream(&self, stream: MirParticipantStreamKey) -> &[Entry] {
        &self.entries[self.entries.partition_point(|v| v.stream < stream)
            ..self.entries.partition_point(|v| v.stream <= stream)]
    }

    /// 归并扫描判定两个参与者流是否共享任一冲突区；每步比较计入工作预算。
    pub(super) fn shares_zone(
        &self,
        a: MirParticipantStreamKey,
        b: MirParticipantStreamKey,
        work: &mut WorkBudget,
    ) -> Result<bool, DiagnosticBundle> {
        let mut a = self.stream(a);
        let mut b = self.stream(b);
        while let (Some(left), Some(right)) = (a.first(), b.first()) {
            work.charge(1)?;
            match left.zone.cmp(&right.zone) {
                std::cmp::Ordering::Equal => return Ok(true),
                std::cmp::Ordering::Less => a = &a[1..],
                std::cmp::Ordering::Greater => b = &b[1..],
            }
        }
        Ok(false)
    }
}
