# #543 LF-CN-URBAN-v1 容量调研（10k/100k exact-object 证据）

本目录保存 #543 调研 spike 的可执行证据生成器与结果。它是独立 Rust workspace，**故意不加入
主 workspace 成员**（#308 规则：研究代码不得进入生产 crate 或 workspace 成员），不是生产
包、公共 API 或已接受的数据格式。所有结论以本文件登记的假设为前提；正式口径以 #304 G1 与
后续实施 Issue 为准。

## 当前执行与历史证据边界

#284 的策略合同落地后，`run` 显式增加一个研究自有策略模块，导入全部拓扑模块，按生成器
声明的键覆盖全根 Gate / ParticipantStream，并以 `PolicyPin` 安装空世界。信号门使用
`ProtectedGroup`，无信号冲突门使用 `Uncontrolled`，两条冲突流使用明确的优先级且无
让行关系；这份 `engineering / capacity-install-v1` 策略只用于安装容量测量，不代表
中国道路规则或运行层容量。直行与左转来源同时明确填写 `turn_direction`。

下面的 100 / 1,000-cell 结果和 `evidence/` 文件保留为 **#284 前的历史证据**。新增
策略、导入边和方向字段改变了对象数量、源记录及准入计数；旧的零导入、12.5% 上限占比
及方案 A 结论不能直接作为当前树的容量证明。`model` 仅输出带 `lf543-historical-model`
前缀的旧解析公式，`run` 不再混入该公式。当前移植仅以 `run 10` 验证真实完整管线可执行，
不替代两档正式重算，也不覆盖或更新历史测量文件。

## 历史问题与结论

#543 要求回答：按 #304 G1 candidate rules 构造的 `LF-CN-URBAN-v1` 10k/100k 拓扑，走真实编译
管线后，落到哪个处置档——

- **A = 现有限制可用；B = 只需新 compiler profile；C = 确需新 LFCA format/version。**

**结论：方案 A。** 在现有具名生产配置档 `LF-COMP-SINGLE-NETWORK-1M-v2`（#315 系）下，
10k（100 cells / 10 tiles）与 100k（1,000 cells / 100 tiles）两档完整走通
"源构建 → 编译 → file-backed 发射 → post-emission check → SharedNetworkRevision build
（headless + spatial）→ TrafficWorld install" 全管线，**已测量维度**均处于上限的
12.5% 以内（最大为 stable entities 125,400 / 1,000,000）；HIR/MIR 维度无公开读数，
仅以 1M profile 编译成功证明低于上限，占比未测量（解析推导值另行标注）。
不需要新 compiler profile（排除 B），也不需要新 LFCA format/version（排除 C）。

同时实测确认：`LF-COMP-P100-INITIAL-v2`（#292/#315 冻结）**不适用**于该 workload——
10k 档在至少 7 个 admission 维度上超限（解析口径，见"P100 拒绝面"节）。这不改变 A 的判定，因为
1M v2 是既有具名生产配置档；但它提示 P100 的标定场景（#308 校准基线）与
LF-CN-URBAN-v1 的城市尺度不是同一档，文档中 "P10/P100" 性能分档与 compiler admission
维度不应混用。

## 口径与假设（显式登记）

1. **cell/tile 参数来自调用方口径**：250 m cell、2×5 macro-tile、整数铺设；
   10k = 100 cells / 10 tiles，100k = 1,000 cells / 100 tiles。当前 main 的
   `docs/design/chinese-style-city-workload.md`（232 行，#541 停车切片为最新改动）与
   #304 issue 正文/评论均**尚未写入**这些数值。#543 评论（wangzishi）给定格式基线
   LFCA 4 / Identity registry 3 / 33 逻辑表，重算触发条件 = #304 G1 变动或 #284 落地。
