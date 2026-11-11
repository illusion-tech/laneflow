# laneflow-static-contract

编译器、线格式和交通运行时共用的静态值类型。这是 `no_std`、无分配、不依赖具体编译器的叶子 crate。

## 职责与边界

本 crate 提供封闭的标识、序号、格式常量和已经接受的数值边界，例如 `StableId128`、有类型稳定标识、有类型序号、`NetworkRevisionId`、`Sha256Digest` 和 `ExactByteLength`。Identity v1 的种类与字段标签、LFCA 5 / LFSM 4 / LFSD 4 / LFCP 2 的 magic 和版本也在这里。

它不构造规范字节、不派生 BLAKE3、不解析来源、不做完整语义校验，也不持有运行时状态。路权策略类型只登记形状，不执行通行裁决。

## 最小使用入口

宿主通常不直接组装这些值，而是从受检 LFCA 或 `TrafficWorld` 读出它们。无窗口示例比较的 `NetworkRevisionId` 就来自共享修订 origin。见[从源码接入无窗口交通运行时](../../docs/guides/source-integration.md)。

## 主要约束与限制

- 有类型序号只用于已经验证的致密表，不能当成跨修订的稳定身份。
- 本叶子 crate 的登记不等于来源编制、共享修订或运行时已经实现对应行为。
- 精确字节长度和摘要类型不自行证明发布真实性。发布来源安装时，运行时严格核对的是 `NetworkRevisionId`；digest / length 用于来源审计。

## 示例与验证

```powershell
cargo +1.99.1 test --locked -p laneflow-static-contract
```

## 深入阅读

- [从源码接入无窗口交通运行时](../../docs/guides/source-integration.md)
- [可移植规范制品](../../docs/design/portable-canonical-artifact.md)
- [共享静态路网](../../docs/design/shared-static-network.md)
- [当前能力与限制 · 编译与加载](../../docs/reference/capabilities.md#编译与加载)
