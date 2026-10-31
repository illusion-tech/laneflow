# #1194 实现方案：#237 最小换道 S1/S3 切片（过渡感知扩展与静态发射）

**文档状态**: 实现方案（报统筹审 → G2 → 实现+等值验证 → 报告 → PR 不入队）
**最后更新**: 2026-10-31
**适用范围**: S1 过渡感知扩展、S3 材料化与静态发射、能力拒绝、A0/A1 硬等值、
合同状态回写（Review → Accepted）
**依据**: `docs/design/lane-change-minimal-contract.md`（已冻结，本切片转 Accepted）、
`research/issue-1185-route-contract-writeback/slice-s1-s3.md`（三项/四项口径）、
Issue #1194 范围与验收标准

## 1. 任务与边界

**做**：S1 过渡声明输入 + 仅声明边界转移 + §3.1 契约同步；S3 材料化 + 静态
发射（来源保真/分段锚点/单一权威拓扑/发射预算实测四口径）；能力拒绝
（`TransitionUnsupported` 结构化拒绝 + 注册表不变）；A0 五组 + A1 十组硬等值；
合同状态 Review → Accepted。

**不做（引用验收形态不实现）**：R1a/R1b/R2/R3 运行时验收（S4）、R3 持久化交叉
（S5）；不放宽生产路线行为（无声明处语义严格不变）；不改已冻结合同语义。

**纪律**：等值失败如实报不修绿；发射失败逐字留档；钉版几何不就地改值。

## 2. 现状勘察（实现点）

| 位置                                                                                      | 现状                                                                                                                                                                                                                                                                                                                            | 本切片触点                                     |
| ----------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------- |
| `convert/routes.rs:186-226` `can_complete_with_observer`                                  | 递归搜索；`:209` 严格规则 `hop.from_lane == lane_index`（memo 键 `(edge_index, lane)`）                                                                                                                                                                                                                                         | **S1 扩展点**：声明边界处允许来源 lane 转移    |
| `convert/routes.rs:228-282` `materialize_path_with_observer`                              | `:255` 同一严格规则选 hop，`:279` `current_lane = hop.to_lane`                                                                                                                                                                                                                                                                  | **S1 扩展点**：转移处物化过渡最终边链          |
| `convert/routes.rs:289` `build_hop_index`                                                 | `topology.path_by_connection` → `HopOption{from_lane,to_lane}`                                                                                                                                                                                                                                                                  | hop 增加可选过渡引用                           |
| `convert/routes.rs:41` `build_routes_and_bind_population`                                 | 生产展开入口（人口绑定）                                                                                                                                                                                                                                                                                                        | 签名加可选过渡声明输入（缺省空 = 行为不变）    |
| `convert/routes/test_support.rs`                                                          | A0 夹具构造器（缺口/连续/缺失 hop/未知 lane）                                                                                                                                                                                                                                                                                   | 扩展过渡夹具（sample s1 三拓扑 + s12/s4 构型） |
| `convert/routes.rs` tests                                                                 | **A0 五组**：`a0_fixture_proves_real_lane_gap_and_continuous_control`、`a0_lane_gap_fails_at_route_expansion_with_exact_error`、`a0_continuous_path_materializes_and_binds_population_deterministically`、`a0_source_reference_failures_are_stage_specific`、`a0_missing_road_hop_shares_current_lane_gap_error`                | **等值只读**                                   |
| `convert/routes/diagnostics.rs` tests + `diagnostics/tests/evidence.rs` + `acceptance.rs` | **A1 十组**：`a1_gap_and_missing_hop…`、`a1_branch_success…`、`a1_all_failed_start_lanes…`、`a1_repeated_occurrence…`、`a1_source_errors…`、`a1_unsupported_normalization…`、`a1_materialization_failure…`、`a1_stability…`、`a1_diagnostic_cost…`、`a1_route_evidence`（+ `a1_continuous_combined_static_result_is_recorded`） | **等值只读**                                   |
| `crates/laneflow-runtime/src/kernel/tables.rs:468` `compile_route` + `crate::RouteError`  | 注册期出现项编译；错误变体 `EmptySequence/UnknownEdge/Disconnected/ManeuverMismatch/AmbiguousManeuver/AllocationFailed/…`                                                                                                                                                                                                       | **能力拒绝点**：新变体 `TransitionUnsupported` |
| `output/emit.rs` + `output/geom/*`                                                        | 发射 + 几何修复（量化焊接预算）                                                                                                                                                                                                                                                                                                 | **S3 发射预算实测**（只测量不改预算）          |
| `output/provenance.rs`                                                                    | 来源出处机制                                                                                                                                                                                                                                                                                                                    | S3 来源保真挂点                                |

