+++
schema_version = 1
id = "reference-wording-guide"
kind = "reference"
status = "active"
revised_date = "2026-11-17"
summary = "按语境区分口语、直译词和具体机制的表述。"
scope = "正式文档、Agent 工作流与代码说明中的条件化措辞"
related = ["governance-terminology-policy", "reference-glossary", "reference-terminology-catalog"]
+++

# 措辞与语境指南

以下规则说明如何按实际对象和机制改写，不是全文字符串替换清单。原则、历史豁免
与精确标识符边界见[语言与术语政策][policy]；确定的概念改名见[术语表][glossary]。

规则正本为 `docs/reference/terms/wording.toml`。正文由 `generate-glossary` 生成，
修改方式见[术语目录规范][catalog]。

[policy]: ../governance/terminology-policy.md
[glossary]: glossary.md#former-names
[catalog]: terminology-catalog.md

<!-- generated-terminology:start -->

<a id="wording-authority-label"></a>
## 权威

适用语境：authority 的职责、正本、唯一来源或依据语义。

组件职责使用裁决、裁决方或裁决权；数据持有使用正本；唯一来源使用唯一事实来源；陈述句按需要写“以……为准”。

- [最终裁决职责][term-authority]：组件最终裁决责任。
- [唯一事实来源][term-single-source-of-truth]：规范来源唯一性。

例外与边界：英文别名 authority 与精确标识符保留。

