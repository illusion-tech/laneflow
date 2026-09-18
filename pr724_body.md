Closes #712
Refs: #720（已由 #722 交付）、#721（已由 #723 交付）；stack 顶层，合并顺序 722 → 723 → 本 PR

切片类型：cross-layer（证据与组合验证层；不再改生产算法）

范围：
- 组合观察常驻测试（tests/pose_source_execution_equivalence.rs）：worker 1/2/4/8/16 逐步来源序列与状态摘要等价；失败 step（DeltaMismatch）只读旧已提交状态、同拍重试恢复且各 worker 一致；停车命令驱动来源显式转移（reserve 不变 → park 变 Parking），转移序列 worker 间一致。
- 独立研究程序 research/issue-712-borrowed-sources：512 平行车道合成修订（虚拟池 + 128 显式泊位），四形态数据集（全 Active 10k/100k、混合停车 10k/100k、高 Completed 10k、稀疏可表现 10k）；三指标（source_full 完整消费 / adapter_full 完整提取 / transform_convert 补充）+ 生命周期（cold / fresh_output / alternate）；A/B 用 legacy-source 薄适配 feature 区分（main.rs 字节相同，由汇总器强制 A/B 全等），两侧都不含 #718。
- 证据治理沿用 #711：环境一致性（完整 rustc -Vv/cargo/OS/CPU/逻辑核数/电源方案）+ feature 方向校验 + 逐日志完整 15 键 oracle + 样本唯一性 + A/B 程序身份；test-analyze.ps1 拒绝测试 6/6。

性能结论（A=b52f9ec4 按值 Vec，B=dc5a3685 借用迭代器；A₁B₁B₂A₂A₃B₃ 交错，每侧 3 墙钟进程 + 分配构建）：
- **稳态来源逐批分配消失**：全部数据集 source_full/adapter_full/alternate 分配 1 → 0 次/调用；fresh_output 3 → 2；cold 4 → 3（#712 验收核心成立）。
- **明确改善**：source_full 大规模档 -15% ~ -69%（100k 全 Active -18.5%、10k -68.6%、高 Completed -28.4%、混合 100k -15.3%）；adapter_full 10k/100k 档稳定 -6% ~ -7.6%；cold -11.8%。量级与 #681 测得的按值来源成本（1 分配 + 12–15 次增长 + 复制）一致。
- 小规模/稀疏数据集与 transform_convert 差异在噪声内（个别档呈本机双峰特征，如实标注，不作为结论）。
- 15 组完整提取 oracle SHA-256 摘要 A/B 逐键一致（输出等价）。原始数据 evidence/{before,after}/run1-3。

#718 组合观察（预集成，明确标注）：#718 未合并期间在 dc5a3685+b139fabc 临时预集成提交上验证——本栈组合观察测试全过；#718 自身等价套件在组合树上通过，其中 preview_at_threshold_equivalence（1_280 Active 超分发阈值）证明真实并行分发路径，来源读取观察叠加其上。#718 合入后本栈 retarget 新 main 由队列重跑，组合观察升级为常规基线。

文档：committed-pose-extraction.md 状态行与 §7 标注切片 2 已由 #712 实施；研究 README 记录方法、结果、局限与复现。

已运行检查：
- cargo +1.98.0 test --workspace --locked（全绿）
- cargo +1.98.0 fmt --all -- --check；cargo +1.98.0 clippy --workspace --all-targets --locked -- -D warnings
- 研究程序：clippy（默认与 allocation feature 分别）+ fmt + --smoke；analyze.ps1 通过 + test-analyze.ps1 6/6
- 预集成树：runtime 组合/等价套件（含超阈值分发）+ bevy 提取测试全过

未运行或未覆盖（复核后遗留，按 #712 关单前须补或明确延期裁定）：
- 栈内超分发阈值（>1_024 Active）的来源/提取观察测试与停车 leave 恢复断言：**尚未交付**；当前 #718 head（ba389ede）的预集成重做亦待执行（旧预集成基于 b139fabc，已过期）。
- 同 Session 涨缩 / 不同容量 output / 失败重试的容量轨迹专项证据：**尚未交付**（分配事件轨迹已有，存活容量轨迹无）。
- 完整城市性能认证与多 Session retained 压力不在 #712 范围（#713 产品式 harness 另行）。
- research/issue-681 历史程序按冻结实验处理（原始提交定位，不迁移）。

复核修复（8541e96a→本轮）：真实 90% Completed 夹具（测前状态断言）；oracle v2 全字段摘要 + 来源全序列摘要（token 敏感性实证）；analyze 23 项完整矩阵 + 编号集合 + 分配 iterations + cold 同强度校验；拒绝测试 9/9；全量 A/B 重采（六轮交错）；墙钟表述收紧（分配消除成立，中位数差值不作稳定幅度结论）。