2. **每 cell 一个 4 引道信号化十字**（synthetic 前端）：20 LaneEdge（4×125 m 进口、
   4×95 m 共享出口、2×65 m 待转专用出口、10×30 m 内部边）、1 Junction、8 Movement、
   8 ManeuverPath、8 StopLine、8 SignalGroup、1 SignalController（4 相位 × 8 组状态）、
   12 ManeuverGate、2 WaitingZone（w/e 左转待转）、1 ParkingFacility（virtual 100 +
   1 进 1 出锚点）、4 显式路侧泊位、1 CanonicalFrame（20 边 × 2 整数值点）。
   每 tile 加 1 个 virtual-only 地下车库（capacity 1,000，2 进 2 出多门锚点）；
   每 synthetic 模块自带 ParticipantClass/VehicleProfile/AccessRule 三元组
   （避免跨模块 import；import_edge = 0）。
3. **停车声明容量是拓扑档参数，不等于 runtime 名义规模**：`C_parking_virtual_declared`
   = cells×100 + tiles×1,000（10k 档 20,000、100k 档 200,000，证据行 `lf543-parking`；
   设施数 cells+tiles、显式泊位 4×cells、virtual 锚点 2×cells+4×tiles）。按设计文档
   （`docs/design/chinese-style-city-workload.md`）§2 口径，该声明值不意味着等量
   `ParkingSpace`、LFCA 行、Runtime slot 或 presented entity；每 cell 100 / 每 tile
   1,000 是 spike 选取的拓扑档输入，不承诺等于 §3 的 runtime 个体/停车容量名义规模
   （10k/100k）。显式泊位 400 / 4,000 已作为 `ParkingSpace` 行计入 stable entities。
4. **冲突/无保护转向由 road-editing 前端承担**：synthetic 前端无 ConflictZone /
   ParticipantStream 声明面，故每 cell 另配 1 个冲突 tile（31 稳定实体：4 走廊/4 区段/
   4 车道/6 边/1 junction/1 ConflictZone+region/2 movement/2 path/2 stopline/2 gate/
   2 ParticipantStream），结构逐项照抄
   `crates/laneflow-compiler/tests/portable_emission_resources.rs` 的
   `build_conflict_module` 并加 per-cell 整数 offset。冲突 junction 与信号 junction
   分离是前端能力所限，计数偏保守高端。
5. **不跨 cell 接线**：exit 边 successors 为空。敏感性上界：若每 cell 4 条出口边各接
   相邻 cell，+8 successor references/relations/cell（100k 约 +8,000），对全部维度
   余量无影响。
6. **无 LaneGroup / FacilityBand**（两表 rows=0）：candidate rules 未要求；若 #304 后续
   引入，按其行增量线性外推即可。
7. **HIR/MIR 总计数无公开读数**：报告给出解析推导值，并用公开锚点交叉验证——LIR 公开
   metric（`CompilationMetrics::lir_record_count`）精确命中、LFCA 逐表行数、shared
   network `entity_counts()`、P100 探针失败点的精确 observed。HIR/MIR 计数公开化是
   独立 G1 事项，不影响 A/B/C 判定（1M profile 下编译成功本身即证明 HIR/MIR 维度通过）。
8. **几何全部为整数值坐标、轴对齐**，边长 == 几何弧长（125.0/95.0/65.0/30.0 精确）。
9. **堆峰值为 1ms 采样下界**（stats_alloc + 采样线程，与既有
   `portable_emission_resources.rs` 同模式）；`compiler_controlled_peak_bytes` 是编译器
   资源模型逻辑值，不是进程 RSS。

## 实测发现的两条几何约束（冒烟阶段触发，已修正并登记）

- **path 相邻边几何端点必须连续**（`InvalidSpatialGeometry/DiscontinuousJoin`）：
  待转 path 的第二段内部边终止于路口中心 +60 m，与共享出口边起点（+30 m）差 30 m 被拒；
  故 w/e 左转各配一条专用 65 m 出口边（每 cell 20 边而非 18 边）。
