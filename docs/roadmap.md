# 路线图

**文档状态**: Active（版本能力边界；实施状态在 GitHub）<br>
**最后更新**: 2026-11-11
**适用范围**: LaneFlow 已完成版本的能力边界、v0.10 验收证据范围，以及已裁定的后续版本目标

本文记录版本能力边界和已验收结论。现行系统见
[架构](architecture.md)。GitHub Milestone 是实施状态入口；本文不镜像 Project
列、未决 Issue、责任人或依赖。

[ADR 0021](adr/0021-traffic-infrastructure-and-host-boundary.md) 定义可嵌入、
引擎无关、确定性的道路交通运行时与工具链。城市经济、出行需求、土地利用和游戏
规则属于宿主。下表的完成不表示对应历史实现仍是现行入口。

## 已完成版本

v0.2–v0.6、v0.8 和 v0.9 的完成时间沿用本文改写前已记录的日期。v0.1 与 v0.7
原文没有日期，取对应里程碑的关闭日（UTC）：2026-07-12 与 2026-07-21。当时的
loader、schema 和实施链留在 git 历史与所链设计中。

| 版本 | 完成时间   | 能力边界                                             | 现行设计                                                                                                                                          |
| ---- | ---------- | ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| v0.1 | 2026-07-12 | 最小车辆状态、显式步进、基本车道图遍历与路线跟随     | [车道图](design/lane-graph.md)、[路线](design/route-system.md)                                                                                    |
| v0.2 | 2026-07-12 | 车道连接、路线定义与路线校验                         | [车道图](design/lane-graph.md)、[路线](design/route-system.md)                                                                                    |
| v0.3 | 2026-07-14 | 前车避让、纵向舒适控制与安全投影                     | [跟车](design/vehicle-following.md)、[ADR 0006](adr/0006-vehicle-following-control-and-safety.md)                                                 |
| v0.4 | 2026-07-15 | 固定时制信号、指示与 Gate 合规                       | [信号](design/signal-system.md)、[ADR 0009](adr/0009-signal-indication-gate-and-policy-separation.md)                                             |
| v0.5 | 2026-07-17 | 停车位占用、绑定和与纵向约束的组合                   | [停车](design/parking-system.md)、[ADR 0010](adr/0010-parking-binding-and-vehicle-lifecycle-authority.md)                                         |
| v0.6 | 2026-07-21 | 整数毫米一维交通几何，以及引擎无关的空间帧与位姿采样 | [数值表示](design/numeric-representation.md)、[空间几何](design/spatial-geometry.md)、[ADR 0028](adr/0028-integer-millimeter-traffic-geometry.md) |
| v0.7 | 2026-07-21 | 首个参考适配器：把已提交交通状态映射到宿主实体与变换 | [Bevy 适配器](design/bevy-reference-adapter.md)、[Adapter API](design/adapter-api.md)                                                             |
| v0.8 | 2026-07-24 | 直行信号化走廊，以及调用方拥有的持续车流             | [示例场景](design/example-scenarios.md)、[ADR 0016](adr/0016-scenario-population-and-recycle-lifecycle-authority.md)                              |
| v0.9 | 2026-07-26 | 受保护左转、直行和右转的信号化走廊，以及路口静态身份 | [路口模型](design/road-junction-model.md)、[受保护转向](design/signalized-corridor-protected-turning.md)                                          |

横断面静态模型和 Waiting / Conflict 已进入现行能力。`RoadCorridor` 组织方向性
路段；静态准入在车辆生成和替换时按参与者类别检查路线后缀。Waiting、Conflict 与显式路权由
`TrafficWorld` 在固定步进中裁决。时变准入仍是已接受但未实现的运行时求值。见
[横断面与准入](design/cross-section-access.md)、
[Waiting / Conflict](design/waiting-zone-conflict-right-of-way.md) 与
[路权策略](design/traffic-runtime-right-of-way-policy.md)。

## v0.10 编译器拥有的静态路网与切换

本版本的能力边界是：静态路网由编译器拥有，运行世界安装不可变的
`SharedNetworkRevision`，路网变化走失败关闭的修订切换。生产数据流、crate 依赖
和生命周期见 [架构](architecture.md)。专题合同见
[ADR 0020](adr/0020-compiler-owned-static-network-and-static-image.md)、
[ADR 0025](adr/0025-checked-canonical-network-and-shared-static-network.md)、
[网络编译器](design/network-compiler.md)、
[共享静态路网](design/shared-static-network.md) 与
[修订切换](design/traffic-runtime-revision-cutover.md)。

里程碑已于 2026-11-09 关闭。该日期只说明版本容器关闭，不是新的性能或 LuST
基线。

### v0.10 验收结论

