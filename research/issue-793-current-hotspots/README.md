# 当前主干热点短窗复测

关联 #793，属于 #707 的下一候选筛选。正式运行时代码不变。

- [实测结果与后续候选](results.md)
- [首批 12 次完整记录](evidence/results.json)
- [单独复跑 100k 的 6 次记录](evidence/rerun100k.json)

## 冻结协议

- Runtime 基线：`fadf6a844d9d22869976089f866c364edc6fc261`。
- Rust/Cargo 1.98.0，release、locked、offline，`CARGO_INCREMENTAL=0`；两种
  EXE 在采集前全部构建，构建目录位于导出源码之外。
- 输入：#707 `4de40e04` 的 MIXED-PEAK 输入及 smoke 计划，seed 544，workers 4；
  10k / 100k 分别使用原计划的 16 / 33 ms 步长，每进程 256 拍，无暖机剔除。
- 每规模次序为 plain、detail、detail、plain、plain、detail，共 12 个独立串行
  进程。按全窗 1–256、入口 1–64、筛选窗 65–256 分列；后者不是稳态认证。
- plain 仅在宿主已有整拍计时之后输出记录；detail 仅在导出的源码启用阶段墙钟。
  不插入工作线程逐车时钟，不累加线程耗时。
- 复用 #762/#768 的采集与分析，保留历史协议固定基线。
  新入口位于 `research/issue-762-post-p3-hotspots/src/current_cost.rs`，协议为
  `current-hotspots-v1`。

## 阶段边界

| 阶段 | 含义 |
| --- | --- |
| P5Setup / P5Slots | 当前 Active 数量读取 / 结果槽位清理、预留、初始化 |
| P5Dispatch | `try_for_each_chunk` 的协调器墙钟，包含调度、计算和完整 join，不能进一步分解三者 |
| P5Consume / P5Fused | 完整 join 后规范消费 / 融合回退路径 |
| WaitingPreview / WaitingAssembly | P2 分发或融合预览及规范消费 / 后续 Waiting 组装 |
| WaitingClear | P2 的 `waiting_plan_by_vehicle.fill(None)`，不含失败回滚 |
| ConflictMotionClear / EligibilityClear | P3 准备阶段两份稀疏表全量清空 |
| P7Eligibility / P7Scan / P7Copy | 资格表提交总耗时 / 全空检测 / 非空表复制 |
| P5Active / EligibilityLen / EligibilityEmpty | 进入 P5 的 Active 数量 / 资格表长度 / 全空布尔值；计数槽不带时间 |

P7Scan 的 `.all()` 可以提前终止；EligibilityLen 是表长，不能解释为实际扫描次数。
P5Setup 极短，不用于细粒度优化推断。所有子阶段必须嵌套于相应父阶段，plain 必须
没有阶段时钟；本协议只接受此城市样本实际走的 P5 分发路径。失败、回退或不完整
采集会被拒绝，不将部分成功样本冒充完整矩阵。

## 入口

构建 `cargo +1.98.0 build -p laneflow-post-p3-research --bin laneflow-current-hotspots-research --locked --offline`。

工具参数：

- `prepare plain <build-root>`、`prepare detail <build-root>`：从冻结基线导出，
  锚点不唯一或输出已存在即失败；保存完整源码 SHA-256 索引。
- 在每个 `<mode>-source` 上构建 `laneflow-urban-harness`，使用源树外独立 target
  目录；将 EXE 放为 `<build-root>/plain.exe`、`detail.exe`（Unix 无扩展名）。
- `run <build-root> <frozen-input-root> <new-raw>`：要求采集器 Git 树干净；保存
  UUID、命令、退出码、前后 Git 状态、二进制哈希、输入与源码索引、原始输出。
- `run-100k <build-root> <frozen-input-root> <new-raw>`：只复跑 100k 的 6 个
  进程，身份记录明确保存 `matrix_scale=100k`，不拼接旧 10k 冒充新矩阵。
  Windows 下在每个进程前后通过 `tasklist` 保存快照；发现已知编译或测量进程
  即停止，保留不完整目录。边界快照不等于连续监测。
- `analyze <raw> <results.json>`：核验原生摘要、逐拍序列、时钟/计数、交通文件
  字节一致性、整拍 p95，输出全量文件索引。结果必须位于 raw 之外。
- `verify <raw> <results.json>`：重新派生并逐值比较，不覆盖现有证据。

首批 12 次记录见 [results.json](evidence/results.json)。其结束快照发现
`cargo.exe` 与 `rustc.exe`，全部样本保留，明确不接受为稳定性能基线。
维护者随后要求单独重跑 100k；复跑沿用相同 Runtime EXE 与输入，另存证据。

采集前后另存环境、已知编译/测量进程和电源快照；这不构成全程温度、频率或背景
进程锁定。宿主扰动和诊断偏差保留在结果中，不通过剔除或选择重复掩盖。

## 验收边界

交付一个有证据支持的下一候选及其正确性验证边界；无法细分时明确剩余问题。
普通短窗反映当前样本的整拍表现，诊断窗只辅助归因，不是 A/B 优化收益；不与旧
批次直接相减。100k 为总车辆数，实际 Active 单独列出。无 44,544 拍或全天正式
认证要求，本切片不关闭 #707，也不声称交通质量最终认证。
