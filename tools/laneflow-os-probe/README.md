# laneflow-os-probe

本进程与整机操作系统计数器的唯一读取入口，供测量与诊断工具在计时区间外调用。
设计、字段语义与 `unsafe` 边界见 [`os-probe.md`][os-probe]。

- 读取整机、本进程与当前线程的 CPU 时间，本进程内存与缺页，Linux 线程就绪等待，
  以及交流电与电池状态。
- 只返回原始值；平台不提供的字段为 `None`，两个平台语义不同的字段分别命名。
- 只允许 `tools/` 下的 crate 依赖，或作为 dev-dependency 引用；`crates/` 下的生产
  依赖引用时 `check-wire-audit` 失败。
- `unsafe` 只在 `src/windows.rs` 的 5 处固定调用，由 `check-wire-audit` 逐处比对。

```bash
cargo +1.100.0 test -p laneflow-os-probe --locked
```

[os-probe]: ../../docs/design/os-probe.md
