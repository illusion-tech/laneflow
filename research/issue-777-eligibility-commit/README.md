# Conflict 资格表提交成本

Refs #777。起点为 `72102fabb2cda5ebadde52dc1260c8402762efda`。先细分复制、
全空扫描和准备清空，再验证全空时避免复制的最小候选。沿用阶段协议的 P7 原子
提交、完整资格谓词和正式 `conflict_state_valid`；不引入触及列表、独立尝试代次、
新 API 或格式。后续是否扩大范围由整拍证据决定。

已完成 72 次正常库整拍和 36 次独立诊断，保留空表跳过复制的最小实现。
见 [结论与边界](results.md)、[完整数表](measurements.md) 和
[封存核验结果](evidence/results.json)。原始记录包含全部告警和拒绝数据。

## 输入和观测

复用 #682 的 compact、capacity、active、parked 四组：固定道路及车辆状态，分别
验证普通容量、大容量低活动率、高活动率和大量 parked。各组暖机 40 拍、观察
128 拍、dt=100 ms、workers=1。

真实资源组使用既有 full-spatial LFCA 的两辆 Conflict 车辆，分别配置容量 8 和
100000，dt=4 ms。每个进程执行八个独立相同的窗口，每窗暖机 9 拍、观察 16 拍，
合计 128 个逐拍时间；必须仍有真实 reservation、NoGrant 和非空资格。
该组验证非空资格路径，不代表城市密集争用。

测试诊断另外覆盖资格表首位、末位、密集和全空的合成槽位；只验证表级操作与
位置/密度边界，不冒充合法交通世界或生产整拍收益。

每臂每组有六次正常库墙钟进程，按三块 ABBA / BAAB / ABBA 顺序采集，第二块
反转组序；每块两臂各一次独立诊断，共 72 次墙钟、36 次诊断。诊断与正常库
二进制分别构建，生产库无 cfg(test) 计时钩子。placement-fixtures 仅用于真实资源
初始摆放，观察窗口全部走正常 step；不在两臂之间切换 feature。

诊断记录完整 Commit、资格提交、复制、全空扫描、motion/eligibility 清空；父子
计时不可相加。scan items 表示传入长度上界，`all`/`any` 遇到非空会提前停止，
不能把长度当成实际逐元素访问数。每拍资格密度扫描在被测 step 之外，仍可能
影响缓存，因此独立诊断绝对耗时不与正常库相减。

## 复测

扩展现有 Rust 采集器 `laneflow-sparse-cost-research eligibility`，复用 #682 的
CPU 基线告警和竞争进程检查。编译完成后才运行测量，所有轮次保留；发现其他
编译或测量任务时停止。CPU 超过本轮空闲 p95 加 10 个百分点仅告警。

在干净、已推送的诊断基线提交构建 A：

`laneflow-sparse-cost-research eligibility build base target/777/base`

在只改变资格提交逻辑的干净、已推送候选提交构建 B：

`laneflow-sparse-cost-research eligibility build candidate target/777/candidate`

`laneflow-sparse-cost-research eligibility calibrate target/777/runs`

`laneflow-sparse-cost-research eligibility measure target/777/base/build.json target/777/candidate/build.json target/777/runs`

`laneflow-sparse-cost-research eligibility verify target/777/runs target/777/results.json`

构建命令保留独立二进制副本、源码/tree、Cargo 参数、manifest/lock、rustc 与二进制
SHA-256。采集元数据区分被测二进制源码与采集器所在提交，不以当前 HEAD 冒充
基线二进制源码。中途拒绝后可重跑 measure，跳过已有同身份接受记录，保留拒绝记录；
独立核验拒绝重复、缺轮、错误配对、日志变更和状态摘要不一致。

本轮冻结 A=`c0d8cc0b056c3daaf5a03d8e6acdc392517acdd7`、
B=`00df1af1031e25b8206ab1e40e2178fb6ffefc1d`。构建/采集使用 Windows
tasklist/typeperf；离线核验只需要 Rust 工具、Git 历史及封存文件。无需重新编译
被测二进制即可核验：

`cargo run --locked -p laneflow-sparse-cost-research -- eligibility verify research/issue-777-eligibility-commit/evidence/runs target/777-reverified.json`

最终入口从 `eligibility_commit/wall.rs` 移到 `eligibility_commit_evidence.rs`，
使共享夹具引用不含 `..`，符合 wire 审计。该修复只改测试入口位置、模块路径和
Cargo test path，未改生产逻辑或输入/时钟；历史测量仍准确绑定上述 A/B 提交，
不把后续文档、归档或入口整理后的 HEAD 冒充被测源码。

资格表分支不会自行缩减已预留容量，不能由整拍变快宣称等比例内存减少。最终
取舍以三块正常库配对结果、非空路径退化及正确性为准，不预设固定收益百分比。
