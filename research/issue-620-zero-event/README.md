# 零转移事件快路径的有限生产对照

#620 仅在 `stage_transition_events` 首遍完整验证、计数及 reserve 后，跳过零事件的
第二遍生成。首遍错误、空批次发布、失败零提交、有事件生成及排序保持不变。
不修改非入口 Gate 扫描、Waiting 规则、迁移日志、公开 API、格式或依赖。

## 结论

**建议采纳这个窄快路径供评审：三组未武装输入观察到整步耗时下降；武装输入尚未
证明提速。** 下表是 12 对独立进程轮次的整步平均时间配对变化，负数表示更快。

| 输入（车辆数 / 边数） | 日志 | 时间变化 | 描述性 95% 区间 |
| --- | --- | --- | --- |
| 1000 / 256 | 未武装 | -8.65% | [-13.60%, -4.82%] |
| 10000 / 256 | 未武装 | -6.46% | [-9.60%, -3.24%] |
| 10000 / 16 | 未武装 | -5.63% | [-10.29%, -0.32%] |
| 10000 / 256 | 武装 | -2.94% | [-7.69%, +3.95%] |

第三组区间上端接近零；武装组区间跨零，不能证明性能等效或排除小幅回归。
保留所有轮次，包括明显波动的轮次；没有扩测或事后挑样本。原画像中输出收尾
子树的 14.45% 权重不是可删除比例。本结果不代表有事件密集场景、100k 或跨机器
保证，也不完成父任务 #219；不追加其他热点。

## 验证与方法

- 新回归先在旧逻辑失败，再验证零事件一次 / 有事件两次访问、首遍错误、由有到无
  的空发布，以及失败保留旧状态和批次、retry/fresh 快照与三类输出一致。
- Runtime release：452 passed、0 failed、29 ignored；手动 A/B、分配入口另跑。
  fmt、release check、Runtime 架构检查、生产库严格 Clippy 通过。
- 两版六组分配输入均暖态零分配 / 重分配；状态摘要和迁移日志字节数一致。
- 原样复用 [#618 的入口及方法](../issue-618-unarmed-journal/README.md)：四组道路
  输入，100 ms 步长，Individual / Active / Intent 数保持，Presented / Aggregate 为 0。
  每世界暖机 32 + 8 拍、计时 64 拍；每进程排除但保留两个预定暖机窗口。
- 12 对 AB/BA 平衡轮次、输入顺序轮转，共 96 个独立进程，270336 拍正式样本及
  12288 拍排除暖机样本。全部进程、窗口、摘要和日志预算校验通过。
- 主指标为配对进程平均时长比值的几何平均；整对 bootstrap 10000 次、seed 618。
  区间为描述性区间，不把逐拍当作独立实验；计时不含建世界、校验、摘要和输出。
- 2026-09-08，同机 AMD Ryzen 9 9955HX、Windows MSVC、Rust 1.98.0 默认 release、
  平衡电源计划；未锁频或绑核，测量期间没有本任务的 Cargo 构建或 ETW 采样。

## 复现

基线 `1a7b3838703c32a7dd31ebe73d27b91a6153abc1`，
候选 `77c63f35233f5d54e3c3b7f89ae8e62f3448ebdb`；共同测试入口与构建环境一致。
分别构建并将 EXE/PDB 冻结到不同目录，然后使用既有脚本：

```powershell
cargo +1.98.0 test --release --locked --offline -p laneflow-runtime --test runtime_profile_evidence --test runtime_profile_allocation --no-run
pwsh -NoLogo -NoProfile -NonInteractive -File research/issue-618-unarmed-journal/run-ab.ps1 -Baseline '<baseline-exe>' -Candidate '<candidate-exe>' -OutputDirectory 'target/issue-620/new-run'
python research/issue-618-unarmed-journal/analyze.py 'target/issue-620/new-run' 'target/issue-620/new-analysis'
```

原始日志、逐拍样本、统计及源码 / 二进制指纹仅留本机 `target/issue-620/`，不入库。
其他机器复算本批历史结果需要先取得这些本地制品；分析器拒绝缺失进程或不一致窗口。
