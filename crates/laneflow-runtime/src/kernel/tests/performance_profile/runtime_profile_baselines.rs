//! CPU 采样与 Journal A/B 共用的冻结状态摘要（BLAKE3-256，摘要版本 8）。
//! 新基线须复核夹具身份与行为变化；Journal 开关不改变预期的已提交状态。

#[derive(Clone, Copy)]
pub(super) struct LaneBaseline {
    pub(super) count: u32,
    pub(super) edges: u32,
    pub(super) digest: &'static str,
}

pub(super) const LANE_1K_256: LaneBaseline = LaneBaseline {
    count: 1_000,
    edges: 256,
    digest: "01571d70db0c99b69b4bbdc03ee892eb49dac367d70ab6c087198b9e5663b0ca",
};

pub(super) const LANE_10K_256: LaneBaseline = LaneBaseline {
    count: 10_000,
    edges: 256,
    digest: "cda9cef9dc2b12f540a8a965648949f3126b5d030a33ce4be20545d3c570b583",
};

pub(super) const LANE_10K_16: LaneBaseline = LaneBaseline {
    count: 10_000,
    edges: 16,
    digest: "25dae5a372d4bcb6582e844121137948c2dc5ba80b9c4b8693b98995ca6f1371",
};
