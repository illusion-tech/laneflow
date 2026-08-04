//! 编译器、制品验证器与运行时共享的静态枚举值。
//!
//! 本模块只定义不依赖存储布局的封闭值域；它不解释运行时通行权，也不持有控制器
//! 状态。制品编码仍须由对应格式版本显式规定，不能把 Rust 判别值直接当作线格式。

/// 固定时制信号相位对一个 `SignalGroup` 给出的灯色指示。
///
/// 该值只表达信号指示，不是交通参与单元的最终通行权。`Yellow` 在 current
/// protected-only compliance profile 中按限制性黄灯解释，但该行为策略不属于本值类型。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum SignalAspect {
    /// 红灯指示。
    Red,
    /// 黄灯指示。
    Yellow,
    /// 绿灯指示。
    Green,
}
