# laneflow-static-network

`laneflow-static-network` 把受检 LFCA 建成进程内、不可变、可由多个世界共享的 `SharedNetworkRevision`。

## 职责与边界

构建输入只有 `CheckedCanonicalNetworkInput`。成功后的修订拥有 Traffic、Identity、Planning Hints、Conflict、Policy，以及按调用选项保留的 Spatial。调用方保存 `Arc<SharedNetworkRevision>`。修订不借用 LFCA 字节。

本 crate 不定义静态镜像文件、ABI、缓存或存档，不依赖编译器、交通运行时、Adapter 或文件系统。最终 worker 数、世界种子和运行时快照都不写入共享修订。

## 最小使用入口

```text
check_canonical_network_input → build_shared_network_revision → Arc<SharedNetworkRevision>
```

无窗口示例使用 `SpatialBuildOption::Omit`，再把得到的修订交给 `TrafficWorld::install`。完整命令和实跑输出见[从源码接入无窗口交通运行时](../../docs/guides/source-integration.md)。

## 主要约束与限制

- `SpatialBuildOption::Omit` 仍检查几何连通，但不保留 Spatial 载荷。`RetainAvailable` 在 LFCA 带有空间载荷时把它留在修订里。没有 Spatial component 时，`SpatialSession::bind` 返回 `Ok(None)`。
- 保留字节、暂存字节和策略工作预算都是调用方上限。超限失败，不产出修订。
- `attestation-fixtures` 默认关闭。`laneflow-runtime` 的 dev-dependencies 为其他测试打开它；普通宿主构建不要启用。该 feature 不改写在用修订。无窗口示例不调用这些测试入口。
- 构建成功不表示宿主已经信任这份发布，也不代替运行时安装时的来源修订核对。

## 示例与验证

走廊无窗口示例会构建这份检入 LFCA 并确认省略 Spatial 后没有 Spatial component。

```powershell
cargo +1.99.1 run --locked -p laneflow-runtime --example headless
cargo +1.99.1 test --locked -p laneflow-static-network
```

## 深入阅读

- [从源码接入无窗口交通运行时](../../docs/guides/source-integration.md)
- [共享静态路网](../../docs/design/shared-static-network.md)
- [交通运行时共享消费](../../docs/design/traffic-runtime-shared-consumption.md)
- [当前能力与限制 · 编译与加载](../../docs/reference/capabilities.md#编译与加载)