- **规范框架坐标界限 ±16,384.0 m/轴**（`InvalidSpatialGeometry/CoordinateOutOfRange`）：
  50 列 tile 布局（x 跨度 25,000 m）在 cell 321 处被拒（x=16,405 > 16,384）。
  100k 布局取 15 列 × 7 行 tile（30×35 cell，7,500×8,750 m）。该界限是几何验证规则，
  不是 profile/format 维度；LF-CN-URBAN-v1 的城市范围布局须满足此界限。

## 证据

原始输出（key=value 行）在 `evidence/`：

| 文件                                           | 内容                                    |
| ---------------------------------------------- | --------------------------------------- |
| `run-100-cells.txt`                            | 10k 全管线（1M v2）                     |
| `run-1000-cells.txt`                           | 100k 全管线（1M v2）                    |
| `parts-100-cells.txt` / `parts-1000-cells.txt` | synthetic-only / conflict-only 分解编译 |
| `probe-p100-synthetic.txt`                     | P100 v2 合成前端拒绝面                  |
| `probe-p100-conflict.txt`                      | P100 v2 冲突前端拒绝面                  |

### 端到端阶段（1M v2；release，Windows x64）

| 指标                                             |      10k（100 cells） |     100k（1,000 cells） |
| ------------------------------------------------ | --------------------: | ----------------------: |
| 稳定实体（shared `entity_counts()`）             |                12,540 |                 125,400 |
| `C_parking_virtual_declared`（声明虚拟容量）     |                20,000 |                 200,000 |
| LIR 逻辑记录（公开 metric）                      |                44,290 |                 442,900 |
| output_logical_bytes                             |             1,919,784 |              19,197,534 |
| compiler_controlled_peak_bytes                   |            55,102,839 |             550,938,489 |
| LFCA exact bytes                                 |             6,025,452 |              60,223,898 |
| LFSM / LFSD exact bytes                          | 6,158,094 / 5,998,848 | 61,565,234 / 59,979,132 |
| bundle exact bytes                               |            18,182,394 |             181,768,264 |
| source-build / compile 耗时                      |       0.36 s / 0.11 s |         26.9 s / 1.18 s |
| emit / post-check 耗时                           |      0.63 s / 0.098 s |         6.58 s / 0.93 s |
| shared retained（headless / spatial）            | 1,017,986 / 1,231,586 | 10,228,316 / 12,364,316 |
| shared 必需 scratch                              |               788,000 |              68,360,000 |
| TrafficWorld install live delta（空载 8 槽世界） |                23,825 |                 190,145 |

post-emission check 全程**零分配**（断言通过）。100k 的 LIR 恰为 10k 的 10 倍；
synthetic-only + conflict-only 分解编译在两档都精确可加
（10k：35,290 + 9,000 = 44,290；100k：352,900 + 90,000 = 442,900）。
`TrafficWorld install live delta` 一行以 `WorldConfig` vehicle_capacity = 8 的
**空载世界**（无车辆、无停车绑定）测量，仅覆盖静态网络安装/驻留分配，不含
`N_individual` 车辆状态与停车绑定；运行层随档缩放（按
`docs/design/chinese-style-city-workload.md` 的运行层档位定义）不在本 spike 证据内。

### 关键维度 vs 配置档上限（100k 档）