## 3. S1 实现设计

### 3.1 过渡声明输入（契约）

```text
TransitionDeclaration {
  id                        // 钉版样本 t01
  source_road               // "B"
  from_lane, to_lane        // 0 -> 1（来源 lane 语义）
  boundary_pair             // (上游 road occurrence, 下游 road occurrence)：(B, C)
  final_edge_chain          // ["T01"]（最终拓扑中的真实边链）
  provenance                // native | derived(+来源映射与构造依据)；不得冒充原 SUMO connection
}
```

- `build_routes_and_bind_population` / `expand_road_route` 增加
  `&[TransitionDeclaration]` 参数；**生产入口传空切片 → 行为与现状逐字节等值**
  （A0/A1 等值的前提）。
- 声明校验（失败关闭）：`from_lane != to_lane`；边界对必须是相邻 road
  occurrence；`final_edge_chain` 非空且每边存在于最终拓扑；重复/歧义声明拒绝。

### 3.2 展开语义（仅声明边界转移）

- `can_complete_with_observer`：hop `(from→to)` 处，除现有
  `hop.from_lane == lane_index` 分支外，**仅当存在声明**
  `(from, lane_index) → (to, hop.to_lane)` 时允许该 hop；其余位置严格规则不变。
  memo 键不变；观察器新增 `transition_hop` 回调（诊断只增不改）。
- `materialize_path_with_observer`：同一允许条件下选 hop；物化时把该 hop 的
  来源 lane 段替换为 `final_edge_chain`（如 `[A0, I1, B0_up, T01, B1_down, I2,
  C0]`）；稳定选择（lexicographic-min）口径不变——**声明转移不改变既有成功
  路径的选择结果**（先按现行规则选最小，仅当现行规则失败且声明可完成时走
  转移路径；两可时不引入新偏好）。
- **错误语义保持**：无声明或声明不覆盖处失败 → 现行 `Error::SumoModel`
  文本 `no complete lane-level path for road route [...]` 逐字不变；A1 诊断
  分类（`LaneContinuityGap`/`RoadHopMissing`/…）语义不变。

### 3.3 §3.1 契约同步（随本切片 G1 语义）

`docs/design/real-road-workloads.md` §3.1 车道级展开条款措辞修订：「禁止边内
换道」→「仅在声明过渡材料处允许来源 lane 转移（转移只消费声明的最终边链，
不产生横向转移）」；其余条款不动。

## 4. S3 实现设计

### 4.1 材料化与静态序列验证（独立于运行时能力）

- `test_support` 扩展：以样本 `minimal-closed-loop.json`（v5）钉版几何构造
  合成夹具——`B-up/TZ/B-down` 分段、`T01` 真实边、`sl-t01`/J2 门停止线锚点、
  provenance 记录（native 合成声明）。
- **独立静态序列验证**（不经 `register_route`）：对过渡路线与直行路线验证
  静态序列合同——完整 ManeuverPath 匹配、内部边覆盖、首末边/StopLine 规则、
  hop 半开区间不相交（复用 `compile_route` 的纯校验子集或等价 helper，
  不含运行时准入）。
- **单一最终权威拓扑**：夹具归一化只产出一份拓扑供展开与发射消费（断言
  两次归一化逐值相等）。

### 4.2 几何发射预算实测（四口径 + 失败留档）

1. **实测条件**：冻结提交（本切片实现 HEAD）+ 固定工具链（Rust 1.99.1）+
   全新输出目录（测试内临时目录）；逐边记录发射结果入证据 JSON。
2. **失败处置**：逐字保留发射器错误原文（参照 A1 `repaired curve sampling
   exhausted … curvature is infeasible under the quantized weld budget`），
   归类独立几何问题；**不修绿、不提预算、不改钉版几何**。
