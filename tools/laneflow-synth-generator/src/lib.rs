//! LF-SYNTH-v2 离线规范输入；不执行性能测量或产品认证。

mod bundle;
mod encoding;
mod network;
mod network_lane_change;
mod plan;
mod state;

use serde::{Deserialize, Serialize};

pub use bundle::{Bundle, FileRecord, Manifest, Provenance, generate, verify_directory};
pub use state::{HandoffDrivingInput, HandoffInput, InitialState, RouteRecord, VehicleRecord};

#[cfg(test)]
extern crate self as laneflow_synth_generator;

/// LF-SYNTH 场景中刚性双轴乘用车的显式作者参数。
pub const PASSENGER_CAR_GEOMETRY: laneflow_compiler::RoadVehicleGeometryInput =
    laneflow_compiler::RoadVehicleGeometryInput::RigidTwoAxle(
        laneflow_compiler::RigidTwoAxleGeometryInput {
            wheelbase_m: 2.6,
            track_width_m: 1.6,
            reference_height_m: 0.35,
            rear_overhang_m: 0.75,
            width_m: 1.9,
            bottom_height_m: -0.15,
            top_height_m: 1.15,
            max_steering_angle_rad: 0.6,
            max_steering_rate_rad_s: 0.7,
        },
    );

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
    /// 两辆车从外侧车道同拍争用中间车道。
    #[serde(rename = "RouteHandoff")]
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
    /// 每簇恰有两名竞争者，簇数为 1 至 64。
    ///
    /// # Errors
    /// 越界时返回错误。
    pub fn validate(&self) -> Result<(), Error> {
        // 每簇的两个源车道分别注册一条指向中间车道的路线。
        if self.changers_per_cluster != 2 || self.clusters == 0 || self.clusters > 64 {
            return Err(Error::Invalid("换道族参数越界".into()));
        }
        Ok(())
    }
}

// 三条无坡直车道上，两车的初始前杠纵向对齐，分别按所在源车道的限速行驶。
const HANDOFF_FRONT_MM: u32 = 13_750;
const HANDOFF_SPEED_MM_S: [u32; 2] = [3_000, 1_500];

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
    /// 规模、步长或换道族参数越界时返回错误。
    pub fn new(scale: u32, case: Case, fixed_delta_ms: u32) -> Result<Self, Error> {
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

    /// 冻结集合校验（构造、`generate` 与目录核验共用同一可行域，
    /// 反序列化或改写 `case` 字段的身份不绕过换道参数检查，#1243 T3）。
    fn validate(&self) -> Result<(), Error> {
        if self.workload_id != "LF-SYNTH-v2"
            || !matches!(self.scale, 10_000 | 100_000)
            || !matches!(self.fixed_delta_ms, 16 | 33)
            || self.seed != 0
        {
            return Err(Error::Invalid("非正式 LF-SYNTH-v2 输入身份".into()));
        }
        if let Case::LaneChange(params) = &self.case {
            params.validate()?;
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

    /// 预热末拍保留首车的活动横移，两车清空后仍至少有一拍供恢复重放。
    pub fn lane_change_window(&self) -> (u64, u64) {
        lane_change_pinned_window(self.fixed_delta_ms)
    }
}

/// L1 换道族钉版窗口的唯一正本：harness 的 worker/restore 矩阵从本函数统一
/// 派生期望窗口，不自持第二套字面量（曾因 #1272 收束边界放宽后未同步而
/// 静默拒绝规范报告）。
///
/// 两种固定步长均覆盖两车的进入、前杠交接和车尾清空。
#[must_use]
pub fn lane_change_pinned_window(fixed_delta_ms: u32) -> (u64, u64) {
    let observation = if fixed_delta_ms == 16 { 2_064_u64 } else { 992 };
    (16, observation)
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
