+++
schema_version = 1
id = "design-lane-change-minimal-contract"
kind = "design"
status = "accepted"
revised_date = "2026-11-28"
decision_date = "2026-10-31"
summary = "规定同向相邻车道动态交接的让行、竞争、制动和清尾行为验收。"
scope = "道路机动车在登记窗口内的 R1a、R1b、R2、R3 和平行分岔验收，含生成负载、Routing 及宿主的跨层边界"
applicability = "#applicability"
issues = ["https://github.com/illusion-tech/laneflow/issues/237", "https://github.com/illusion-tech/laneflow/issues/1200", "https://github.com/illusion-tech/laneflow/issues/1271", "https://github.com/illusion-tech/laneflow/issues/1512", "https://github.com/illusion-tech/laneflow/issues/1526"]
related = ["design-cross-section-access", "design-cross-section-stationing-and-lateral-geometry", "design-road-junction-model", "design-route-system", "design-traffic-runtime-phase-protocol", "design-traffic-runtime-snapshot", "design-vehicle-following"]
+++

# 最小换道合同（Lane Change Minimal Contract）

<a id="applicability"></a>
## 适用边界与依据

道路机动车通过路线中登记的有向边出现对和走廊站位窗口换道。申请提交给固定步进，世界按整车轨迹、净空限速和两侧进入者裁决。已取得的空间保护持续到车尾清空。连接道、匝道和路口机动按其实际道路边、后继与机动门运行。[交接与占用设计][handoff]

## 必验交通行为

- **R1a，同速直行车优先。** 直行车以 10 米/秒从目标侧接近，距扫过区入口 8 米，无法在入口前安全停下。换道车等待，直行车照常通行。目标侧清空后换道车重新申请并完成交接。
- **R1b，两车竞争同一片路面。** 两辆车单独申请时均有安全轨迹。共同申请时按稳定车辆顺序授予，改变申请排列和工作线程数应得到同一结果。只有实际进入的车辆持有保护，另一辆在释放后重新申请。
- **R2，快速后车。** 后车以 13.9 米/秒接近，步长 100 毫秒，两拍反应时间、4.5 米/秒² 服务减速度及 2.5 米净距合计需要约 26.75 米。入口前 8 米时须拒绝，直行车通过后可以重试。入口前 35 米的对照须可准入，并能用常规制动完成后续各拍。准入证明与普通车运动均须为反应时间留出距离。
- **R3，长车清尾。** 12 米车的前杠交接后，源侧仍被车身占用。跟随车须遵守保护，车尾完全清空后释放。同修订恢复再继续步进，保护和释放时点应一致。
- **平行分岔。** 直行路线可以完全避开换道车路线上的前驱和后继。直行车已占目标侧扫过范围时，换道须等车尾驶离。换道车先获准时，直行车应在仍受保护的半开区间外制动，随后可从自己的分岔正常通过。[平行直行车反例][parallel]

四组行为通过编译路网和正式世界步进入口验收。平行场景另核对两条路线无公共边、车体见证、保护区间及双方最终通行。[四组行为回归][four]、[平行分岔回归][fork]

## 跨层验收

LF-SYNTH L1 由有向出现对和窗口发起实际申请，逐车核对进入、前杠交接和清尾，并验证同修订恢复与重放。Routing 候选注册和已准入路线重放应保留原窗口。宿主从同一提交拍提取车体参考姿态，覆盖前杠交接及车尾清空前后。[路线系统][route]、[快照与恢复][snapshot]、[宿主位姿][pose]

v0.10 的固定过渡负载和测量属于当时的实现。动态负载须绑定各自的输入、执行配置与运行记录。[历史验收范围][roadmap]

[handoff]: cross-section-stationing-and-lateral-geometry.md#41-从换道意图到一次路线交接
[parallel]: https://github.com/illusion-tech/laneflow/issues/1271
[four]: ../../crates/laneflow-runtime/tests/support/route_handoff/road_safety.rs
[fork]: ../../crates/laneflow-runtime/tests/support/route_handoff/parallel_entrant.rs
[route]: route-system.md
[snapshot]: traffic-runtime-snapshot.md
[pose]: cross-section-stationing-and-lateral-geometry.md
[roadmap]: ../roadmap.md