登记依据：[\#1229][issue-1229]。

<a id="wording-completion-language"></a>
## 收口 / 闭合 / 口径

适用语境：任务完成、治理检查、几何、集合或统计表述。

完成状态写明任务完成、最终审阅、验收通过、表示统一或入口限制；闭合须有明确对象与判据；口径须说明计数对象、统计范围、测量方法或比较基准。

例外与边界：几何首尾相接、集合封闭、所有权完整性及最小发布闭合等明确含义可以保留。

例外与边界：保持历史完成时间、未完成范围与验收条件；不得据此宣称没有漏检。

登记依据：[\#1017][issue-1017]。

<a id="wording-core-runtime-name"></a>
## Core / Core Runtime / LaneFlow Core / current Core

适用语境：指现行交通运行时的散文、注释或错误文案。

改用 LaneFlow 交通运行时；与空间层、适配器并列或修饰步进指标时可在上下文明确后写运行时；边长写交通边长。

- [LaneFlow 交通运行时][term-laneflow-traffic-runtime]：现行动态执行层。

例外与边界：已拆除对象的精确标识符、ADR 历史决策与已退役文档、已登记 Skill ID laneflow-core-design、任务类别 core-runtime、现有文件名和第三方同形词保留。

例外与边界：机器度量键与公开 API 标识符的改名由对应合同处理，不从文字规则推导。

登记依据：[\#1339][issue-1339]。

<a id="wording-digest-envelope"></a>
## 摘要封套 / 完整性封套 / 证据封套

适用语境：harness 自校验。

区分摘要映射与被摘要记录，保留先核对摘要、后读取运行字段的顺序。

- [文件摘要清单][term-file-digest-manifest]：文件名到摘要的映射。
- [诊断记录][term-diagnostic-record]：被核对的诊断载荷。

例外与边界：同目录摘要不提供执行或来源真实性认证。

登记依据：[\#1012][issue-1012]。

<a id="wording-distortion-colloquial"></a>
## 打中 / 被打中

适用语境：完整性与内存失真语境。

按句法写发生失真、受失真影响或导致失真，保留对象、时间、可能性与共因条件。

- [失真][term-distortion]：仅描述内容偏离应有内容。

例外与边界：不据此断言已经确认页级故障或特定失真机制。

登记依据：[\#1009][issue-1009]。

<a id="wording-experiment-arm"></a>
## 臂 / 双臂 / 跨臂 / 参考臂 / 分发臂

适用语境：实验、测试与测量语境。

按实际对象区分参数组合、测试用例、执行实例、代码路径与候选方法。

- [运行配置][term-run-configuration]：参数组合。
- [测试分组][term-test-case-group]：一组测试用例。
- [单次运行][term-individual-run]：一次实际执行。
- [执行路径][term-execution-path]：条件选择的代码路径。
- [实验方案][term-experimental-variant]：候选实现或方法。

登记依据：[\#1012][issue-1012]。

<a id="wording-fallback-check"></a>
## 兜底

适用语境：校验、测试、异常、检测及处置。

写明检查对象、执行阶段、错误或捕获动作、未覆盖面与宿主处置责任。

- [补充检查][term-additional-check]：额外核对另一条件或粒度。

例外与边界：不暗示恢复成功或必然检出。

登记依据：[\#1014][issue-1014]。

<a id="wording-fallback-order"></a>
## 兜底

适用语境：排序、残留歧义或路径竞争。

写明最终排序字段、消解歧义的机制或拒绝同名路径。

- [最终排序键][term-final-sort-key]：前序字段相同时才比较的键。

例外与边界：deny-overrides 消解残留歧义是被否决的方案；路径竞争检查不等于消除全部 TOCTOU。

登记依据：[\#1014][issue-1014]。

<a id="wording-fallback-output"></a>
## 兜底

适用语境：诊断内容、缺失来源和名称截断。

写明占位、补入来源位置或用摘要后缀区分名称。

- [占位输出][term-placeholder-output]：为继续诊断保留的替代内容。

例外与边界：占位不提升为验收通过，摘要不提升为无碰撞保证。

登记依据：[\#1014][issue-1014]。

<a id="wording-fallback-path-language"></a>
## 兜底

适用语境：路径选择、测试执行方式与默认规则。

按实际触发条件与优先级表述。

- [回退路径][term-fallback-path]：当前路径条件不成立时转入既有路径。
- [备用测试方式][term-alternative-test-execution]：另一种测试执行方式。
- [默认规则][term-default-rule]：未指定可选条件时采用的规则。

例外与边界：调试配置直接说明变量用途；不同测试线程模型和计时结果不能混同。

登记依据：[\#1014][issue-1014]。

<a id="wording-gate-label"></a>
## 开发闸口 / 设计闸口字段 / 门禁 / 闸口 / 关卡

适用语境：流程、质量检查或验收语境。

按对象区分人流程的阶段评审、机器阻断的检查以及产品或性能验收。

- [开发阶段评审][term-development-gate]：G0–G2 人流程。
- [设计评审字段][term-design-gate-field]：GitHub 的 Design gate 字段。

例外与边界：ManeuverGate 的机动门领域义、Gate 精确标识符、字段名与文件名不变。

登记依据：[\#1229][issue-1229]。

<a id="wording-preimage-label"></a>
## 前像

适用语境：摘要、标识与编码中的 preimage。

使用“原像”，并说明是哪一种摘要或标识的原像。

例外与边界：精确标识符保留原文。

登记依据：[\#1229][issue-1229]。

<a id="wording-reader-label"></a>
## 读器

适用语境：reader 组件语境。

使用自然中文“读取器”。

登记依据：[\#1229][issue-1229]。

<a id="wording-road-arm"></a>
## 臂 / 臂道 / 道路臂 / 入口臂 / 出口臂 / 接近臂

适用语境：场景生成或路口有向接近关系。

区分路口外部实体道路与指向入口、出口的有向关系；计数明示有向、双向或车道。

- [路口外部道路][term-external-road-segment]：描述实体道路。
- [进口道][term-approach]：指向入口的有向关系。
- [出口道][term-exit-approach]：指向出口的有向关系。

例外与边界：Arm、approach 等标识符，以及悬臂灯架、横臂等实体构件名保留。

登记依据：[\#1012][issue-1012]、[\#1229][issue-1229]。

<a id="wording-root-shorthand"></a>
## 根 / 根审计 / 根比较 / 候选根 / 该根 / 根重建

适用语境：运行时状态完整性与处置中把共享路网修订简称为根。

按实际对象写修订审计、修订比较、候选修订、该修订或修订重建。

- [共享路网修订][term-shared-network-revision]：上下文明确时可使用已经登记的简称“修订”。

例外与边界：明确描述 ADR 0025 所有权结构的根对象、根 Arc、从根借用、根共享、根配对、换根或同修订换根不在此范围。

登记依据：[\#985][issue-985]。

<a id="wording-shared-root-identifiers"></a>
## shared\_root

适用语境：历史内部与测试标识符、机器可读度量键或 CSV 表头。

内部与测试标识符按实例或内容区分命名；度量字段按其计数对象命名，并与读取端同步。

- [共享路网修订][term-shared-network-revision]：实例命名使用 shared_network_revision。
- [共享静态路网][term-shared-static-network]：层或内存度量使用 shared_static_network。

例外与边界：公开 API 与协议字面量不因散文旧称规则自动改名；历史记录保留原键，读取兼容范围由对应工具合同规定。

登记依据：[\#985][issue-985]。

<a id="wording-task-slice"></a>
## 切片

适用语境：任务类别或工作分块语境。

类别使用任务类别；工作分解按语境写子任务、某部分或任务与批次。

- [任务类别][term-task-category]：PR 的验证维度。

例外与边界：Rust slice、字节切片等数据义不适用。

登记依据：[\#1229][issue-1229]。

<a id="wording-timing-envelope"></a>
## 计时封套 / 执行封套

适用语境：harness 的运行输出。

按实际文件说明记录对象；语义比较排除集合应列出具体文件。

- [逐拍计时日志][term-per-tick-timing-log]：逐拍耗时。
- [测量记录][term-measurement-record]：协议、配置与样本。
- [诊断记录][term-diagnostic-record]：运行诊断。
- [运行记录][term-run-records]：整次执行输出的统称。

例外与边界：不能以泛称扩大既有校验或语义比较排除范围。

登记依据：[\#1012][issue-1012]。

<a id="wording-truth-source-label"></a>
## 单一事实源 / 事实源

适用语境：唯一规范来源或一般事实来源的表述。

SSOT 写唯一事实来源；一般来源写事实来源。

- [唯一事实来源][term-single-source-of-truth]：强调同一事实的规范所有权。

登记依据：[\#1229][issue-1229]。

<a id="wording-wrapper-envelope"></a>
## 封套

适用语境：按值 Vec 包装或 RowV1 外层结构的泛称。

分别说明类型层与编码层的包装。

- [包装类型][term-wrapper-type]：类型层。
- [外层编码结构][term-outer-encoding-structure]：编码层。

例外与边界：源映射封套、语义差异封套及明确指代它们的简称保留；验证收据封套仅在历史方案语境保留。

登记依据：[\#1012][issue-1012]。


[issue-1009]: https://github.com/illusion-tech/laneflow/issues/1009
[issue-1012]: https://github.com/illusion-tech/laneflow/issues/1012
[issue-1014]: https://github.com/illusion-tech/laneflow/issues/1014
[issue-1017]: https://github.com/illusion-tech/laneflow/issues/1017
[issue-1229]: https://github.com/illusion-tech/laneflow/issues/1229
[issue-1339]: https://github.com/illusion-tech/laneflow/issues/1339
[issue-985]: https://github.com/illusion-tech/laneflow/issues/985
[term-additional-check]: glossary.md#term-additional-check
[term-alternative-test-execution]: glossary.md#term-alternative-test-execution
[term-approach]: glossary.md#term-approach
[term-authority]: glossary.md#term-authority
[term-default-rule]: glossary.md#term-default-rule
[term-design-gate-field]: glossary.md#term-design-gate-field
[term-development-gate]: glossary.md#term-development-gate
[term-diagnostic-record]: glossary.md#term-diagnostic-record
[term-distortion]: glossary.md#term-distortion
[term-execution-path]: glossary.md#term-execution-path
[term-exit-approach]: glossary.md#term-exit-approach
[term-experimental-variant]: glossary.md#term-experimental-variant
[term-external-road-segment]: glossary.md#term-external-road-segment
[term-fallback-path]: glossary.md#term-fallback-path
[term-file-digest-manifest]: glossary.md#term-file-digest-manifest
[term-final-sort-key]: glossary.md#term-final-sort-key
[term-individual-run]: glossary.md#term-individual-run
[term-laneflow-traffic-runtime]: glossary.md#term-laneflow-traffic-runtime
[term-measurement-record]: glossary.md#term-measurement-record
[term-outer-encoding-structure]: glossary.md#term-outer-encoding-structure
[term-per-tick-timing-log]: glossary.md#term-per-tick-timing-log
[term-placeholder-output]: glossary.md#term-placeholder-output
[term-run-configuration]: glossary.md#term-run-configuration
[term-run-records]: glossary.md#term-run-records
[term-shared-network-revision]: glossary.md#term-shared-network-revision
[term-shared-static-network]: glossary.md#term-shared-static-network
[term-single-source-of-truth]: glossary.md#term-single-source-of-truth
[term-task-category]: glossary.md#term-task-category
[term-test-case-group]: glossary.md#term-test-case-group
[term-wrapper-type]: glossary.md#term-wrapper-type

<!-- generated-terminology:end -->
