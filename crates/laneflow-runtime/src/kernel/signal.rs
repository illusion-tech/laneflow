use laneflow_static_contract::{SignalAspect, SignalGroupOrdinal};

/// 稳定按组序号的已提交信号指示。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommittedSignalGroupBatch {
    pub(crate) items: Vec<(SignalGroupOrdinal, SignalAspect)>,
}

impl CommittedSignalGroupBatch {
    /// 按组序号稳定顺序返回已提交信号指示条目切片。
    #[must_use]
    pub fn as_slice(&self) -> &[(SignalGroupOrdinal, SignalAspect)] {
        &self.items
    }
}