| 维度                       |                      100k 观测 |    1M v2 上限 |  占比 | P100 v2 上限 |
| -------------------------- | -----------------------------: | ------------: | ----: | -----------: |
| declarations               |                        125,400 |     1,500,000 |  8.4% |     11,265 ✗ |
| stable entities            |                        125,400 |     1,000,000 | 12.5% |     11,265 ✗ |
| LIR records                |                        442,900 |     8,000,000 |  5.5% |     38,112 ✗ |
| identity field occurrences |                        353,800 |     8,000,000 |  4.4% |     29,184 ✗ |
| relation occurrences       | ≈234,000（合成侧精确 212,700） |    16,000,000 | ≈1.5% |     10,032 ✗ |
| references                 | ≈218,000（合成侧精确 192,700） |    16,000,000 | ≈1.4% |     37,920 ✗ |
| typed AST records          |         ≥785,100（合成侧精确） |     8,000,000 | ≥9.8% |     58,387 ✗ |
| geometry points            |                         52,000 |    16,000,000 | 0.33% |     22,368 ✗ |
| maneuver gates             |                         14,000 |     1,000,000 |  1.4% |      2,304 ✗ |
| waiting zones              |                          2,000 |     1,000,000 |  0.2% |      1,536 ✗ |
| modules                    |                            101 |        65,536 | 0.15% |        522 ✓ |
| source bytes total         |  ≈29.1 MB（合成 18.0 MB 精确） |       512 MiB | ≈5.4% |  542,741 B ✗ |
| controlled live peak       |                    550,938,489 | 6,442,450,944 |  8.6% | 43,269,120 ✗ |
| portable object（LFCA）    |                     60,223,898 |         4 GiB |  1.4% |       无维度 |
| portable bundle            |                    181,768,264 |         8 GiB |  2.1% |       无维度 |

合成侧逐维度精确增量（与 `synthetic.rs` 各 `add_*` 的 `DeclarationResourceDelta`
逐项同构，经 10k/100k 公开锚点验证）：每 cell declarations 94 / typed_ast 782 /
references 192 / relations 212 / identity_fields 266 / symbols 94 / gates 12 /
waiting 2 / geometry_points 40；每 tile 车库 1/11/4/4/2/1；每模块共享三元组
3/20/3/3/6/3。冲突侧每 cell identity_fields 实测 87（10k 档 35,380 − 合成侧 26,680，
100k 档 353,800 − 266,800 精确吻合）。

### LFCA 逐表行数（100k；33 逻辑表全量见 evidence 文件）

最大表是 **CanonicalIdentity：125,400 行 / 3 chunks / max_chunk_rows 53,890 /
max_chunk_bytes 13,022,306**。其余表最大 26,000 行（LaneEdge、LaneEdgeGeometry），
均单 chunk。LaneGroup、FacilityBand、FacilityBandGeometry 三表为空（0 行 0 chunk）。

**"max_rows_per_table = 65,536" 的口径修正**：65,536 是 **单 chunk** 行数硬上限
（`crates/laneflow-static-contract/src/portable.rs:55`
`FORMAT_HARD_MAX_ROWS_PER_CHUNK = 65_536`），不是单逻辑表上限。LFCA 4 分 chunk 设计下
逻辑表行数 = Σ chunk 行数（`firstLogicalRow + rowCount` checked ≤ u32::MAX，
`docs/reference/portable-canonical-artifact.md` §7）。本 spike 实测 CanonicalIdentity
125,400 行 > 65,536 且全管线（writer 分 chunk → 零分配 verifier → shared build →
runtime install）合法通过。既有仓库证据同向：
`crates/laneflow-format/src/security_tests.rs:329`（65,536 接受 / 65,537 拒绝）与
`crates/laneflow-format/src/writer.rs:1781`（writer 在行数路径上恰好按 65,536 分
chunk）。本 workload 的 identity chunk 在 53,890 行处切分，由单 chunk 字节类预算驱动
（`FORMAT_HARD_MAX_TOTAL_UTF8_BYTES` / `FORMAT_HARD_MAX_TOTAL_VECTOR_BYTES`，各 8 MiB，
`portable.rs:67/73`；四个 chunk 预算的判定见
`crates/laneflow-format/src/limits.rs:40` `canonical_chunk_with_appended_row`），同样合法。

### P100 拒绝面（exact / exact+1 式边界证据，`LF-COMP-P100-INITIAL-v2`）

**(a) 真实 admission oracle 实测拒绝**（`DiagnosticPayload::CompileLimitExceeded`
携带 dimension/limit/observed）：

