//! LF-SYNTH-v2 离线规范输入；不执行性能测量或产品认证。

mod bundle;
mod encoding;
mod network;
mod network_lane_change;
mod plan;
mod state;

use serde::{Deserialize, Serialize};

pub use bundle::{Bundle, FileRecord, Manifest, Provenance, generate, verify_directory};
pub use state::{InitialState, RouteRecord, VehicleRecord};

const NAMESPACE: &str = "lf-synth";
const BUILD_ID: &str = "laneflow-synth-generator-1";
const POLICY_KEY: &str = "protected-entry";
const EDGE_MM: u32 = 5_130_000;
const NOMINAL_MS: [u32; 6] = [30_000, 3_000, 1_000, 20_000, 3_000, 1_000];

/// 交通场景形状；W 族为信号化背景，换道族由参数给出密度维度。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum Case {
    W1,
    W2,
    W3,
    W4,
    /// 换道族：`clusters` 个横向过渡段簇 × 每簇 `changers_per_cluster` 名换道者；
    /// 过渡密度 = 过渡出现项 / 边数，候选密度 = 候选数 / 过渡出现项。
    LaneChange(LaneChangeParams),
}

impl Case {
    /// 场景族标签（内部任务/目录命名口径，不参与序列化）。
    pub fn label(&self) -> &'static str {
        match self {
            Case::W1 => "W1",
            Case::W2 => "W2",
            Case::W3 => "W3",
            Case::W4 => "W4",
            Case::LaneChange(_) => "L1",
        }
    }
}

/// 换道族参数（确定性输入的可复现参数）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct LaneChangeParams {
    pub clusters: u32,
    pub changers_per_cluster: u32,
}

impl LaneChangeParams {
    /// 参数校验：至少一簇、每簇至少两名竞争者，且总量有界。
    ///
    /// # Errors
    /// 越界时返回错误。
    pub fn validate(&self) -> Result<(), Error> {
        // 每簇换道者串行通过 T：观察窗必须早于 leader 完成路线，而末车必须
        // 在窗内完成 crossing。当前下游几何（约 72 m）与加速曲线下，只有
        // K=2（R1b s12 钉版）同时满足；K≠2 需同步扩展下游几何与窗口模型
        // （#1233 F1：K=3 的末车 crossing 仅比 leader 完成晚数拍，无可行的
        // 观察窗口）。簇数上界防止输入规模失控。
        if self.changers_per_cluster != 2 || self.clusters == 0 || self.clusters > 64 {
            return Err(Error::Invalid("换道族参数越界".into()));
        }
        Ok(())
    }
}

/// 换道者入口边（`lc-e0`）长度（毫米，#1233 对象 b/F1）。
///
/// 可行域内（K=2）保持 30 m 钉版几何。
pub(crate) fn lane_change_entry_length_mm(_changers_per_cluster: u32) -> u64 {
    30_000
}

/// 换道者在入口边上的确定性前保险杠位置（毫米，#1233 对象 b/F1）。
///
/// R1b s12 钉版：leader 11,000 / follower 3,000（净间隙 3,500 > min_gap，
/// 满足 spawn 安全与无重叠）。
pub(crate) fn lane_change_slot_progress_mm(_changers_per_cluster: u32, slot: u32) -> u32 {
    if slot == 0 { 11_000 } else { 3_000 }
}

/// 交通输入身份；表现比例不参与本身份。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub workload_id: String,
    pub scale: u32,
    pub case: Case,
    pub fixed_delta_ms: u32,
    pub seed: u64,
}

impl Identity {
    /// 构造正式的一万或十万输入身份。
    ///
    /// # Errors
    /// 规模或步长不属于冻结集合时返回错误。
    pub fn new(scale: u32, case: Case, fixed_delta_ms: u32) -> Result<Self, Error> {
        if let Case::LaneChange(params) = &case {
            params.validate()?;
        }
        let identity = Self {
            workload_id: "LF-SYNTH-v2".into(),
            scale,
            case,
            fixed_delta_ms,
            seed: 0,
        };
        identity.validate()?;
        Ok(identity)
    }

    /// 场景族标签（#1233：供 worker/矩阵等内部命名口径使用）。
    pub fn scenario_label(&self) -> &'static str {
        self.case.label()
    }

    /// 换道族参数（W 族为 `None`）。
    pub fn lane_change(&self) -> Option<&LaneChangeParams> {
        match &self.case {
            Case::LaneChange(params) => Some(params),
            _ => None,
        }
    }

    fn validate(&self) -> Result<(), Error> {
        if self.workload_id != "LF-SYNTH-v2"
            || !matches!(self.scale, 10_000 | 100_000)
            || !matches!(self.fixed_delta_ms, 16 | 33)
            || self.seed != 0
        {
            return Err(Error::Invalid("非正式 LF-SYNTH-v2 输入身份".into()));
        }
        Ok(())
    }

    pub(crate) fn cells(&self) -> u32 {
        self.scale / 100
    }

    pub(crate) fn durations(&self) -> [u32; 6] {
        NOMINAL_MS.map(|ms| ms.div_ceil(self.fixed_delta_ms) * self.fixed_delta_ms)
    }

    pub(crate) fn cycle_ms(&self) -> u32 {
        self.durations().into_iter().sum()
    }

    pub(crate) fn warm_up_ticks(&self) -> u32 {
        4 * self.cycle_ms() / self.fixed_delta_ms
    }

    /// L1 换道族自有窗口（#1218 D3：出生位钉版，恢复检查点钉在首 crossing 前）。
    ///
    /// 预热 16 拍（F3 检查点 0/1 = 16/32 拍，早于首 crossing）；观察窗按步长钉版
    ///（探针 `lane_change_lifecycle_probe_freezes_window_pinning` 冻结拍号）：
    /// 终点晚于败者清空后进入拍、早于任一换道者路线完成拍。
    pub fn lane_change_window(&self) -> (u64, u64) {
        // K=2 钉版窗口（调度可行域推导见 LaneChangeParams::validate）。
        let observation = if self.fixed_delta_ms == 16 { 304 } else { 144 };
        (16, observation)
    }
}

/// 生成、受检编译或文件验证失败。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("输入包无效：{0}")]
    Invalid(String),
    #[error("静态编译失败（{stage}）：{detail}")]
    Compile { stage: &'static str, detail: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

fn compile_error(stage: &'static str, error: impl std::fmt::Debug) -> Error {
    Error::Compile {
        stage,
        detail: format!("{error:?}"),
    }
}

fn key(cell: u32, route: u32, name: &str) -> String {
    format!("c{cell:04}-r{route}-{name}")
}

#[cfg(test)]
mod tests;