3. **改几何走版本流程**：如需调整钉版几何 → `meta.version` 递增 + changelog +
   另行审阅。
4. **成功判据**：发射通过且预算零放宽；逐边结果与整数毫米长度入证据档。

## 5. 能力拒绝设计（A6 选项 3）

- `RouteError` 新增变体 `TransitionUnsupported`（结构化、失败关闭、零部分
  状态：注册表不变——沿 `compile_route` 失败原子语义）。
- **过渡边判定（派生规则，不新增静态实体）**：某 hop 的出边/入边分属同一
  道路（RoadCorridor）**不同 lane 链**、且中间过渡边未被任何 lane 覆盖
  （uncovered）→ 该过渡边为横向转移段；junction internal 跨道路
  （entry/exit 属不同 road）不误判。
- **入口**：`compile_route`（注册期出现项编译）检出过渡 hop 且当前构建不支持
  过渡执行 → `TransitionUnsupported`；车辆准入/恢复入口按合同 §6 双层复核
  （本切片先落注册期 + 绑定期复核 stub 面向 S4；全部入口统一在 S4 收口）。
- **验收边界**（PR #1188 评审修正条目）：S3 静态序列验证 +「执行不支持时
  注册被拒」断言；含过渡路线的**注册成功验收在 S4**。

## 6. 文档回写

- `docs/design/lane-change-minimal-contract.md` 状态行
  `Review（#237 冻结提案…）` → `Accepted（#237 G1 冻结，维护者 2026-10-31）`。
- `real-road-workloads.md` §3.1 措辞修订（§3.3）。
- `research/issue-1194-lane-change-s1-s3/README.md`：交付索引 + 验收对照 +
  未执行声明（R1–R3 引用不实现）。

## 7. 测试与等值计划

| 组               | 内容                                                                                            | 判据                                                                                      |
| ---------------- | ----------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| **A0/A1 硬等值** | A0 五组 + A1 十组 + 既有包级全部测试                                                            | 逐项通过；`git diff` 确认既有测试文件只增不改；旧入口（生产签名缺省空声明）旧结论逐值不变 |
| S1 新增          | sample `s1` 三拓扑（gap/continuous-control/transition）+「无声明即转移」负例 + 声明校验负例     | gap 失败关闭且无替代链；transition 物化 `[A0,I1,B0_up,T01,B1_down,I2,C0]`；负例失败关闭   |
| S3 新增          | 样本夹具静态序列验证（过渡/直行路线）+ 单一拓扑两次归一化逐值相等 + 发射预算实测证据            | 静态合同全过；发射结果逐边入档                                                            |
| 能力拒绝         | 过渡路线注册 → `TransitionUnsupported` 且注册表不变（前后快照逐值相等）；非过渡路线注册不受影响 | 结构化错误 + 失败原子                                                                     |
| 持久化/运行时    | R1a/R1b/R2/R3：**引用验收形态不实现**（slice-s1-s3.md §4 矩阵 + 合同 §7），测试面留 S4/S5       | 本切片不产出                                                                              |

## 8. 风险与待统筹确认

1. **过渡边判定派生规则**（§5）：以「uncovered 边 + 同走廊不同 lane 链」为
   判据不新增静态实体——请确认与 A1「无新增注册校验类别」的相容口径（新错误
   变体属 A6 授权的拒绝面，非新校验类别）。
2. **选择语义**（§3.2）：声明转移只在现行 lexicographic-min 失败时启用（不
   引入新偏好）——请确认该口径与 §3.1 修订措辞一致。
3. **S3 夹具载体**：以 converter `test_support` 合成夹具承载样本网络（非
   LuST pinned 输入）——请确认发射预算实测以该合成为准（LuST 全量发射属 S6）。
4. 生产入口 `build_routes_and_bind_population` 签名变更（加可选参数）属
   converter 内部 API；`routes.toml` schema 不变。

## 9. 排程

实现方案报审 → G2（记录于 #1194）→ 实现（S1/S3/能力拒绝/文档回写）+ 等值
验证（A0/A1 全量 + 新增测试）→ 报告（等值/发射/拒绝结果如实）→ PR（不入队
交统筹）。发射实测在实现期执行；等值失败即报不修绿。
