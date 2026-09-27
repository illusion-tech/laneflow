# 低活动率、容量与占用索引成本

本研究对应 #682。正式基线为 `03da6f57d40dceef59c86e86d3532a28751e5b15`，
包含 #713。只新增研究工具、夹具和测试构建诊断，不改变正式 Runtime API、交通规则、
数据格式或资源权威。不运行候选优化；本项用归因证据选择后续研究或实施方向。

## 输入与完成边界

同一张受检 LFCA、同一串行 `TrafficWorld`，每组暖机 40 拍、观察 128 拍，每拍
100 ms。每组分别运行三次独立的正常库墙钟进程与测试库诊断进程。第二轮反转输入
顺序；所有轮次保留。单元测试诊断含既有计数器和分配器记账，不能与正常库延迟相减。

| 组 | Active | live | 槽位长度高水位 | 配置容量 | 静态边数 |
| --- | ---: | ---: | ---: | ---: | ---: |
| compact | 1000 | 1000 | 1000 | 10000 | 256 |
| vacant | 1000 | 1000 | 10000 | 10000 | 256 |
| parked | 1000 | 10000 | 10000 | 10000 | 256 |
| capacity | 1000 | 1000 | 1000 | 100000 | 256 |
| active | 10000 | 10000 | 10000 | 10000 | 256 |
| edges-small | 1000 | 1000 | 1000 | 10000 | 16 |
| edges-large | 1000 | 1000 | 1000 | 10000 | 4096 |
| ring-10m | 64 | 64 | 64 | 10000 | 4096 |
| ring-1m | 64 | 64 | 64 | 10000 | 4096 |

前三组之间：compact → vacant 单独提高历史槽位长度；vacant → parked 保持槽位
长度、配置容量与 Active 不变，只把空槽变成 parked。capacity 只提高配置容量。
长边组均只使用 16 条 8 km 道路，每车初始零速、车头间距 10 m。环形组每环 64 边，
64 个独立环，每环一辆车；路线重复四圈，改变边长也会改变物理路网长度与前视访问数，
因此只能解释该组跨边工作量，不能归因为唯一一项成本。所有组无表现、聚合或降频。
输入通过编译器原生输入、可移植发射与后发射检查，编译使用
`CompileLimits::single_network_1m_v2`；编译/安装/暖机不计入 step 墙钟。

另对既有 Waiting 单车、Conflict 双车夹具各测三轮 16 拍，记录协调器候选排序、
cell 排序去重和公开决定排序的 calls/items/批次时间。这是很小的资源合同对照，
不能推断城市密集冲突或 P4 并行。热冷布局只报告实际类型尺寸与完整所有权内存账本；
没有 PMU cache-miss 证据就不宣称 cache miss，不从局部时间承诺整拍收益。

## 证据与边界

采集前要求 Git 干净且 HEAD 与已推送的 upstream 相同；记录 source/tree、
manifest/lock、rustc、二进制 SHA-256、进程 UUID、前后 HEAD/clean、日志摘要。
采集器拒绝其他 Cargo/rustc/link/cl、已知 Runtime/harness 测量进程。运行中每秒
检查已知竞争进程；短于采样间隔的任务仍可能漏检。先采集 30 个每秒 CPU 样本作为
本轮空闲基线，前后检查已知竞争进程；每次运行前后各记录两个 CPU 样本。
超过基线 p95（nearest rank）加 10 个百分点时只告警，不据此拒绝或认定干扰。
该告警尺度是透明的经验规则，不是校准出的性能合格线。完整三轮均值、p95 与
均值极差/中位数共同报告，不删除高 CPU 样本，也不以最低耗时代表性能。
这是开发机干扰筛查，不证明操作系统完全静默，也不锁频或绑定 CPU。
被拒绝的元数据和失败日志保留；发现竞争进程时停下并通知用户，再决定重测。
schema 1 的固定 20% 硬门禁记录保持原样，不重分类；schema 2 使用独立输出目录。
`accepted` 只表示输入完整、无已观察到的竞争进程和源码稳定，不能单独证明性能稳定。

正常构建输出全部 128 个逐拍时间；独立核验拒绝缺拍、重复、混合输入、截短窗口、
摘要不一致、日志改动、缺失诊断与重复已接受轮次。诊断的 Occupancy 四段嵌套于
Occupancy 阶段，容量清空嵌套于准备阶段；不能重复相加。

完整 retained-memory ledger 统计 owner 的 heap capacity，不是 RSS、峰值或实际
内存总线写入字节。分配流量和 retained bytes 分开。
`conflict_state_valid` 保持正式检查；失败重试和句柄代次另有正确性测试。

## 运行入口

从仓库根目录构建，先完成全部编译再计时：

`cargo test -p laneflow-runtime --release --test sparse_cost_evidence --no-run --locked`

`cargo test -p laneflow-runtime --release --lib --features placement-fixtures --no-run --locked`

`cargo build -p laneflow-sparse-cost-research --locked`

保存 Cargo 打印的两个测试二进制绝对路径，确认 source 已提交推送且工作树干净。
直接调用采集器，避免把正在运行的 Cargo 混入测量：

`laneflow-sparse-cost-research calibrate OUTPUT_DIR`

`laneflow-sparse-cost-research matrix WALL_EXE DIAGNOSTIC_EXE OUTPUT_DIR`

若中途有受干扰轮次，保留该记录；用单次入口补齐缺失的已接受轮次：

`laneflow-sparse-cost-research capture EXE MODE CASE ROUND OUTPUT_DIR`

`laneflow-sparse-cost-research verify OUTPUT_DIR RESULTS_JSON`

MODE 为 wall、diagnostic；补充资源对照使用 resource/resources/0。Windows 采集器
依赖系统 tasklist/typeperf，计数器不可用即拒绝；夹具、测试与核验器仍使用 Rust。
当前是有限研究窗口，不替代 #707、#537 或 #539 的产品认证。
