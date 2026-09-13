//! #285 单世界复杂路口跨层成本采样；输入和窗口必须先冻结。
#[path = "support/junction_scale.rs"]
mod scale;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    scale::run(false, false)
}
