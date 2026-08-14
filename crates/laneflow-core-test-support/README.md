# laneflow-core-test-support

`laneflow-core-test-support` 是 `laneflow-core` 集成测试（`tests/`）与 Criterion
基准（`benches/`）共享的夹具包。它承载命令校验、停车运行时、信号校验、车辆跟驰
场景，以及数值精度、数值契约标定、最小边长、路线距离的研究专用候选。

本包不是生产接口，不改变 `laneflow-core` 的 feature 集合与公开表面；夹具模块内的
既有 `pub` 项零改名、零签名变更。包保持 `publish = false`，仅作为 workspace 私有
成员经 path 依赖 `laneflow-core`，并由 `laneflow-core` 以 dev-dependencies 引用
（dev-dependency 环为 Cargo 正式支持形态），消除 tests 与 benches 之间的交叉
`#[path]` 挂载。

使用方只有 `laneflow-core` 的 `tests/` 与 `benches/` 目标；其它 crate 不得依赖本包。
