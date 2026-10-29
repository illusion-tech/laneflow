//! LF-SYNTH-v2 离线规范输入；不执行性能测量或产品认证。

mod bundle;
mod encoding;
mod network;
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

/// 四种冻结交通场景。
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum Case {
    W1,
    W2,
    W3,
    W4,
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
