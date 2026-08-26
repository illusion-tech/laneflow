# LaneFlow Signalized Corridor Generator

本工具提供受保护左转、直行和右转走廊的可复现离线生成路径。它读取仓库内部 TOML 配置，从几何
建造填充 compiler 合成来源模块，写出 scenario-local catalog 0.3 TOML 与可 `install` 的
LFCA（含 Spatial）。不写出 current JSON。50–200 确定性回流见
[#475](https://github.com/illusion-tech/laneflow/issues/475)。

## 使用

从仓库根目录生成 checked-in 默认制品：

```powershell
cargo +1.96.0 run --locked -p laneflow-corridor-generator -- generate --config examples/config/v0.10-signalized-corridor.toml
```

只检查当前制品是否与配置逐字节一致：

```powershell
cargo +1.96.0 run --locked -p laneflow-corridor-generator -- check --config examples/config/v0.10-signalized-corridor.toml
```

`check` 不写文件。两个命令比较 catalog 与 LFCA 字节，并做 catalog cross-reference 校验。

## 依赖与分发

- `toml 1.1.4+spec-1.1.0` 只解析/序列化仓库内部配置与 catalog，许可证为 MIT OR Apache-2.0，MSRV 低于 workspace 1.96。
- 工具离线运行，不进入 Runtime fixed-step 或 Adapter 热路径，不引入网络、引擎或 copyleft 依赖。

## 边界

- lane count 固定为主路双向六车道、两条次路各双向四车道。
- 显式生成共享 road edge、32 条 ManeuverPath/internal edge/Gate；28 条路线写入
  catalog 边键，不写进 LFCA。不从 connector 名称或 geometry 反推 owner。
- catalog 0.3 由 Portal 拥有 ordered PortalLane，PortalLane 拥有共享 entry
  SpawnSlot 与 weighted RouteChoice。
- `vehicles`、`seed`、回流策略、Bevy Entity 和展示资源不属于本工具配置。
- 工具不进入 Runtime fixed-step 热路径。可运行世界只从检入的 LFCA 安装共享路网修订；
  catalog 字符串在 prepare 用 Identity v1 绑到该修订，不查 LIR。
