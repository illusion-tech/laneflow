# laneflow-host-observe

测量环境记录、进程快照与子进程观测，设计见
[`host-observe.md`][host-observe]。本 crate 继承工作区
`unsafe_code = "forbid"`，只供 `tools/` 下的 crate 使用，或作为 dev-dependency 引用；
`crates/` 下的生产依赖引用时 `check-wire-audit` 失败。

全部接口都可能分配内存、枚举进程或启动外部命令，只能在计时区间外调用。帧间高频
计数器由 [`laneflow-os-probe`][os-probe] 提供。

## 环境记录

`environment::collect` 采集一份 `laneflow-environment-record` 第 1 版记录：

- 自动字段：OS、CPU、物理内存、交流电与电池（经 os-probe）、Windows 电源计划
  （`powercfg /getactivescheme`）、Linux 调频策略与 `platform_profile`、BIOS（Windows
  读注册表 `HKLM\HARDWARE\DESCRIPTION\System\BIOS`，Linux 读 `/sys/class/dmi/id`）、
  工具链（`rustc -Vv`、`cargo -V`，按 `rust-toolchain.toml` 钉住的 channel 选择）与
  源码状态（git）。取不到的字段写成
  `{"unavailable": "<原因>"}`，不填空串、零或省略。
- 执行者输入文件只含 `vendor_performance_mode` 与 `hardware_role`（`R0`、`P10`、`P100`、
  `O1`、`uncertified`），由 `ManualFields::from_json` 解析；空值、只含空白或占位值一律拒绝。
- 机器指纹按 `laneflow-p100-hardware-identity-v2` 由 SMBIOS 计算，只记录摘要；仅
  Windows 采集。
- 一份进程快照作为后台进程记录。

工具链 channel 经 `environment::pinned_toolchain` 取自当前 checkout 的
`rust-toolchain.toml`。`EnvironmentRecord::require_measurement_ready` 核对工具链
（选择须等于该钉版）、源码干净且提交匹配、必填字段，以及 P100 角色的指纹；`EnvironmentRecord::differences` 给出测量前后须一致而
不一致的字段组。

P100 参考机上的人工验证：

```text
cargo test -p laneflow-host-observe --lib -- --ignored reference_machine
```

## 进程快照

`snapshot::capture` 读取进程表两次，报告每个进程的区间 CPU 增量与驻留字节；不记录
命令行、完整路径、环境变量与用户名。

## 子进程观测

`child::ChildObserver` 接管标准库启动的子进程，范围为仅直接子进程或进程组。
`sample` 返回成员与观测器自身的读数；`summary` 在退出后给出退出状态、采样峰值与
采样 CPU 时间，后两者是下界；`terminate` 按 PID 与启动时刻终止进程组，观测器被
丢弃时执行同一流程。

[host-observe]: ../../docs/design/host-observe.md
[os-probe]: ../laneflow-os-probe/README.md