- 合成前端：逐 tile 准入，3 个模块通过，第 4 个模块拒绝——
  `SourceBytesTotal limit=542,741 observed=720,740`（合成模块源字节精确值
  180,185 B/模块 × 4）。
- 冲突前端（单模块逐 cell）：48 cells 通过，49 cells 拒绝——
  `SourceBytesPerModule limit=542,741 observed=542,750`（超出 9 B）。

**(b) 解析模型推论**（非 oracle 实测；基于每模块精确资源增量线性外推）：

- 即便无视源字节维度，relation occurrences 会在第 5 模块超限
  （5×2,127 = 10,635 > 10,032）；declarations 在第 12 模块超限
  （12×944 = 11,328 > 11,265）。
- 推论：10k 全量（11 模块）在 P100 v2 下至少 7 维超限（source bytes、declarations、
  stable entities、typed AST、relations、identity fields、symbols）；其中仅
  source bytes 两维（total / per-module）有 (a) 的实测拒绝点，其余维度为解析口径，
  未逐一探针实测。P100 两档均不可用。

## 复现命令

```powershell
# 在 spike 分支（codex/issue-543-capacity-spike）worktree 内：
cargo build --release --manifest-path research/issue-543-cn-urban-capacity/Cargo.toml
$bin = "research/issue-543-cn-urban-capacity/target/release/laneflow-issue-543-spike.exe"
& $bin run 100      # 10k 全管线
& $bin run 1000     # 100k 全管线
& $bin parts 1000   # 分解可加性
& $bin probe-p100 synthetic 20
& $bin probe-p100 conflict 60
& $bin model 1000   # 仅解析计数模型
```

环境：rustc/cargo 1.98.0（workspace `rust-version = "1.98"`），Windows x64，66 GB RAM。
100k 档全程约 36 s（其中冲突模块 FlatBuffers 源构建约 27 s）。

## 风险与后续

- **Adapter 层计数不在本 spike 范围**：本 spike 是 headless 容量测量，没有引擎表现域
  可测，`N_presented` 等 Adapter 侧计数未采集。设计文档 §4.5 要求记录
  "declaration/IR/LFCA/shared-static/runtime/Adapter 各层计数"，并称 #543 覆盖
  build/load/Adapter 各层完整核算；而 #543 Issue 正文未列 Adapter 层——两处口径差异
  在此登记。Adapter 层证据归 #544（Adapter harness）与 #545（证据矩阵）；建议
  #304 G1 冻结时对齐 §4.5 表述。本报告与 #543 关闭口径不得暗示已覆盖 Adapter 层。
- **运行层随档缩放不在本 spike 范围**：install live delta 用空载 8 槽世界测量，
  不含 `N_individual` 车辆状态与停车绑定；运行层 `N_individual`/停车绑定随档
  缩放的实测证据归 #544（Adapter harness）/#545（证据矩阵）。本 spike 的 install
  delta 不得被引用为运行层容量证据。
- **重算触发条件**（#543 评论）：#304 G1 candidate rules 变动（cell/tile/设施配比），
  或 #284 落地改变 LFCA 表集。本 spike 的生成器参数化到 cell 粒度，重跑成本低。
- **HIR/MIR 计数公开化**是独立 G1 事项；本报告以公开锚点 + admission 成功代替直接读数。
- 100k 档最紧的维度是 stable entities（12.5%）与 typed AST（≥9.8%）；若 candidate rules
  把每 cell 实体密度提高一个量级（如 8 引道 × 2 待转方向 × 显式泊位 ×25），stable
  entities 将逼近 1M v2 上限，届时才需要重议配置档（方案 B）。当前口径下无此需求。
- 坐标界限 ±16,384 m/轴 对城市范围布局是真实约束（单框架约 32.8 km 见方）；更大盘子
  需要多 CanonicalFrame 分片，属于路网建模口径，不是 format 限制。
