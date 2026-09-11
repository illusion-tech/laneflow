# LaneFlow Complex Junction Generator

本工具提供复杂路口参考场景（#285 阶段二）的可复现离线生成路径。它读取仓库内部 TOML
配置，经 RoadEditing 编制单四岔信号路口，写出 scenario-local catalog 0.1 TOML 与可
`install` 的 LFCA（含 Spatial）。不写出 current JSON。

## 使用

从仓库根目录生成 checked-in 默认制品：

```powershell
cargo +1.98.0 run --locked -p laneflow-junction-generator -- generate --config examples/config/v0.1-complex-junction.toml
```

只检查当前制品是否与配置逐字节一致：

```powershell
cargo +1.98.0 run --locked -p laneflow-junction-generator -- check --config examples/config/v0.1-complex-junction.toml
```

`check` 不写文件。两个命令比较 catalog 与 LFCA 字节，并做 catalog cross-reference 校验。

## 场景合同

- 单四岔信号路口：主路东西向 2+2 车道，次路南北向 1+1 车道；臂长、路口半径、车道
  宽度等几何参数全部来自配置文件。
- 7 条 Movement / 9 条 ManeuverPath：主路直行每车道一条车道级路径、主路西进口保护
  左转（带 12 m 待转 pocket，三门：admission / waiting-entry / release）、次路北进口
  许可左转、次路南进口直行、两条右转。
- 固定时制信号程序 9 相位 4 组：主路左转独占保护；主路直行与次路许可左转同相位
  （许可左转必须在主路直行车流中找间隙）；次路直行独占。相位时长来自配置。
- 冲突区只为「许可门路径 × 同相位并发直行路径」的几何交叉对编制：同入口边跳过，
  同出口边取许可路径末点作合流区；信号分离的方向对不占冲突区。默认配置产出 3 个
  ConflictZone（许可左转 × 两条东→西直行车道的交叉区、许可左转 × 西→东 lane0 的
  合流区）与 4 条 ParticipantStream（许可流 priority 0 + 三条直行流 priority 100）。
- 1 个 WaitingZone（容量 1，挂在保护左转路径上），8 条环路回连边把每条出口车道
  一对一接到顺时针下一条入口车道（三段 90 度圆弧绕角 + 直线段），让 10 条 catalog
  路线中的两条焦点路线成环并多次穿过同一机动门（重复 Gate occurrence）。
- catalog 0.1 由 Portal 拥有 ordered PortalLane（每 portal 2 条回连边车道），
  PortalLane 拥有共享 entry SpawnSlot 与 weighted RouteChoice；280 个 spawn slot
  只放在环路回连边上，端点净距 = 车长 + min_gap。
- 策略：单一 RightOfWayPolicySet，regulation `engineering` / `complex-junction-1`，
  依据 `repository:tools/laneflow-junction-generator`；示例声明是工程验证场景模板，
  不宣称覆盖现实法域法规全集。

## 确定性约束

- 编制只用 IEEE 精确运算（加减乘除、`sqrt`、硬编码常量），禁止三角函数与
  `hypot`；两次 `generate` 必须产出逐字节相同的 catalog、LFCA 与 portable sidecars。
- 编译/发射用 `CompileLimits::single_network_1m_v2`（声明规模超出 p100 暂存上限）；
  StableId 按内容派生与限制档位无关，catalog policy pin 与 binder 侧身份派生固定用
  p100（身份键长上限 53 字节即身份上限），两侧结果一致。
- 环路回连边与道路边一样由 alignment → corridor → section → lane 链派生几何
  （canonical frame 来源）；junction internal 边保持裸 LaneEdge + 内联几何，且按
  编译器规则不携带 LaneEdge 后继——路径内的转移只由 ManeuverPath 边序承载。

## 依赖与分发

- `toml 1.1.3+spec-1.1.0` 只解析/序列化仓库内部配置与 catalog，许可证为 MIT OR
  Apache-2.0，MSRV 低于 workspace 1.98。
- 工具离线运行，不进入 Runtime fixed-step 或 Adapter 热路径，不引入网络、引擎或
  copyleft 依赖。

## 边界

- `vehicles`、`seed`、回流策略、Bevy Entity 和展示资源不属于本工具配置。
- 工具不进入 Runtime fixed-step 热路径。可运行世界只从检入的 LFCA 安装共享路网修订；
  catalog 字符串在 bind 用 Identity v1 绑到该修订，不查 LIR。
- `policy_selection` 是 catalog 顶层必填的闭合选择；本场景带 Gate/ConflictZone/
  ParticipantStream，`not_required` 会被绑定阶段拒绝。调用方把 bind 结果的
  `policy_selection` 传入唯一世界安装入口。
