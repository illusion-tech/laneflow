# LuST 转换验收证据（#253 / PR #261）

本目录收录 pinned LuST 基线（`c4bd5bd3751d426d42a9a1749c815e47ea188549`，
`scenario/lust.net.xml` digest `sha256:6f5d76223cf14b797ae6267f13b23eb6c872d76adec1fb22a8569a806dc09341`）
下 converter 诊断模式的可复核交付证据。原始 LuST 大体积源数据不进 Git
（复审确认不要求）；本目录只收生成的确定性证据。

## 文件

| 文件                             | 内容                                                                                                                                                         | 锁定方式                                                                                                                                                                                                                              |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `lust-stub-weld-candidates.json` | 点状 stub 删焊候选 manifest：84 条候选逐条度量、处置（80 welded / 4 rejected-shared-entry）、规则版本 `stub-weld/g1-six-cond@2`、pinned commit 与 net digest | `convert::junction::stub_weld_manifest_tests::lust_stub_weld_candidates_match_manifest`（crate 内 #[cfg(test)] 模块） 对 pinned 源重扫逐字节比对（候选身份与处置的单一数据源是 `scan_stub_weld_candidates`，与 normalize 共用评估器） |
| `lust-infeasible-survey.md`      | 全网发射层不可行诊断清单：8,230 条（内 7,533 / 外 697，junction 1,854），逐条 span/弦长/转角/坐标/切向来源/机制/预算裁决                                     | `convert::acceptance::full_lust_net_topology_matches_external_lane_anchor`（crate 内 #[cfg(test)] 模块） 转换后与本文件逐字节比对                                                                                                     |

## 生成与双跑协议

```text
set LUST_SOURCE_DIR=<pinned c4bd5bd3 的 LuSTScenario 根目录>
cargo +1.98.0 test --locked -p laneflow-lust-converter --lib -- --ignored
```

- 主测试内部即执行**两次独立诊断转换**并断言清单逐字节一致；验收时完整跑
  两次测试进程，两次落盘（`target/issue253-infeasible-survey.md`）逐字节一致。
- 当前基线（规则 `stub-weld/g1-six-cond@2` + R9 诚实分类）的 survey SHA-256：
  `7d4f8243ee23fb06199986fd8f103ff0558a23992caadbc256d1c545f21fff6c`。
- 引入本文件版本的 commit 即对应 HEAD（git history 可追溯）；每次重锁在
  commit message 中记录新 digest。

## 语义声明（重要）

8,230 条不可行条目是「当前发射策略（采样划分 + 0.105 m 质量目标预检）未能
找到可行表示」的**诚实上界**，不是「源几何在硬预算（0.1 m / Balanced2Deg /
5 mm join）下不存在可行表示」的证明（PR #261 R9 审计；`ProvenBudgetConflict`
当前发射层无发射点）。预算裁决分布：采样候选耗尽 5,613、预算预检拒绝 2,599、
已证预算冲突 0、非预算类 18。

## 更新流程

发射语义、验收常数或源数据变化导致清单漂移时：

1. 跑上方命令两次，核对两次落盘逐字节一致并记录新 SHA-256；
2. 以新生成内容更新本目录对应文件（manifest 用
   `convert::junction::stub_weld_manifest_tests::regenerate_lust_stub_weld_candidates_manifest`，
   survey 取 `target/issue253-infeasible-survey.md`）；
3. commit message 记录新 digest 与漂移原因；G1 授权域（manifest 候选集合）
   变化须先回 #253 走 G1。
