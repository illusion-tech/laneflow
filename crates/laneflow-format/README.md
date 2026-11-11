# laneflow-format

LaneFlow 可移植对象的线格式 crate。它检查并编码 `LFCA`、`LFSM`、`LFSD` 和 `LFCP`，不拥有编译器语义或交通运行时。

## 职责与边界

当前只承认 LFCA / LFSM / LFSD 组合 `5/4/4`，以及 LFCP 2。`check_canonical_network_input` 对一份已经通过宿主准入的 LFCA 重算 digest、精确长度和 `NetworkRevisionId`，得到 `CheckedCanonicalNetworkInput`。这是 `laneflow-static-network` 的唯一构建输入。

检查分层是：对象框架、附录登记形状、不依赖外部对象的直接值域，然后才是上述受检路网输入。更早的视图不证明跨表引用、语义差异完备性或发布真实性。共享修订建成之后不再借用输入字节。

本 crate 不做完整路网语义闭包，不构造 `TrafficWorld` 或 `SpatialSession`，也不判定宿主是否信任一份发布。

## 最小使用入口

运行时接入消费的是受检 LFCA，不是原始文件扩展名。无窗口示例的调用顺序见[从源码接入无窗口交通运行时](../../docs/guides/source-integration.md)。

只持有最终 LFCA 字节时，调用 `check_canonical_network_input`。同时持有 LFCA / LFSM / LFSD 时，用 `check_post_emission_bundle` 闭合 digest、长度、修订和跨对象绑定，再视需要取出其中的受检 LFCA。

## 主要约束与限制

- `FormatLimits::HARD` 是格式安全天花板，也是仓库示例的用法。它不是推荐给任意不受信路网的安全预算；调用方只能收紧，不能放大天花板。
- 发布来源安装到 `TrafficWorld` 时，严格拒绝条件是 `NetworkRevisionId` 不一致。digest / length 差异只用于来源审计。这个判据在运行时，不在本 crate 的线格式检查里。
- 写入接口保证框架、登记形状和通用数值约束。跨表闭包和跨对象绑定仍由发射端的已验证输入以及后继检查负责。
- 本 crate 不读取文件系统，也不把线格式对象当成存档或共享修订。

## 示例与验证

无窗口示例用 `FormatLimits::HARD` 检查检入的走廊 LFCA。线格式自身的向量和拒绝面在本 crate 的测试里。

```powershell
cargo +1.99.1 test --locked -p laneflow-format
cargo +1.99.1 run --locked -p laneflow-runtime --example headless
```

## 深入阅读

- [从源码接入无窗口交通运行时](../../docs/guides/source-integration.md)
- [可移植规范制品](../../docs/design/portable-canonical-artifact.md)
- [后发射检查](../../docs/design/compiler-post-emission-check-and-minimal-publication-closure.md)
- [当前能力与限制 · 编译与加载](../../docs/reference/capabilities.md#编译与加载)
