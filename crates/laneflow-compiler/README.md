# laneflow-compiler

LaneFlow 的生产编译器。它把受检官方来源编译成只读的已验证规范低层中间表示，并可以发射尚未受信的可移植候选。

## 职责与边界

编译器是准备静态资产的上游，不是交通运行时内部的加载器。`TrafficWorld` 不解析编制输入，也不直接安装 LFCA 字节。

本 crate 负责受检来源构造、确定性编译、结构化诊断和资源限制。成功时同时给出 `ValidatedCanonicalLir` 与配对的来源映射输入；任一阶段失败只返回结构化诊断，不留下半份结果。`emit_portable_candidate` 在此之上原子借出同一份编译输出，得到 LFCA / LFSM / LFSD 候选。

候选仍然未受信：它不授予发布、迁移或运行时加载权限。后发射检查通过后，才可以构造 LFCP，或交给 `laneflow-static-network` 构建 `SharedNetworkRevision`。路线不写入静态制品，由每个世界的 `TrafficWorld::register_route` 注册。

本 crate 不依赖交通运行时、Spatial、引擎适配器或文件系统发布事务。

## 最小使用入口

无窗口交通接入从已经检入的受检 LFCA 开始，不在运行时里调用编译器。见[从源码接入无窗口交通运行时](../../docs/guides/source-integration.md)。

需要自己编制路网时，从 `Compiler` 的原子编译入口进入，再用 `emit_portable_candidate` 取出可移植候选。官方前端和道路编辑来源都走这条编译器私有接入路径，不另建一套语义后端。

## 主要约束与限制

- 现行可移植对象是 LFCA 5 / LFSM 4 / LFSD 4；LFCP 2 在后发射检查之后构造。道路编辑来源有自己的版本轴。
- `FormatLimits` 与编译资源上限都是调用方预算。`FormatLimits::HARD` 是仓库示例使用的格式天花板，不是任意不受信路网的安全预算。
- 编译成功不会自行启用运行时冲突执行，也不把出行需求或路线选择策略放进编译器。
- 当前只覆盖已经接入的道路机动车静态子集。没有声明中心线时仍可形成无图形载荷的合法 LIR；这和运行时无窗口接入不是同一件事。

算法、字段和失败分类以专题合同为准，不在这里复述。

## 示例与验证

检入走廊和复杂路口的 LFCA 由对应生成器产出，运行时示例直接消费制品。编译器自身的确定性与诊断由本 crate 的测试覆盖。

```powershell
cargo +1.99.1 test --locked -p laneflow-compiler
```

## 深入阅读

- [从源码接入无窗口交通运行时](../../docs/guides/source-integration.md)
- [网络编译器](../../docs/design/network-compiler.md)
- [编译器基础](../../docs/design/compiler-foundation.md)
- [可移植规范制品](../../docs/design/portable-canonical-artifact.md)
- [后发射检查](../../docs/design/compiler-post-emission-check-and-minimal-publication-closure.md)
- [当前能力与限制 · 编译与加载](../../docs/reference/capabilities.md#编译与加载)
