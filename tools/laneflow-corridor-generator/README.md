# LaneFlow Signalized Corridor Generator

本工具为 #188 提供 v0.8 直行信号化走廊的可复现 authoring 路径。它读取仓库内部 TOML 配置，生成并校验：

- Traffic package v0.7 JSON；
- SpatialPackage v0.1 JSON；
- ScenarioManifest v0.1 JSON；
- scenario-local startup catalog TOML。

Traffic、Spatial 和 Manifest 是 production interchange 制品；catalog 只用于 #203/#189 的仓库内部启动路径，不进入 Manifest。

## 使用

从仓库根目录生成 checked-in 默认制品：

```powershell
cargo +1.96.0 run --locked -p laneflow-corridor-generator -- generate --config examples/config/v0.8-signalized-corridor.toml
```

只检查当前制品是否与配置逐字节一致：

```powershell
cargo +1.96.0 run --locked -p laneflow-corridor-generator -- check --config examples/config/v0.8-signalized-corridor.toml
```

`check` 不写文件。两个命令都会执行 JSON Schema、production loader、Manifest size/SHA-256、Spatial length/join 和 catalog cross-reference 校验。

## 依赖与分发

- `toml 1.1.3+spec-1.1.0` 只解析/序列化仓库内部配置与 catalog，许可证为 MIT OR Apache-2.0，MSRV 低于 workspace 1.96。
- `jsonschema 0.48.1` 沿用 workspace 已锁定版本，用于写盘前校验三个 production JSON 文档；其 `borrow-or-share 0.2.4` 传递依赖采用 MIT-0，`deny.toml` 只为该精确 crate/version 设置例外。
- 工具离线运行，不进入 Core fixed-step 或 Adapter 热路径，不引入网络、引擎或 copyleft 依赖。

## 边界

- lane count 固定为主路双向六车道、两条次路各双向四车道。
- 默认只生成直行 connector/route，但 ID 和 catalog 使用显式 movement/route 数组，不假设唯一 outgoing。
- `vehicles`、`seed`、回流策略、Bevy Entity 和展示资源不属于本工具配置。
- 工具不进入 Core fixed-step 热路径，不改变 Core/Data/Spatial/Adapter public API。
