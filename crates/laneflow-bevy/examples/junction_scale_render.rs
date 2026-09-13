//! #285 相同逐拍输入下的离屏渲染成本；显式等待 GPU 完成每帧。
#[path = "support/junction_scale.rs"]
mod scale;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    scale::run(false, true)
}
