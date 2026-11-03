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
        if self.clusters == 0
            || self.changers_per_cluster < 2
            || self.clusters > 64
            || self.changers_per_cluster > 64
        {
            return Err(Error::Invalid("换道族参数越界".into()));
        }
        Ok(())
    }
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
    pub(crate) fn lane_change_window(&self) -> (u64, u64) {
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