#305 以验收提交 `c0045f8f`（#1282 冻结）运行统一五轴正式汇总，未完成项为零。
本节结论只绑定该提交，不是当前 `main` 的性能认证或 LuST 认证。
[汇总记录](https://github.com/illusion-tech/laneflow/issues/305#issuecomment-6678897695)、
[适用性核对](https://github.com/illusion-tech/laneflow/issues/305#issuecomment-6678716065)
与[裁定和更正](https://github.com/illusion-tech/laneflow/issues/305#issuecomment-6708446002)
是本结论的唯一事实来源，本节只作入口摘要：

- `LF-SYNTH-v2`：behavior、determinism、snapshot 为语义对照通过，routing 为执行回执
  通过（#1267、#1279）；
- 中国特色城市负载 `LF-CN-URBAN-v1`：behavior、determinism、snapshot 为执行回执通过
  （验收提交上重跑，覆盖范围以裁定和更正为准）；routing 经维护者授权豁免，城市负载的候选准入、墙钟与 tick
  间隔干扰未测；
- performance 轴两套负载均为“未认证”（`Product TBD / Uncertified`）：v0.10 不执行
  P100 产品认证，研究测量与已知契约级超标逐项登记，不记为通过，也不构成例外；
- resource 轴同为“未认证”（维护者 2026-11-07 裁定）：验收提交上只重跑了城市编译器
  受控峰值（低于 2 GiB，属资源上界计量，不是实际堆峰值）；#537 的内存与共存基线为
  历史结果，RSS 与容量没有测量面；不记为通过，也不构成例外；
- LuST 五轴已移至 v0.11，v0.10 没有真实路网证据；
- 维护者范围裁定中的授权豁免（worker 两两组合、其他失败点、Spatial/Adapter 跨 worker
  接线、跨修订/Editable 组合矩阵、非标称步长交叉组合、多 worker 十万整帧）随汇总记录
  逐项登记未覆盖范围；
- 能力限制如实记录，不记为通过：含活动过渡的世界拒绝保存（#1217，恢复矩阵随 #1200
  在 v0.11）；互不可见的共横向过渡段保守拒绝（#1220）；平行边直行车对 P4 不可见
  （#1271，L1 输入已核查不含该拓扑）；停止约束前的硬停（#1272）；
  `LF-COMP-P100-INITIAL-v1/v2` 单次发射最多 4,011 个稳定实体（#1288，目标设计 #1296）；
  设计 §7.1 的 Routing G2 case 在验收提交上不能复现（#1288）。

产品里程碑与 LFCA、LFRS 等数据格式版本相互独立；以上结论不改变任何格式版本。

## v0.11 任意位置变道与 LuST 真实路网验收

已裁定的能力目标是两项：任意位置变道，以及以 LuST 真实路网完成五轴验收。
固定预制过渡是已实现的首版换道，不是这项变道目标。v0.10 在 `c0045f8f` 上的
未认证项和 LuST 缺证也不因版本号前进而变成认证。

实施状态见里程碑
[v0.11 任意位置变道与 LuST 真实路网验收](https://github.com/illusion-tech/laneflow/milestone/14)。

## 城市级扩展研究

#72 是独立研究入口，不属于 v0.6–v0.11 的完成边界，也不因 v0.11 已裁定 LuST
验收而自动进入该版本。v0.6 的几何与交通参与单元空间分区是不同层次；v0.7 的
表现层细节层次与交通运行时保真度也不同。多世界共享静态路网可以验证内存复用，
不能代替单个大型城市世界的边界、负载和迁移证据。

#72 何时进入版本范围留待对应版本规划。未来稳定运行时 API 的设计冻结前，必须
解决或显式接受
[城市规模扩展决策基线](design/traffic-runtime-scale-decision-baseline.md)
中标为需要补证据和存在实际阻碍的接口项。该复核不表示已经选定生产架构。

后继城市级工作仍至少包括四个独立设计题目。道路机动车世界里已经实现的
worker 1–16、快照 v7、失败关闭切换和观测注册，只覆盖当前特化，不关闭这些
题目的城市级证据，也不把它们算进 v0.11：

1. 保持个体身份的单世界确定性并行执行。当前执行配置已实现，城市性能未认证。
2. 不可变路网修订、运行时快照、存档 / 回放与修订切换。快照 v7 与失败关闭
   切换已实现；活动过渡仍拒绝保存。
3. 路径规划服务、动态成本快照和出行编排接入。观测与候选注册已实现；路径
   算法由宿主拥有。v0.10 的城市 Routing 豁免仍然只绑定 `c0045f8f`。
4. 城市拓扑、需求与运行时工作负载 `LF-CN-URBAN-v1`。首版交付合同见
   [中国特色城市负载](design/chinese-style-city-workload.md)。公交运营、出租
   派单、上下客和动态路侧摩擦不属于该合同的本版交付。非机动车和步行不作为该
   负载的隐含验收项。

这些工作负载与 `LF-SYNTH-v2` 和 LuST 基线并列，不能静默替换既有 ID，也不能用
尚未支持的参与者伪装机动车。城市人口、乘客、出行需求与交通参与单元必须分层。
个体、活动、意图更新、表现、聚合记录和聚合等价计数必须按交通执行域分别报告。
“城市规模”不等于每个居民始终作为完整微观参与单元进入每个 tick。当前车辆证据
也不能代表非机动车、行人或轨道交通。

## v1.0

状态：待规划。产品目标、交付范围、完成定义，以及和城市级研究、外部宿主边界
证明、稳定性承诺的关系都未冻结。里程碑名称存在，不表示未决工作已经绑定到
v1.0，也不表示 v1.0 的范围已经进入 v0.11。范围须经后续治理决策和设计冻结
重新建立。
