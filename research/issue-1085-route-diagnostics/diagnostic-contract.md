# A1 诊断修订（#1085）

本轮修复错误归因和计量，不改正式路线排序、选择、错误文本或绑定行为。

归一化使用同一个实现。在现有 lane 地址或 via 引用解析实际失败的位置标记 `SourceReference`；其他失败保留 `Unclassified`。正式入口剥离内部诊断原因，返回原 `Error`。诊断不解析错误字符串，也不复制一套独立归一化算法。真实的点状 stub 携带后续几何被原规则拒绝时，结果为 `Normalization / Unclassified`；保留原始错误，不伪造 population 上下文。

`records` 是提交给诊断的记录数。`expansionCalls`、`stateEvaluations`、`memoHits` 和 `materializeCalls` 只计诊断观察器实际执行，不含为对照而调用的正式入口。来源校验在展开前失败时，展开计数为 0；空路线实际进入展开入口后失败，计数为 1。注入标记仅在物化入口实际执行时成立。

`elapsedNs` 是整个 `diagnose_population` 内部工作耗时：建索引、正式入口基线、诊断展开、绑定和排序全部计入。它不包含调用方的 XML 解析、先前归一化或之后 JSON 序列化；它不是纯展开计时，也不是路线性能提升证据。

证据新增真实的 `normalization-unsupported-stub-continuation`，共 15 个案例。原来的 11 组 A0 正式行为比较继续执行。两个来源错误案例的展开计数从错误的 1 更正为 0；不能要求新旧诊断 JSON 逐字相同。运行时注册、车辆准入、完整 LuST TOPO/DEMAND 和安全验证仍未在 A1 执行。
