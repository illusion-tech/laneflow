# 编译器资源与性能预算校准

**文档状态**: Draft（#308 G1；未取得 G2）<br>
**最后更新**: 2026-07-30<br>
**适用范围**: 编译器工作负载、编译器校准规模（Compiler Calibration Scale）、
编译器压力规模（Compiler Stress Scale）、编译资源上限（Compile Limits）、冷实例
与稳定容量复用测量、研究停止护栏（Research Stop Guardrail）、私有容器候选和
机器可读研究证据<br>
**实现状态**: 未实现；本文只冻结非生产研究设计，不实现生产编译器、不冻结公共
应用程序接口（API），也不形成产品服务等级协议（product SLA）

**关联决策与设计**:

- `../adr/0014-residual-aware-f32-core-authority-and-migration-gates.md`
- `../adr/0020-compiler-owned-static-network-and-static-image.md`
- `../adr/0021-city-simulation-game-traffic-foundation.md`
- `network-compiler.md`
- `core-runtime-performance-baseline.md`
- `data-format.md`
- `spatial-geometry.md`
- `../reference/glossary.md`
- `../reference/compiler-calibration-workloads-v1.json`
- `../reference/compiler-calibration-evidence-v1.schema.json`
- `../reference/compiler-calibration-contract-v1.json`
- [#292 编译器基础设施草案文件（PR #307，head `3b430e3`，待合入）](https://github.com/illusion-tech/laneflow/blob/3b430e37343949ea8511a6da0596d1795dadcf0d/docs/design/compiler-foundation.md)
- [#308](https://github.com/illusion-tech/laneflow/issues/308)
- [#292](https://github.com/illusion-tech/laneflow/issues/292)

`compiler-foundation.md` 当前只存在于 #292 的兄弟分支，不能在本分支伪装成本地文件。
本设计合入后，#292 必须先 rebase，再把上面的跨分支链接替换为本地
`compiler-foundation.md` 互链并按第 11 节完成术语、工作负载和预算回写；该动作是
#292 G1 的前置条件。

## 术语规范

本文的中文术语和中文定义是权威事实，英文只作辅助理解。完整双语映射以
`../reference/glossary.md` 为单一事实源（SSOT）；本文不另建竞争词表。Rust 类型、
crate、字段、版本值、算法和协议常量等精确标识符使用反引号保留原文。

编译器工作负载只按编制来源、编译器中间表示、关系和输出记录计数。本文不使用
车辆、交通参与单元或 `Agent` 数量表达编译规模，也不把合成对象数量解释为真实城市
规模。

## 1. 问题、目标与证据边界

### 1.1 问题

#292 需要为 `CompileLimits`、干净单工作线程编译、冷实例、稳定容量复用、失败清理
和私有容器选择冻结可执行预算。#308 只测量研究干净单工作线程编译（Clean
Single-thread Research Compile），不能重定义 #292 的生产参考路径。当前仓库可以
证明：

- 当前固定夹具能够提供正确性和小输入固定成本；
- ADR 0020 与 `network-compiler.md` 已冻结编译阶段、标识 v1、确定性和安全边界；
- `core-runtime-performance-baseline.md` 已登记 R0 固定研究机及研究证据声明纪律。

当前仓库不能证明任意“参考城市对象数”、绝对编译秒数、内存上限或产品编辑体验。
若直接填写这些数值，会把设计直觉伪装成实测事实。

### 1.2 目标

本文冻结一套独立、可复现的非生产研究：

1. 用三个正交合成工作负载测量标识/符号、走廊关系、密集路口关系的扩展曲线；
2. 用当前固定夹具证明研究内核的语义对照和小输入固定成本；
3. 通过实测噪声底、等比规模阶梯和预先声明的拐点规则发现校准规模与压力规模；
4. 分别测量冷实例、稳定容量复用、失败输入、分配、存续内存与输出字节；
5. 对私有容器和哈希候选进行同输入、同语义输出的平衡顺序比较；
6. 产出机器可读原始证据和供 #292 G1 裁决的预算建议。

### 1.3 非目标

本研究明确不：

- 实现 `laneflow-static-contract`、`laneflow-compiler`、生产合成领域专用语言前端
  （Synthetic DSL Frontend）或任何生产后端；
- 冻结或修改公共编译器 API、规范低层中间表示（canonical LIR）ABI、可移植规范
  制品或目标静态镜像；
- 用研究停止护栏冒充生产资源上限或性能通过条件；
- 从 R0 结果外推最低/推荐产品硬件、玩家编辑体验或中国特色城市工作负载 SLA；
- 用当前运行时的一万/十万车辆证据、多世界吞吐或表现层结果替代编译器证据；
- 预先选择第三方容器、哈希库或生产依赖。

### 1.4 结果允许支持的声明

| 结果种类     | 允许声明                                     | 禁止声明                                          |
| ------------ | -------------------------------------------- | ------------------------------------------------- |
| 固定夹具等价 | 研究内核对当前夹具产生冻结的语义摘要         | 已实现生产编译器或完整行为等价                    |
| 合成规模曲线 | R0 上特定内核、候选和记录组合的成本/内存增长 | 对应真实城市容量或交通参与单元规模                |
| 容器候选比较 | 相同输入与输出下的平衡顺序相对成本           | 未测键域、平台或攻击面的全局最优                  |
| 研究预算建议 | #292 G1 可审阅的 R0 研究预算输入             | 产品通过（Product Pass）、玩家体验 SLA 或公共契约 |
| 停止护栏触发 | 继续扩大实验会突破本研究的安全/成本边界      | 生产实现达到容量极限                              |

## 2. 权威输入与非生产边界

### 2.1 上游权威

研究内核必须服从以下 Accepted 事实：

- ADR 0020 与 `network-compiler.md` 冻结的
  `Typed AST -> HIR -> MIR -> canonical LIR` 阶段职责；
- `identityEncodingVersion = 1`、`identityRegistryRevision = 1` 的二十二种实体与
  必需标签顺序；
- 来源模块图、稳定命名空间、显式键、父实体稳定标识、所有者局部键和有类型
  `u32` 序号的职责边界；
- 相同规范输入产生相同语义、顺序和要求范围内相同字节的确定性要求；
- 外部可控输入必须先受调用方资源上限约束，哈希命中后仍须完整相等比较。

研究内核可以只实现这些阶段的代表数据变换，不得把缺失的生产语义伪装成完整
编译器。

### 2.2 研究包边界

取得 #308 G2 后，研究代码拟位于：

```text
research/issue-308-compiler-budget-calibration-research/
```

其边界必须与 `research/issue-123-spatial-prototype` 的既有治理方式一致：

- 可以作为 Rust 工作区成员接受统一格式、测试、Clippy、文档和依赖审计；
- 包名固定为 `issue-308-compiler-budget-calibration-research`，README 必须明确标记
  “非生产研究”，`Cargo.toml` 必须设置 `publish = false`；
- 生产 crate 不得依赖该研究包；
- 研究 runner 的普通二进制必须能够看见其运行时依赖；第三方候选以及
  `laneflow-data` / `laneflow-spatial` 必须登记为该私有研究包的**可选普通依赖**，
  不得错误放入只对测试、示例和基准可见的 `[dev-dependencies]`；
- `default = []`；每个第三方候选使用独立私有 feature，当前夹具对照预言机使用
  `fixture-oracle`，`research-runner-full` 作为正式单入口的封闭总 feature。G2
  依赖审计冻结具体 package/version 后同步冻结该总 feature 的精确成员；
- `laneflow-data` / `laneflow-spatial` 只允许由 `fixture-oracle` 在计时区外读取当前
  夹具和运行对照预言机，不得被合成研究管线调用；
- `[dev-dependencies]` 只保存不会被普通 runner 二进制链接的测试辅助依赖；
- 研究类型、候选标识和证据封套不进入生产公共 API；
- 研究代码不得通过路径依赖复用尚未实现的 #292 生产 crate。

本文合入、#308 G1 通过或研究包进入工作区都不等于 G2。只有 #308 Issue 上明确的
G2 开工判断才能授权实现研究代码。

### 2.3 研究管线

研究基准执行器（Research Benchmark Harness）使用独立的、最小代表数据形状：

```text
确定性生成配方/清单
  -> 有类型抽象语法树形状记录的受检物化
  -> HIR 形状的符号/引用解析
  -> MIR 形状的拓扑/几何/关系展开
  -> canonical LIR 形状的稳定排序、定长序号和输出所有权落定
  -> [主计时区外] 完整输出比较、语义摘要与证据序列化
```

每个箭头必须是显式阶段，输出有独立记录数、逻辑字节和分配归属。各阶段粗粒度时延
只在独立归因进程采集；正式端到端时延只用管线外侧一对时钟读取，避免阶段计时器本身
改变热路径。研究实现不得调用当前 `laneflow-data` loader 代替合成阶段，也不得把
JSON 解析或磁盘 I/O 计入合成内核时延。

计时区外的生成配方只保存工作负载标识符、模块图/字符串配置档、`N`、workload
seed、固定比例、工作负载清单摘要和预期计数，不得预先分配与 `N` 成比例的领域记录。
有类型抽象语法树形状记录的字符串、声明和关系物化属于受测管线，必须同时进入端到端
时延和编译器控制总存续内存；否则研究会漏掉 Synthetic DSL builder 与来源资源上限
的主要成本。

研究输出只需要 canonical LIR **形状**的规范记录，不生成生产规范制品、源映射、
验证收据或静态镜像。规范记录构造和所有权落定属于受测成本；完整输出比较与摘要计算
只用于计时区外证明候选等价，不宣称逐字节等于未来生产制品。

#### 2.3.1 研究阶段记录模型

研究阶段记录模型（Research Stage Record Model）只冻结 #308 非生产替身的可比数据
形状，不冻结未来生产编译器的 Rust 类型、IR 公共 API 或静态制品布局。每个阶段使用
一段连续值记录和一段连续载荷字节；禁止每条记录单独堆分配。机器清单固定字段顺序、
字段宽度、`repr(C)` 大小、记录粒度和每级公式：

| 阶段          | 值记录字段宽度 / `repr(C)` 大小 | 记录粒度与逻辑字节公式                                                                                                                        |
| ------------- | ------------------------------: | --------------------------------------------------------------------------------------------------------------------------------------------- |
| Typed AST     |                 `32 / 32` bytes | module/import/declaration/identity field/reference/relation/geometry 各一条；`32 * records + source bytes + string bytes + 20 * source spans` |
| HIR           |                 `32 / 32` bytes | module/import/symbol/identity field/resolved reference/typed relation/checked geometry 各一条；载荷只用字符串字节和规范 `u32` 操作数          |
| MIR           |                 `44 / 48` bytes | 每条最终研究语义记录一条、尚未规范排序；`44 * semantic records + exact semantic payload bytes`                                                |
| canonical LIR |                 `44 / 48` bytes | 每条最终研究语义记录一条、已排序并落定 owner-local ordinal；公式与 MIR 相同                                                                   |

来源位置固定为五个 `u32`、二十个逻辑字节。MIR/LIR 的四个填充字节属于真实分配，
不属于按字段宽度求和的逻辑字节；`size_of`、容器容量、分配器元数据和进程指标分别进入
内存证据。G2 在任何测量前必须断言全部 `repr(C)` 大小，不能用字段相同但布局不同的
私有结构替代。

每级聚合输入也由清单冻结：来源文档数等于模块数；来源声明、身份声明、身份字段、
带配置键出现项、引用、关系和几何分别由工作负载每单元阶段输入乘 `N`，再叠加模块图
明确登记的共享来源常量和跨模块引用；符号数等于来源声明数；来源位置数等于声明、
引用、关系和几何之和。来源字节数按每条声明/引用/关系/几何的来源令牌（source
token）连同行尾分别固定为 `21/19/18/18` 字节求和，不再允许执行器自行选择序列化文本。

字符串聚合按以下规范顺序逐项枚举：模块名、来源文档键、导入目标模块名、每个身份
声明的三十二字节命名空间、身份字段中的配置键、每条来源引用的三十字节规范拼写，
最后才是可选共享常量的名称和值。模块图配置档冻结模块名/文档键/导入目标的总字节与
最大项；字符串配置只控制配置键长度。字符串项数、单字符串最大字节数和总字符串字节
数必须从该同一枚举计算，禁止只把 `profiledKeyLengthBytes` 冒充完整字符串聚合。
精确语义载荷字节由 `recordKinds`、`identityBindings`、字符串配置和该级规范记录逐条
求和。

当前夹具不套用 `N` 公式，而是对每个 case 使用清单冻结的文件到研究记录投影，并登记
全部领域计数、聚合输入、语义载荷和八阶段记录/逻辑字节常量；它仍不参与预算或候选
排名。

`metrics.stageBreakdown` 的八个阶段必须逐项使用清单公式。生成器与独立验证器分别
枚举聚合输入、阶段记录数、载荷字节、逻辑字节和输出构造字节；只报告任意阶段计数、
却没有物化清单要求的值记录与载荷缓冲区，属于无效证据。该约束保证不同 G2 实现测量
同一研究替身，同时不把研究替身冒充生产 IR。

### 2.4 精确研究预言机

研究执行器必须包含一个精确研究预言机（Exact Research Oracle）。它只使用标准库
`BTreeMap` / `BTreeSet`、直接标量转换和规范键遍历，不使用任何受测哈希器、候选
容器或候选排序实现；除不可变输入/输出值类型和第 3.3 节编码器外，不与受测管线共享
符号解析、关系展开或排序辅助函数。

- `N = 1`、`N = 2` 和全部研究夹具对照用例比较完整有类型记录与诊断；
- 每个正式规模级别在计时区外至少运行一次预言机，并比较完整计数、完整有类型输出、
  语义摘要和规范顺序；
- 预言机同样受研究停止护栏约束；若某级别无法完成预言机验证，该级别不能形成有效
  性能证据；
- 预言机时延和内存只作诊断，不进入候选排名或 #292 预算。

这样候选共同复用的错误不能只靠“候选彼此摘要相同”逃过正确性检查。G2 必须为预言机
和受测管线分别保留测试入口。

## 3. 统一计数与生成清单

### 3.1 每个结果必须报告的计数

每个工作负载级别（workload level）必须在计时前生成不可变清单，并至少报告：

- 来源模块数、导入边数、来源字节和来源字符串字节；
- 来源文档数、来源位置数、字符串项数、单字符串最大字节和总字符串字节；
- 各实体种类的声明数与必需身份字段出现数；
- 有类型抽象语法树、HIR、MIR、canonical LIR 各阶段记录数和逻辑字节；
- 符号数、引用解析数、关系出现项数、几何点数和诊断数；
- 输出记录数、输出逻辑字节和稳定语义摘要；
- 工作单元数、精确种子、生成器版本和生成清单摘要。

“逻辑字节”是各研究记录按冻结字段宽度求和的受控数据量；实际容量、分配器开销、
进程工作集和私有字节必须另行报告，不能混为一项。

### 3.2 版本化模块图与来源形状

三个合成工作负载都必须分别执行三种模块图配置档（Module Graph Profile），规模参数
`N >= 1` 表示拥有领域声明的工作单元数：

| 配置档                | 模块与导入边                                                                                           | 跨模块引用                                                                            |
| --------------------- | ------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------- |
| `wide-star-v1`        | `root` 依次导入全部 `unit/{i:08x}`                                                                     | `root` 按工作单元序号引用每个单元规范顺序第一项声明                                   |
| `deep-chain-v1`       | `root -> unit/00000000`，且 `unit/{i:08x} -> unit/{i+1:08x}`                                           | 除末单元外，每个单元引用下一单元规范顺序第一项声明                                    |
| `shared-fanin-dag-v1` | `root -> group/{g:08x}`；每组固定容纳六十四个单元；`group/g -> unit/i`；每个 `unit/i -> shared/common` | `shared/common` 声明一个不进入 LIR 的只读来源常量，每个单元恰好解析一次对该常量的引用 |

`shared-fanin-dag-v1` 的组数是 `ceil(N / 64)`；最后一组只包含尚存单元，不填充虚假
记录。三个配置档都没有循环，且扩大 `N` 不改变既有工作单元的模块名、命名空间、声明、
字段或引用。每个 `(workloadId, graphProfile)` 独立发现 `B`、正式阶梯和成本拐点，
不得用宽星形图的结果代表深链或共享扇入图。

- 模块名、命名空间和显式键由工作负载标识符、配置档、固定种子和规范模块名确定，
  不依赖总工作单元数 `N`；规范模块名必须覆盖 `root`、`shared/common`、
  `group/{g:08x}` 和 `unit/{i:08x}` 每个实际模块图节点，不能为非单元模块虚构
  保留工作单元序号；
- `root`、`group` 和 `shared/common` 不复制领域声明；共享常量只进入来源、Typed
  AST 与 HIR 计数，不进入 MIR/LIR 领域记录；
- 每个 `unit` 模块拥有一个工作单元，工作单元之间不共享可变状态；
- 每项来源声明、引用和关系都带有虚拟来源位置；来源文档键、行列和字节长度由机器
  可读清单冻结，不得由实现自行编造；
- 主阶梯使用 `short-unique-v1` 字符串配置；`LF-COMP-ID-v1` 还必须在 `B`、校准规模
  和压力规模执行 `shared-prefix-256-v1` 与 `long-4096-v1`，分别隔离共享前缀和长
  字符串成本。这些长度是研究输入，不是生产 `CompileLimits`；
- 导入、声明、引用、关系和几何输入顺序使用固定置换后再解析，输出必须恢复规范
  顺序；置换算法、字符串配置或来源位置规则改变时必须提升对应工作负载修订。

命名空间以
`BLAKE3-128(domain || generator_version_u32_le || base_seed_u64_le ||`
`len(workload_id)_u32_le || workload_id_utf8 || len(graph_profile_id)_u32_le ||`
`graph_profile_id_utf8 || len(canonical_module_name)_u32_le ||`
`canonical_module_name_utf8)` 派生，其中 `domain` 是
`"LF-COMP-NAMESPACE-v1\0"`。全部可变长字段均有显式小端长度前缀，因此不同字段
拼接不能产生同一前像；`BLAKE3-128` 取标准三十二字节输出按原顺序的前十六字节，
再编码为三十二字节小写 ASCII 十六进制，满足 identity v1 的
`authoringNamespaceId` 字段要求。输入
置换使用种子（seed）`0x4c46_434f_4d50_0001` 的 SplitMix64 和从末项到首项的
Fisher-Yates；
第 `i` 步交换位置
`i` 与 `next_u64 mod (i + 1)`。SplitMix64 使用公开算法的固定
`0x9e3779b97f4a7c15`、`0xbf58476d1ce4e5b9`、`0x94d049bb133111eb`
常量和 `64` 位回绕运算。每个展开模块的 import/declaration/reference/relation/
geometry 列表分别独立置换，序列种子固定为
`base_seed XOR (sequence_kind_u64 << 56) XOR module_seed_ordinal_u64`；
`sequence_kind` 的 `1/2/3/4/5` 分别表示导入（imports）、声明（declarations）、引用
（references）、关系（relations）和几何（geometry）。模块种子序号固定为：
`root = 0`、`shared/common = 1`、`group/g = (1_u64 << 40) | g_u64`、
`unit/i = (2_u64 << 40) | i_u64`；`g/i` 必须是精确 `u32`，第 `40..47` 位只标识
模块种类，第 `56..63` 位留给 `sequence_kind`，两域不得重叠。空/单项列表仍绑定
同一序列种子但不执行交换。因此 root 引用、group imports 和 unit imports 都不再由
实现自行选择顺序。G2 必须为每种模块图配置档的 `N = 1` 和 `N = 2` 发布全部展开
模块的完整已知向量。三种图分别测量宽扇出、深闭包和共享扇入，不把任意一种合成结构
写成现实城市模块结构。

### 3.3 研究语义摘要

候选等价在主计时区外先比较完整有类型输出，再计算 SHA-256 `semanticDigest`。摘要
输入是研究专用规范记录流，不复用生产规范制品或路网修订摘要：

```text
"LANEFLOW-COMPILER-CALIBRATION-SEMANTIC-V1\0"
stream_version_u32_le = 1
record_count_u64_le
repeated record_count times:
  record_kind_u16_le
  entity_kind_u16_le
  stable_id_16_bytes
  owner_ordinal_u32_le
  local_index_u32_le
  payload_length_u64_le
  payload_bytes
```

保留的缺失值是 `entity_kind = 0` 与 `owner/local index = u32::MAX`，但 v1 的
十三种现行记录都必须拥有恰好一个记录所有者（record owner）：`entity_kind`、
`stable_id` 和 `owner_ordinal` 均不得缺失，只有下表标为“无”的 `local_index`
使用 `u32::MAX`。`stable_id` 的十六字节没有缺失哨兵。

`owner_ordinal` 不是执行器自由分配的序号。对每个 `entity_kind`，先把该种类的全部
身份/声明记录按 `stable_id` 无符号逐字节升序排列，再取记录所有者的零起始同类序号；
每条记录的 `(entity_kind, stable_id)` 必须唯一解析到对应身份/声明，重算序号必须与
`owner_ordinal` 相等。这样 StableId128 与稠密序号同时受到摘要约束，而不会把被引用
实体误写成记录所有者。

记录种类码及 envelope 绑定冻结为：

| 代码 | 研究记录（Research Record）                   | `entity_kind + stable_id` 的记录所有者         | `local_index` 来源                      |
| ---: | --------------------------------------------- | ---------------------------------------------- | --------------------------------------- |
|    1 | 身份/声明（identity/declaration）             | 被声明实体                                     | 无                                      |
|    2 | 所有者关系（owner relation）                  | 子实体                                         | 无                                      |
|    3 | 边连接（edge connection）                     | 来源 `LaneEdge`                                | 来源边 `outgoingConnections` 的规范索引 |
|    4 | 路线出现项（route occurrence）                | 所属 `StaticRoute`                             | 载荷 `occurrenceIndex`                  |
|    5 | 规范几何点（canonical geometry point）        | 所属 `LaneEdge`                                | 载荷 `pointIndex`                       |
|    6 | 准入关系（access relation）                   | 所属 `AccessRule`                              | 规则内关系的规范索引                    |
|    7 | 信号组关系（signal group relation）           | 所属 `SignalGroup`                             | 信号组内 `ManeuverGate` 的规范索引      |
|    8 | 信号阶段状态（signal phase state）            | 所属 `SignalPhase`                             | 阶段内状态的规范索引                    |
|    9 | Gate 出现项（gate occurrence）                | 所属 `ManeuverPath`                            | 载荷 `occurrenceIndex`                  |
|   10 | 等待区出现项（waiting-zone occurrence）       | 所属 `ManeuverPath`                            | 载荷 `occurrenceIndex`                  |
|   11 | 停车锚点记录（parking anchor record）         | `ParkingSpace`；载荷同名 StableId 必须与其相等 | 无                                      |
|   12 | 车道覆盖出现项（lane coverage occurrence）    | 所属 `AuthoringLane`                           | 载荷 `occurrenceIndex`                  |
|   13 | 路口内部边角色（junction internal-edge role） | 所属 `Junction`                                | 路口内部边角色列表的规范索引            |

“规范索引”均在有类型引用解析完成后按记录所有者分组，把该集合的最终
`payload_bytes` 按无符号逐字节升序排列，再连续分配零起始索引；相同载荷的副本占据
连续索引，不使用来源顺序作破坏确定性的平局规则。载荷字段型索引则必须与 envelope
的 `local_index` 精确相等。被引用的 LaneEdge、Gate、SignalGroup 等只进入载荷，
不得替代上表的记录所有者。机器清单
`semanticRecordEnvelopeRules + recordKinds[].envelopeBinding` 是同一规则的可执行
登记；生产者与独立预言机必须各自按该登记生成并交叉验证。

载荷（payload）只使用显式小端定宽整数、规范浮点位、十六字节 `StableId128`，以及
`u32` 字节长度 + UTF-8 字节的字符串；序列使用 `u32` 计数后紧跟元素。输入
先拒绝 NaN/无穷值并把 `-0.0` 规范化为 `+0.0`。记录按
`(record_kind, entity_kind, stable_id, owner_ordinal, local_index, payload_bytes)`
逐字节升序。G2 必须发布各工作负载 `N = 1` 的完整记录流和摘要已知向量；编码或
记录码改变时必须提升记录流/工作负载修订（stream/workload revision）。

停车锚点记录每个 `ParkingSpace` 恰好一条，载荷依次编码停车位
`StableId128`、入口边 `StableId128`、入口 `EdgeProgress` 的规范 high/residual
两项 `f32` 位、出口边 `StableId128` 和出口 `EdgeProgress` 的规范 high/residual
两项 `f32` 位。该表示遵守 ADR 0014 已接受的目标数值权威，不把当前 JSON 的裸
`f64` 变成第二份输出权威。入口与出口即使落在同一边和同一有效 progress 也必须
分别编码；因此三个停车位仍产生三条记录，同时完整保存两种角色且不需要依靠相邻
记录配对。工作负载清单冻结
`high = canonicalF32(inputF64)`、
`residual = canonicalF32(f64(high) - inputF64)` 和
`effective = f64(high) - f64(residual)`；规范零统一为 `+0.0`。当前三个停车位的
四个不同 progress 数值都可由 `f32` 精确表示，因此其 residual 必须是 `+0.0`。

失败结果在主计时区外另计算 SHA-256 `diagnosticDigest`：先编码
`"LANEFLOW-COMPILER-CALIBRATION-DIAGNOSTIC-V1\0"`、
`diagnostic_stream_version_u32_le = 1` 和记录数，再按规范诊断顺序编码每项稳定
诊断代码、严重程度、`sourceDocumentKey`、起止行列和有类型载荷；每项仍使用小端
定宽值及长度前缀。
自然语言渲染文本、宿主绝对路径和哈希表迭代顺序不进入诊断摘要。G2 必须同时发布
一个未知引用和一个诊断截断已知向量。

### 3.4 机器可读工作负载清单

`../reference/compiler-calibration-workloads-v1.json` 是研究工作负载清单（Research
Workload Manifest）的机器可读 SSOT，冻结：

- 三种模块图配置档及其精确模块/导入/跨模块引用公式；
- 三种字符串配置、来源文档键和虚拟来源位置规则；
- identity v1 二十二种实体的字段绑定与父项拓扑；
- 研究记录 envelope、每种 `record_kind` 的记录所有者/同类序号/本地索引绑定、
  有序载荷字段和诊断流；
- 非生产研究阶段记录模型的字段宽度、`repr(C)` 大小、记录粒度、逐阶段记录/载荷/
  逻辑字节公式和工作负载每单元阶段输入；
- 来源令牌（source token）、模块/文档/导入/命名空间/配置键/引用/共享常量的字符串
  枚举与精确聚合公式；
- 三个可扩展工作负载的每单元实体、关系、出现项和几何计数；
- 当前夹具的文件路径、格式、精确字节长度、SHA-256、确定性研究投影、领域/阶段常量；
- 限制维度到唯一研究工作负载的绑定、失败变体、清理实验选择，以及每种结果必须满足
  的计数公式；
- 候选注册表修订、闭合候选标识符、允许键域、哈希器种子策略、算法常量，以及容器/
  哈希器/适配器/排序器的依赖组件身份。

设计正文解释语义，JSON 清单负责消除实现选择。两者冲突时视为 G1 缺陷，不允许 G2
自行选择；必须同步修订、提升受影响的 manifest/workload/stream revision，并重新
取得 G1。G1 候选冻结清单 `84389` exact bytes，SHA-256 为
`07ff1f5becae5bfb79586f5f7b35aca5c58a20042cd7b8586f7e63261062506b`；G2 只能发布
由该摘要输入产生的已知向量和研究证据。任何格式化或内容修改都必须同步更新长度、
摘要和 G1 审阅证据。

## 4. 工作负载契约

### 4.1 `LF-COMP-ID-v1`

目的：隔离身份编码、字符串驻留、符号登记、稳定排序和有类型序号分配。

一个工作单元恰好包含 identity v1 登记表的二十二种实体各一项，并包含登记表要求的
五十八个必需标签出现。父子实体按 `network-compiler.md` 的唯一所有者关系闭合；
所有字段值、父项求值拓扑和局部键字节由研究工作负载清单的 `identityBindings`
冻结，不允许 G2 自行挑选等价字符串。

机器清单的 `perUnitCounts` 必须逐项列出二十二种实体且每种值为 `1`；证据中的对应
逐实体计数必须全部等于 `N`，其和必须等于 `identityDeclaration = 22 * N`。只报告
聚合声明数不能证明各实体种类都被保留，也不构成有效 `LF-COMP-ID-v1` 证据。

冻结要求：

- 每个种类恰好一项，不通过大量复制某一种类歪曲 identity registry 成本；
- `LaneEdge` 使用独立 `laneEdgeKey`，不把可选 road/junction 角色写入身份；
- `RoadSection` 与 `FacilityBand` 分别拥有恰好一个 `RoadCorridor` 所有者；
- 所有父锚点先以完整 `StableId128` 解析，再编码子身份；
- 输入声明顺序按固定置换打乱，输出按 `(entityKind, StableId128, owner-local key)`
  恢复规范顺序；
- 每个候选必须产生完全相同的二十二项 StableId128 向量和语义摘要。

主归一化分母是身份字段出现数；实体数、字符串字节和符号数同时报告。

### 4.2 `LF-COMP-CORRIDOR-v1`

目的：隔离道路走廊所有者树、车道边连接、路线出现项、横断面/准入关系、信号关系、
几何展开和全局规范排序。

一个工作单元以仓库当前
`examples/data/v0.10-signalized-corridor.laneflow.json`、配对
`examples/data/v0.1-signalized-corridor.spatial.json` 和
`examples/data/v0.10-parking-signals-baseline.laneflow.json` 的**结构计数**为来源；
研究工作负载清单固定这些文件的精确字节长度与 SHA-256，并按其中登记的数组顺序为
每种实体生成零起始八位十六进制局部键；引用随原引用映射到对应局部键。G2 可以在
计时区外从固定文件构造一个常量大小单元配方，但每次规模运行不得解析 JSON，也不得
把原始 ID 或路径复制进领域键。停车基线没有配对 SpatialPackage；其三条 LaneEdge
在研究生成器中各使用两个清单冻结的规范点，并与信号化走廊共同绑定到每工作单元
唯一的 CanonicalFrame。合并后的基线计数冻结如下：

| 计数对象                                                                                    |     每工作单元 |
| ------------------------------------------------------------------------------------------- | -------------: |
| 车道图边（LaneEdge）/边连接（edge connection）                                              |        69 / 66 |
| 路口（Junction）/机动（Movement）/机动路径（ManeuverPath）                                  |    3 / 26 / 34 |
| 静态路线（StaticRoute）/路线边出现项（route edge occurrence）                               |       30 / 120 |
| 车辆参数配置（VehicleProfile）/参与者类别（ParticipantClass）                               |          3 / 4 |
| 道路走廊（RoadCorridor）/道路区段（RoadSection）/车道组（LaneGroup）/设施带（FacilityBand） | 8 / 15 / 6 / 7 |
| 编制车道（AuthoringLane）/车道覆盖出现项（lane-edge coverage occurrence）                   |        35 / 36 |
| 准入规则（AccessRule）/准入关系出现项（access relation occurrence）                         |        18 / 18 |
| 停止线（StopLine）/机动门（ManeuverGate）                                                   |        21 / 34 |
| 信号组（SignalGroup）/信号控制器（SignalController）/信号阶段（SignalPhase）                |     9 / 3 / 27 |
| 信号阶段状态出现项（signal phase state occurrence）                                         |             99 |
| 停车区域（ParkingArea）/停车位（ParkingSpace）                                              |          1 / 3 |
| 规范坐标框架（CanonicalFrame）                                                              |              1 |
| 规范几何点（canonical geometry point）                                                      |           1398 |

冻结要求：

- 八棵 RoadCorridor 所有者树必须完备，RoadSection 与 FacilityBand 不得出现零所有者
  或多所有者；
- 六十六条 edge connection、120 个 route edge occurrence、三十六个车道覆盖出现项、
  九十九个 signal phase state occurrence 和全部 signal relation 使用有类型引用
  解析，不允许字符串留在 MIR/canonical LIR 形状记录；
- 几何点只执行单位检查、规范坐标复制、边长累计和边范围（edge range）展开；本工作负载
  不测试曲线细分算法；
- 不复制当前夹具的原始 ID 或文件路径；命名空间和显式键由生成清单确定；
- 每个工作单元的领域记录比例固定，改变比例必须提升工作负载标识符。

主归一化分母是 canonical LIR 形状关系与几何记录总数；各领域计数必须单列，禁止只
报告一个未解释“对象数”。

### 4.3 `LF-COMP-JUNCTION-GRID-v1`

目的：隔离密集路口中 ManeuverPath、多 ManeuverGate、WaitingZone、StopLine 与路线
出现项预计算，放大关系基数和规范排序压力。

每个工作单元是一个四进口方向、禁止掉头的路口单元。四个进口方向分别连接其余三个
出口方向，因此固定形成十二个有向 Movement。每个 Movement 只有一条四 edge
ManeuverPath，并沿路径放置三个 ManeuverGate 和两个相邻门之间的 WaitingZone；
每个 ManeuverGate 恰好关联一条独立 StopLine，因此可推导三十六条 StopLine。

| 计数对象                                                        |   每工作单元 |
| --------------------------------------------------------------- | -----------: |
| 进口/出口车道图边（approach/exit LaneEdge）                     |            8 |
| 等待阶段内部车道图边（waiting-stage internal LaneEdge）         |           24 |
| 路口（Junction）/机动（Movement）/机动路径（ManeuverPath）      |  1 / 12 / 12 |
| 静态路线（StaticRoute）/路线边出现项（route edge occurrence）   |      12 / 48 |
| 停止线（StopLine）/机动门（ManeuverGate）/等待区（WaitingZone） | 36 / 36 / 24 |
| 规范几何点（canonical geometry point）                          |           64 |

几何只为每条 LaneEdge 提供两个规范点，避免本工作负载重复承担走廊几何压力。单元
按固定宽度 `4096` 的行优先网格布置：`x = unit_index mod 4096`、
`y = floor(unit_index / 4096)`；扩大 `N` 不改变既有坐标。网格坐标只影响规范几何，
不改变工作单元内关系比例。不同单元不连接车道图，防止边界单元改变固定计数；跨模块
成本由第 3.2 节配置档承担。

这一工作负载表达关系密度，不表达信号控制策略、冲突裁决正确性、排队溢流或中国
特色城市交通代表性。

主归一化分母是 ManeuverGate、WaitingZone 与 route edge occurrence 的总出现数。

### 4.4 `LF-COMP-RESEARCH-CURRENT-FIXTURES-v1`

目的：证明研究管线能够表达当前固定夹具的关键结构，校验错误定位与小输入固定成本。
它不是可放大的规模负载。

固定用例（case）：

1. `signalized-corridor`：
   `examples/data/v0.10-signalized-corridor.laneflow.json`、
   `examples/data/v0.1-signalized-corridor.spatial.json` 与
   `examples/data/v0.1-signalized-corridor.scenario.json`；
2. `parking-signals-baseline`：
   `examples/data/v0.10-parking-signals-baseline.laneflow.json`；
3. `multi-gate-waiting-zone`：
   `examples/data/v0.10-multi-gate-waiting-zone.laneflow.json`。

执行器在计时区外读取当前 JSON，按当前生产加载器（loader）得到对照摘要；研究管线另行从
冻结的等形输入构造阶段记录。两者只比较本文明确登记的实体、引用、顺序、数值和
关系摘要。未被研究内核实现的生产语义必须列为“不比较”，不得静默视为等价。
ScenarioManifest v0.1 只承担当前制品配对和来源沿袭对照，不进入编译器领域对象计数。

研究投影（research projection）按以下规则闭合：

- `traffic` 模块按清单登记的 JSON 数组顺序枚举二十二种身份实体；嵌套车道和信号相位
  使用父数组顺序再使用子数组顺序；
- 十一种关系记录分别由连接、路线、owner、信号、停车和待转区字段枚举；没有进入
  清单 `relationSources` 的生产字段一律是“不比较”，不能隐式生成记录；
- 含 SpatialPackage v0.1 的用例增加 `spatial` 模块；按 `trafficEdgeId` 连接到
  `traffic` 的 `LaneEdge` 声明顺序，再按 `centerline.points` 数组顺序生成规范几何点，
  同时登记六十六条跨模块引用；
- 原始 ID 只在计时区外用于解析当前夹具引用；计时区内身份键固定为
  `fixture/{entityKindCodeHex2}/{sourceArrayOrdinalHex8}`，来源引用固定为三十字节
  规范拼写。ScenarioManifest 不生成任何模块或阶段记录。

三个用例的闭合聚合输入如下；“引用”已经包含身份前像、关系载荷、几何与跨模块引用，
不是只统计 JSON 中名为 `*Id` 的字段：

| 用例                       | 模块/导入/跨模块引用 | 身份声明/身份字段/配置键 |  来源引用/关系/几何 | 语义输出记录 | 语义载荷字节 |
| -------------------------- | -------------------: | -----------------------: | ------------------: | -----------: | -----------: |
| `signalized-corridor`      |         `2 / 1 / 66` |        `330 / 945 / 378` | `2374 / 597 / 1392` |         2319 |        84624 |
| `parking-signals-baseline` |          `1 / 0 / 0` |           `27 / 76 / 31` |       `59 / 31 / 0` |           58 |         3026 |
| `multi-gate-waiting-zone`  |          `1 / 0 / 0` |           `18 / 47 / 20` |       `40 / 21 / 0` |           39 |         1984 |

机器清单还逐 case 冻结来源字节、字符串项数/最大项/总字节、来源位置，以及
`sourceInput`、Typed AST、HIR、MIR、canonical LIR、diagnostics、scratch 和
`outputConstruction` 的记录数/逻辑字节。证据 Schema 通过精确常量约束
（exact-constant constraints）固定这些领域计数与阶段结果；独立验证器仍必须从文件
与投影规则重新枚举，不能只回抄常量。

固定夹具文件摘要、字节长度、格式版本和来源提交必须进入证据。文件内容改变
时，即使文件名不变，也必须形成新的输入摘要；不得覆盖历史结果。

`LF-COMP-RESEARCH-CURRENT-FIXTURES-v1` 不参与 `N` 的规模发现、校准规模、压力
规模或候选排名，只表示 #308 研究内核对上述三个夹具的静态结构/诊断对照。

`LF-COMP-CURRENT-EQUIV-v1` 保留给 #292 的生产编译器、集成专用投影和当前态预言机
端到端等价矩阵；#308 不定义或扩大它。#292 rebase 后必须让
`compiler-foundation.md` 的工作负载表显式引用本设计和研究证据，并继续为生产矩阵
冻结独立用例、投影记录、行为、事件与空间采样范围。研究夹具集合或生产投影集合任一
发生变化，都必须提升各自的 workload revision，不得跨 ID 继承旧证据。

## 5. 规模发现与拐点规则

### 5.1 为什么不预设绝对对象数

合成工作单元只冻结结构比例，不冻结“参考城市”规模。绝对级别由 R0 上的测量稳定性
和资源曲线发现：

1. 每个 `(workloadId, graphProfile)` 从 `N = 1` 开始；
2. 只按二倍扩大；
3. 先寻找可可靠测量的编译器测量基准规模 `B`；
4. 正式阶梯至少覆盖 `B、2B、4B、8B、16B`；
5. 未发现拐点时继续按二倍扩展，直到拐点确认或研究停止护栏触发。

任何结果都必须同时报告 `N` 和第 3.1 节的精确领域计数，不能只写 `B` 或倍数。

### 5.2 编译器测量基准规模 `B`

执行器启动时先测量单调时钟分辨率：连续读取时钟，记录至少十万个非负差值，以最小
正差值 `q` 作为本次运行的观测时钟量子。候选 `N` 必须用相同生成清单运行七个独立
新进程试运行样本（fresh-process pilot samples），并满足：

- 七个有效样本的单次完整管线时延中位数不小于 `10000 * q`；
- 时延中位绝对偏差除以中位数不大于 `2%`；
- 七个输出语义摘要完全一致；
- 未触发研究停止护栏。

首个满足条件的二次幂 `N` 定义为该工作负载在该环境的 `B`。`10000 * q` 让时钟
量化误差上界远小于 `2%` 稳定性阈值；两者是测量质量条件，不是性能预算。
`B` 只由第 9.2 节的基线配置发现；其他候选必须复用同一组 `N`，不得为自己另选更
有利的基准规模。

`baseScales[]` 不是自由 `(workloadId, graphProfile, b)` 断言。每条记录的自然身份是
`(candidateId, workloadId, workloadRevision, graphProfile, stringProfile,
generatorVersion)`，其中候选固定为完整管线基线、字符串配置固定为
`short-unique-v1`。`selectionRule` 固定为
`first-power-of-two-qualifying-seven-pilot-runs-v1`；`pilotLevels[]` 若非空，必须从
`N = 1` 开始按严格二倍递增，直到首次合格或下一候选受停止护栏阻止。

每个已完成候选级别保存一条基准规模试运行摘要（base-scale pilot summary）：精确七个
有效冷实例 `runId`、`median-and-mad-of-seven-exact-integers-v1`、墙钟中位数、墙钟
中位绝对偏差、`10000 * protocol.clockQuantumNs` 阈值、共同语义摘要、摘要一致布尔值、
护栏全清布尔值和 `qualifies`。七个运行必须是 `scaleRole = pilot`、基线候选、相同
工作负载键和 `N`、`sampleOrdinal = 0` 的全新进程；其 `workload.b` 使用
`null + base-scale-not-yet-selected`，不能提前写入最终选择。作废/重试运行保留在
`runs[]`，但不得进入七项贡献集合。

若已选择 `B`，`b.value` 必须等于最后一条、也是第一条 `qualifies = true` 的
`pilotLevels[].n`，之前所有级别必须为 false，`terminalGuardRunId` 固定为
`null + base-scale-selected`。若护栏前没有合格级别，`b` 固定为
`null + no-reliable-base-scale-before-guard`，全部已完成摘要必须为 false，并以
`terminalGuardRunId` 引用下一二倍级别的实际 `guard-preflight` 运行。独立验证器必须
扫描同一自然身份的全部 pilot 运行，核对 `N = 1, 2, 4, ...` 无跳级，重算七样本
中位数/MAD、摘要一致性、护栏事实和首次合格选择；漏掉较早合格级别、混入作废样本、
自由改写 `B` 或用无来源终止原因代替 guard 运行都必须失败。

如果在研究停止护栏前没有找到 `B`，该工作负载结果为“环境无法建立可靠基准”，
不得自行降低要求或填写预算。

### 5.3 正式规模阶梯

每个 `(workloadId, graphProfile)` 的基线候选至少运行五级正式阶梯。正式实验包含
五个新进程轮次
（fresh-process rounds）；每轮的每个级别分别使用一个非插桩时延进程和一个插桩
内存进程，因此每级最终得到五个冷实例时延样本和五个冷实例内存样本。若共有 `K`
个级别，第 `r` 轮（`r = 0..4`）的第 `j` 个用例固定为
`level[(j + r) mod K]`，避免所有大规模都集中在实验末端。每个进程：

1. 在计时区外生成并验证固定大小的配方/预期计数清单；
2. 记录一次冷实例编译；
3. 释放冷实例产生的全部语义结果，只保留执行器明确登记的无语义暂存容量；
4. 执行三次不计时的稳定容量预热；
5. 每次先释放上一轮语义结果，再记录七次稳定容量复用编译；
6. 验证每次摘要、记录计数和失败状态一致；
7. 记录该进程模式允许的指标和进程内存高水位。

每个新级别进入正式轮次前，先运行一次插桩护栏预检和一次计时区外预言机。正式轮次
中，偶数轮先运行插桩内存进程，奇数轮先运行非插桩时延进程。两种进程使用相同
release 编译优化、输入清单和语义检查，但：

- 非插桩时延进程使用正常分配器，只允许其墙钟值进入时延预算；
- 插桩内存进程使用计数分配器，只允许其分配/存续/保留字节进入内存预算，其墙钟值
  只作诊断；
- 性能分析器另用独立进程，时延和内存都不得混入正式预算。

稳定容量复用只复用暂存容器容量，不得复用符号解析、身份结果、规范排序结果或其他
语义结果；否则该样本属于增量编译，不得混入本研究。

每种进程模式、每级分别报告五个冷实例原始值、三十五个稳定容量复用原始值、每轮
中位数、跨轮中位数和中位绝对偏差。不得删除离群值；外部干扰导致整轮作废时，
必须保存作废原因并重跑该模式的完整轮次。进程正常退出只记录
`process.exitKind = success`，不自动令样本有效；作废轮次中的正常退出样本必须保留
为 `status = invalid` 并携带至少一项具名 `invalidationReasons`，不得进入中位数、
候选比较或其他派生结论。

每个冷实例原始值保存为 `sampleOrdinal = 0`；同一进程的七个稳定容量复用原始值按
执行顺序保存为 `sampleOrdinal = 0..6`。权威 JSON 必须为每个候选、完整测量分层、
指标、批次和轮次建立轮次指标汇总（round metric summary）：冷实例汇总恰好引用一个
有效原始 `runId`，稳定容量复用汇总恰好引用同一子进程的七个有效原始 `runId`，且
序号集合完整、无重号。轮次中位数是排序后中间项；轮次中位绝对偏差先对每项计算其与
中位数的整数绝对差，再取这些绝对差的中位数。

正式阶梯批次汇总（ladder batch summary）必须按 `round = 0..4` 引用恰好五个轮次
汇总，并对五个轮次中位数再次计算跨轮中位数和中位绝对偏差。上述样本数均为奇数，
所以全部结果保持精确整数；算法标识符固定为
`median-and-mad-of-exact-integers-v1`，没有浮点转换或舍入步骤。独立验证器必须从
原始运行重算两层结果；全部 `summaryId` 在整份证据中必须唯一。缺少原始引用、跨进程
拼接、错序、重号、作废样本或自报汇总不一致都使对应汇总无效。

### 5.4 确认成本拐点

对基线候选每对相邻正式级别，必须分别建立以下完整测量分层（measurement stratum）：

- `wall-time-ns + timing + cold-instance` 与
  `wall-time-ns + timing + stable-capacity-reuse`，都除以该级主归一化记录数；
- `peak-live-requested-bytes + memory` 的冷实例与稳定容量复用分层，都除以
  canonical LIR 形状输出记录数；
- `private-bytes + memory` 与 `commit-peak-bytes + memory` 的冷实例与稳定容量复用
  分层使用同一 canonical LIR 分母，但只作诊断，不直接触发拐点。

权威 JSON 的每条 `adjacentLevelRatios[]` 必须保存基线候选 ID、下级/上级各自完整
`measurementStratum`、指标、批次、下级/上级正式阶梯批次汇总 ID、规范化基准、五个
相邻级别轮次对（adjacent-level round pair）、精确中位比值和 `candidateKnee`。上下级
分层除 `N` 与对应规模角色外必须逐字段相等；工作负载、修订、模块图/字符串配置档、
生成器版本、`B`、用例、样本种类、二进制模式或键域不同都不得配对。两项正式阶梯
批次汇总 ID 必须分别解析为同一记录已引用五轮的下级/上级汇总，不能只让轮次引用
看似相符。

五个轮次对按 `round = 0..4`，把同一批次、同一轮、同一指标和上述上下级分层的两个
`purpose = formal-ladder` 轮次指标汇总一一绑定。比值方向固定为上级归一化中位数 /
下级归一化中位数，并以
`(upper_median * lower_normalizer) / (lower_median * upper_normalizer)` 的互素正整数
比值保存；四项乘数都必须严格大于零，算法使用无溢出的数学整数语义。五项精确比值按
交叉乘法排序后取第三项作为中位数，不经过浮点数或舍入。`pairingMethod` 固定为
`same-batch-same-round-adjacent-level-v1`，`aggregationMethod` 固定为
`median-of-five-exact-round-ratios-v1`。

batch 0 是候选发现批次，batch 1 是独立确认批次。某上级是**候选拐点**，需要 batch 0
至少一条分层满足：

- `wall-time-ns` 的五轮归一化比值中，至少四个大于等于 `1.10`，且比值中位数
  大于等于 `1.20`；
- `peak-live-requested-bytes` 的五轮归一化比值全部大于等于 `1.05`，且比值中位数
  大于等于 `1.10`。

`private-bytes` 与 `commit-peak-bytes` 的 `candidateKnee` 必须为 `false`，也不进入
`knees[]`。每条可触发指标的相邻级别对必须生成一条拐点评估（knee assessment）：
保存基线候选、指标、下级/上级完整分层，以及分别由下级/上级正式阶梯批次汇总 ID
组成的 batch 0 候选比值引用与 batch 1 确认比值引用。其规范身份是
`(candidateId, metric, lowerStratum, upperStratum)`，不得再使用自由 ID。

`candidateKnee` 必须等于所引用 batch 0 比值的判断；`confirmedKnee` 当且仅当 batch 0
与 batch 1 在相同指标、样本种类、二进制模式和其余分层字段上都满足同一条件。候选/
确认规模都由 `upperStratum.n` 唯一派生，不再重复保存可漂移的 `candidateN` 或
`confirmedN`。候选为 `false` 时，确认必须为 `false`，性能分析制品使用
`null + not-a-candidate-knee`；候选为 `true` 时必须保存一项能解析到
`artifacts[].kind = profiler` 的 SHA-256，说明主导分配、排序、哈希碰撞、缓存未命中或
尚未解释的机制。

独立验证器必须解析每个上下级 `roundSummaryId` 与正式阶梯批次汇总 ID，核对完整分层、
`round = 0..4` 集合和 batch 0/1 引用，重算规范化分母、五个精确比值、中位数、候选/
确认布尔值和上级规模；缺失引用、跨分层/跨轮拼接、分母错用、批次倒置、重复自然身份、
分析制品摘要无对应实物或自报布尔不一致均使该拐点证据无效。无法归因不取消已重复的
拐点事实，但不得据此选择私有容器。这些百分比只定义研究信号和复测触发，不是生产
回归 Gate。

### 5.5 校准规模与压力规模

- 编译器校准规模：首个确认成本拐点之前最大的已完成正式级别。
- 编译器压力规模：首个确认成本拐点级别；若护栏前没有确认拐点，则为护栏之前最大
  的完整级别，并明确标记“未观察到拐点”。

若首个可比较级别 `2B` 已确认拐点，则校准规模为 `B`、压力规模为 `2B`。若护栏使
正式五级无法完成，研究必须报告“不足以冻结规模”，而不是把未完成级别降格为证据。
若护栏前完成至少五级但始终没有确认拐点，压力规模取最大完整级别，校准规模取其
前一完整级别；两者都必须标记“未观察到拐点”，不得伪造线性区间以外的结论。

### 5.6 重复性包络与研究预算建议

基线配置的全部正式级别必须在环境审计合格的另一独立执行批次完整复测，不能只复测
疑似拐点。每批仍包含第 5.3 节的五轮。下式中的指标 `m` 只包含严格大于零的时延、
存续/峰值/保留字节和进程内存指标；可能为零的分配次数或诊断数报告精确值，不做
除法。对每个工作负载、模块图配置档、级别、模式和适用指标：

```text
batch_value = median(five round medians)
repeat_ratio = max(batch_value_A / batch_value_B,
                   batch_value_B / batch_value_A)
E_m = max(repeat_ratio over all completed non-guard cases)
observed_upper = max(all ten round medians from batch A and B)
suggested_R0_budget = observed_upper * E_m
```

`reproducibilityEnvelopes[]` 以指标作为自然身份，每个适用指标恰好一条记录。记录
必须绑定基线候选、固定聚合范围
`all-completed-non-guard-baseline-ladder-strata-v1`、产生全局最大双向比值的 batch 0/1
正式阶梯批次汇总 ID，以及约分后的精确正整数 `repeatRatio`。独立验证器必须扫描该
指标全部已完成、未受护栏影响的基线正式阶梯分层，重算每个双向比值和全局最大值；
自由指标名、只保存最终小数或把非最大分层登记为来源都必须失败。

`recommendations[]` 不再接受自由 `id`、单位文本或任意运行 ID 集合。v1 唯一允许的
闭合种类是 `recommendationKind = r0-budget-v1`；规范身份是
`(recommendationKind, candidateId, stratum, metric)`。每条记录必须保存完整测量分层、
指标、公式标识符、同一分层的 batch 0/1 正式阶梯批次汇总 ID、按指标自然身份引用的
重复性包络、从两项汇总所引用十个轮次中位数重算的 `observedUpper`、舍入规则/量子、
精确正整数建议值、闭合单位和非产品 SLA scope。

公式标识符固定为 `ceil-div-observed-upper-times-envelope-to-quantum-v1`。若
`E_m = E_num / E_den`，则：

```text
quantum = protocol.clockQuantumNs  when metric = wall-time-ns
          1                        for byte metrics
suggested_R0_budget =
  ceil((observed_upper * E_num) / (E_den * quantum)) * quantum
```

时延单位固定为 `nanosecond`，舍入规则固定为
`ceil-to-protocol-clock-quantum-v1`；字节单位固定为 `byte`，舍入规则固定为
`ceil-to-whole-byte-v1`。全部乘法和上取整使用无溢出的数学整数语义。证据不足时省略
该自然身份的建议并在报告中说明稳定原因，不得用 `null`、零、自由单位或无推导来源的
整数占位。`E_m` 是同环境实测重复性包络，不预设固定百分比。若某项 `repeat_ratio`
不能由后台干扰、热/功耗状态或测量错误解释，并大到改变候选/拐点结论，则该研究无权
冻结预算，必须先稳定协议并重跑两批。

线性区间增长斜率使用泰尔－森估计（Theil-Sen Estimator），但权威证据不得保存平台
`log2` 的 JSON 浮点结果。每个指标的 `growthSlopes[]` 必须引用两批各自在确认拐点
之前、按主记录数递增排列的全部正式阶梯批次汇总；每批至少三个级别，也就是至少两个
可比较相邻区间。每个点是
`(primary_record_count, metric_batch_median)`，两项均为严格正整数。

两点 `(x_l, y_l)`、`(x_u, y_u)` 满足 `x_l < x_u` 时，数学斜率仍定义为
`log(y_u / y_l) / log(x_u / x_l)`，但使用
`theil-sen-q16.16-nearest-ties-even-v1` 的纯整数算法编码。令 `D = 65536`：

1. 若 `y_u = y_l`，`slopeQ16_16 = 0`；
2. 否则令 `a = max(y_u, y_l)`、`b = min(y_u, y_l)`，符号
   `sign = +1`（`y_u > y_l`）或 `-1`；
3. 以任意精度整数找出最大的非负整数 `k`，使
   `a^D * x_l^k >= b^D * x_u^k`；这等价于正斜率幅值不小于 `k / D`；
4. 用
   `a^(2D) * x_l^(2k+1)` 与 `b^(2D) * x_u^(2k+1)` 比较精确中点：
   左侧较小则取 `k`，较大则取 `k + 1`，相等时取两者中的偶数；
5. 最终 `slopeQ16_16 = sign * rounded_magnitude`，必须落在有符号三十二位整数范围，
   否则该研究分层失败关闭。

该算法不调用宿主浮点、对数库或容差。每批必须保存所有无序级别对的 lower/upper
`ladderBatchSummaryId` 与 `slopeQ16_16`；独立验证器重算组合数
`L * (L - 1) / 2`、每一对和排序结果。泰尔－森中位数以有符号 Q16.16
有理斜率（signed Q16.16 rational slope）保存：奇数项取中间整数，偶数项取两个
中间整数的精确平均，约分后的 denominator 只能是 `1` 或 `2`，
`fractionalBits = 16`，其真实值是
`(numerator / denominator) / 65536`。

两批分别计算，不合并原始样本；建议斜率上界使用精确有理运算
`max(slope_A, slope_B) + abs(slope_A - slope_B)`，以同一
`signed Q16.16 rational` 表示。权威 JSON 同时保存两批完整 summary 引用、两两斜率、
批内中位数、上界公式标识符和上界值。低于两个可比较区间时不生成该
`growthSlopes[]` 记录，只在建议中报告稳定原因码 `fewer-than-two-comparable-intervals`，
不得填写零或浮点近似斜率。

上述公式只生成 #292 可审阅的 R0 研究预算建议。#292 可以接受、收紧或因生产语义
重新测量，但任何修改都必须记录证据和理由，不能把停止护栏或未解释整数替代该来源。

## 6. 研究停止护栏

### 6.1 R0 默认护栏

停止护栏保护研究机器并限制实验成本。执行器必须使用父协调进程（Coordinator）和
一次性子进程（Child）两层，而不是让可能 OOM 的受测进程自行保存最后证据。
父进程在启动每个级别/样本前：

1. 根据研究工作负载清单计算下一级全部精确记录数、逻辑字节、字符串字节、输出字节
   和最大有类型序号，并按清单 `guardPredictionContract` 重算清单单缓冲区下界
   （manifest single-buffer lower bound）：对 source input 的 `logicalBytes`、Typed
   AST/HIR/MIR/canonical LIR 的 `recordAllocationBytes`、diagnostics/scratch/output
   construction 的 `logicalBytes` 取最大值；
2. 分别预测编译器控制峰值存续字节、进程私有字节和完整管线墙钟。有上一完整级别时，
   后两者按主记录数比例线性外推并乘以 `1.25` 安全因子；编译器控制字节的历史外推项
   精确为
   `ceil(previousPeakLiveRequestedBytes * nextPrimaryRecordCount * 5 /
   (previousPrimaryRecordCount * 4))`，最终预测取该项与清单单缓冲区下界的最大值；
3. 首级没有历史观测时，编译器控制字节预测精确等于清单单缓冲区下界，三个历史字段
   使用 `null + first-level-no-completed-level`；私有字节和墙钟使用结构化
   `null + first-level-monitor-only`，由父进程硬监控兜底。协议 v1 不存在独立“候选
   已知固定开销”输入，执行器或候选不得自行测量、填写或调节该自由量；
4. 把本机精确阈值、三项预测依据、前一级原始墙钟/私有字节/存续字节、输入清单摘要
   和预期计数写入父进程证据；
5. 任一非空预测值达到对应阈值、系统可用物理内存低于总物理内存的 `25%`、计数/字节
   `checked_add` 或 `u32::try_from` 失败时，拒绝启动子进程。

所有乘法使用受检 `u128`；上取整除法从商与非零余数求值，不使用可能溢出的“分子加
分母减一”，最终结果以 `u64::try_from` 收窄。若预测本身发生受检算术失败，证据使用
`checked-arithmetic-failed + null + reason`
保存负面事实并以 `trigger = checked-arithmetic` 拒绝启动；若历史监控缺样，则私有
字节/墙钟依据使用 `unavailable-invalid-sample`，该级别无权形成正式阶梯证据。
Schema 必须允许记录这些无效实验，但 G2 独立验证器不得把它们计为有效停止。

子进程接收不可提高的受控分配硬上限（Controlled Allocation Hard Ceiling）：

- 编译器控制请求字节上限是
  `min(floor(physical_memory_bytes / 4), 16 GiB)`；
- 进程私有字节监控上限是
  `min(floor(physical_memory_bytes / 3), 24 GiB)`；
- 单次完整管线墙钟上限是 `60 s`；
- 所有与规模成比例的 `Vec`、map、字符串、排序暂存区、诊断和输出容量必须先用
  `checked_*` 与 `try_reserve`，或使用同等失败关闭的有界研究 arena；禁止依赖
  `push`/隐式扩容越过硬上限后再观察；
- 子进程在每次容量请求前先原子预占请求字节额度；预占将越过硬上限时返回稳定的
  `guard/allocation-hard-ceiling`，释放已经取得的额度并正常退出；
- 父进程独立监控墙钟、系统可用内存、子进程私有字节和退出状态；到达监控上限时终止
  子进程并把该样本标为无效，不能把被强杀的部分数据写成有效护栏证据。

“受护栏终止”只允许两种有效状态：父进程在启动前拒绝，或子进程通过受检容量请求
正常返回稳定护栏错误。操作系统 OOM、分配器 abort、panic、超时强杀、监控失联或
非零异常退出一律是无效实验；父进程必须保存预检、最后监控快照和退出码，修复后从
上一完整级别重跑，不能声称护栏成功。

最后私有内存快照（last private-memory snapshot）是可空观察值，而不是用 `0`
代替未采集数据：

- `exitKind = guarded-before-start` 时，`childPid`、`exitCode` 和
  `lastPrivateBytes` 必须统一记录 `value = null`、
  `reason = "child-not-started"`，终止观察使用 `kind = not-started`；
- 子进程已经启动时，`childPid` 必须是实测整数；正常退出使用
  `termination.kind = exit-code` 和整数 `exitCode`。POSIX 信号终止使用
  `kind = posix-signal`、`exitCode = null + signal-termination`，并保存正整数信号号
  与 `posix-wait-status-hex-u32:<八位小写十六进制>` 原始 wait status；不能映射为
  退出码的平台终止使用 `kind = platform-status`、
  `exitCode = null + platform-status-without-exit-code` 和
  `native-status-hex-u64:<十六位小写十六进制>` 原始平台状态；
- `posix-signal` 与 `platform-status` 只能形成 `invalid-abnormal-exit` 或
  `invalid-monitor-termination`，不能冒充成功或子进程内受检护栏退出。除
  `trigger = monitoring-gap` 的无效样本外，`lastPrivateBytes` 仍必须是实测非负整数；
- `success` 与 `guarded-in-child` 都必须正常返回 `exitCode = 0`；前者只表示进程
  正常结束，运行状态可因轮次质量为 `valid` 或 `invalid`，后者为 `guarded`。以数值
  退出码结束的 `invalid-abnormal-exit` 或
  `invalid-monitor-termination` 必须使用非零退出码，并把运行状态标为 `invalid`、
  `invalidationReasons` 包含 `child-abnormal-exit`；
- 监控缺样允许保存 `lastPrivateBytes = null + reason`，但该运行必须保持无效，
  不得用它形成停止护栏或候选比较证据。

`16 GiB`、`24 GiB` 和 `60 s` 是 #308 R0 的运维安全上限；它们不进入
`CompileLimits` 建议，也不表示生产可接受成本。换机执行必须保存同一比例公式和
本机算出的精确字节阈值；改变公式需要提升研究协议版本。

### 6.2 禁止从护栏反推预算

压力规模触发护栏只说明 R0 不应继续扩大。#292 若需要冻结生产资源上限，必须结合：

- 已确认的线性区间和成本拐点；
- 实际生成记录与字节；
- 失败关闭能够在分配前执行的限制维度；
- 生产需求与产品硬件的后续证据。

不得简单使用护栏的 `25%`、`16 GiB`、`24 GiB` 或 `60 s` 作为生产通过条件
（production pass）数值。

## 7. 测量边界与内存记账

### 7.1 固定环境

首轮正式研究复用 `core-runtime-performance-baseline.md` 的 R0 **硬件角色**：

```text
设备: MECHREVO JIAOLONG
处理器: AMD Ryzen 9 9955HX, 16C/32T
物理内存: 61.68 GiB
操作系统: Windows 11 Pro Insider Preview build 29617
电源计划: 平衡
目标三元组: x86_64-pc-windows-msvc
工具链: Rust 1.96.0 / LLVM 22.1.2
```

这只复用机器、工具链和研究证据（Research Evidence）声明，不复用运行时规模、
tick/frame
预算或产品认证。每次正式执行仍须重新记录 CPU、物理内存、OS build、AC/电池、
厂商模式、电源计划、BIOS/固件（firmware）、工具链、来源提交、锁文件摘要和后台
进程。

### 7.2 计时边界

正式时延使用单工作线程 release 二进制。计时区包含：

- 按固定配方物化有类型抽象语法树形状记录和接收其所有权；
- 符号/引用解析、身份编码、阶段降阶；
- 关系/几何展开、规范排序和规范输出所有权落定；
- 成功或失败诊断的有界构造；
- 当前编译请求产生的输出对象构造。

计时区不包含：

- 进程启动、磁盘 I/O、JSON 解析和固定大小配方/预期计数清单生成；
- 完整输出与精确研究预言机比较、`semanticDigest`/`diagnosticDigest` 计算；
- 环境审计、证据 JSON 落盘和性能分析器启停；
- 第三方性能分析器自身开销；
- 把结果打印到终端。

固定大小配方和预期计数清单必须在计时前校验摘要；与 `N` 成比例的字符串、声明、
关系和来源位置仍在计时区内物化。输出所有权落定后先把结果交给等价于
`std::hint::black_box` 的消费边界并停止外层计时，再在计时区外执行完整输出比较和
摘要计算。摘要成本若需要归因，只能作为独立指标报告，不进入候选排名、`B`、拐点或
#292 预算建议。
正式时延进程不得启用阶段计时器、计数分配器、采样性能分析器或其他逐分配插桩；
只允许一个包围完整管线的外层计时区。这些分项指标由第 7.3 节和独立归因进程取得，
不得把分项时延相加冒充端到端结果。

### 7.3 内存口径

独立内存进程使用计数分配器或等价的进程内插桩（instrumentation）。固定大小配方和
清单验证完成后重置计数器，再由受测管线物化全部规模相关来源/阶段记录；分别报告：

- 当前请求分配次数、重分配次数、分配/释放字节；
- 当前和峰值存续请求字节（live requested bytes）；
- 编译结束后保留容量字节（retained capacity bytes）；
- 来源输入、各 IR 阶段、诊断、暂存区（scratch）、输出对象各自的存续/峰值；
- 进程工作集（working set）、私有字节和提交峰值（commit peak）。

“编译器控制总存续内存”必须包含编译请求期间同时存活的来源所有权、各阶段尚未释放
记录、字符串、诊断、暂存区和输出构造；不得只统计区块分配载荷（arena payload）。
进程指标只作外部
交叉检查，不能替代受控分配记账。

插桩二进制与非插桩二进制必须来自同一来源提交、Rust 工具链、编译配置（Cargo
profile）和特性集合（features）；证据记录各自二进制 SHA-256 与模式。插桩墙钟
不得与非插桩墙钟相加、比较或用于性能预算。

编译完成后允许保留明确的稳定容量，但必须区分：

- 必须随请求释放的语义结果；
- 可复用的无语义暂存容量；
- 分配器/操作系统无法立即归还的进程级内存。

## 8. 失败关闭与清理研究

### 8.1 受检限制维度

研究内核的私有限制结构必须覆盖、但不冻结未来公共 `CompileLimits` 类型：

- 模块、导入和来源字节；
- 声明、身份字段、符号、引用和关系出现项；
- 字符串项数、单字符串字节和总字符串字节；
- 几何点、路线出现项、ManeuverGate、WaitingZone 和诊断；
- 每阶段记录、暂存区字节、输出字节；
- 编译器控制总存续内存。

机器可读限制维度标识符固定为：

- 模块数（`module-count`）、导入边数（`import-edge-count`）、来源字节数
  （`source-byte-count`）；
- 声明数（`declaration-count`）、身份字段出现数
  （`identity-field-occurrence-count`）、符号数（`symbol-count`）、引用数
  （`reference-count`）、关系出现数（`relation-occurrence-count`）；
- 字符串项数（`string-item-count`）、单字符串字节数
  （`single-string-byte-count`）、总字符串字节数（`total-string-byte-count`）；
- 几何点数（`geometry-point-count`）、路线出现数（`route-occurrence-count`）、
  机动门数（`maneuver-gate-count`）、等待区数（`waiting-zone-count`）、诊断数
  （`diagnostic-count`）；
- 有类型抽象语法树记录数（`typed-ast-record-count`）、高级中间表示记录数
  （`hir-record-count`）、中级中间表示记录数（`mir-record-count`）、低级中间表示
  记录数（`lir-record-count`）；
- 阶段暂存字节数（`stage-scratch-byte-count`）、输出字节数（`output-byte-count`）、
  编译器控制存续字节数（`compiler-controlled-live-byte-count`）。

除诊断数外，每一维都需要“恰好等于限制成功”和“限制加一失败”的配对。诊断数不能
绑定零诊断的有效输入：它使用每单元一条未知引用、精确产生 `N` 条诊断的冻结变体；
`at-bound` 与 `plus-one` 都是编译错误，分别证明完整保存 `N` 条诊断与在
`maxDiagnostics = N - 1` 时保存 `N - 1` 条后稳定截断。长度、累计和容量计算必须使用
经检查算术（checked arithmetic）；普通限制在与请求大小成正比的分配、复制、排序或
哈希前尽早失败，诊断限制则必须在尝试发射超限诊断时立即停止且不得进入后续编译阶段。

清单为每个限制维度选择恰好一个能够自然产生非零目标计数的研究工作负载，禁止执行器
对所有工作负载做笛卡尔积或为不兼容工作负载虚构记录：

- `LF-COMP-ID-v1`：模块、导入、来源字节、声明、身份字段、符号、三项字符串、
  Typed AST、HIR 和编译器控制存续字节；
- `LF-COMP-CORRIDOR-v1`：引用、关系、几何点、路线出现项、MIR、LIR、
  阶段暂存字节和输出字节；
- `LF-COMP-JUNCTION-GRID-v1`：ManeuverGate 与 WaitingZone。

诊断数同样绑定 `LF-COMP-CORRIDOR-v1`，但必须使用
`corridor-missing-reference-per-unit-v1` 输入变体，不得使用零诊断的规范有效输入。

配对使用同一个规范工作负载级别和同一个清单输入变体：`at-bound` 把选中限制设为
该级精确值；`plus-one` 只把该限制设为精确值减一，使同一输入恰好超限一。完整私有
限制集合进入变体输入摘要，因此两项仍有不同摘要；这种做法隔离限制行为，不为“加一”
另造不属于工作负载的实体或阶段记录。

编译器控制存续字节数不能由受测候选自己的首次结果决定。该维度使用存续字节基准
预扫描（live-byte baseline prescan）：

1. 对每个 `LF-COMP-ID-v1` 的模块图配置档 × 校准/压力规模分层，固定
   `short-unique-v1`、全新进程和
   `baseline-std-randomstate-stable-vec-v1`；外部字符串表与已验证定长键表都使用
   标准 `HashMap + RandomState`，规范输出使用稳定 `Vec` 排序；
2. 以 `attribution` 二进制执行两个独立 `sampleKind = limit-baseline` 副本，只关闭
   `compiler-controlled-live-byte-count` 私有限制，继续执行运维受控分配硬上限和
   其余私有限制；
3. 两个有效副本的 `peakLiveRequestedBytes` 必须逐字节整数相等；不相等时该分层
   证据无效，必须先修复容量/生命周期非确定性，不能取最大值、均值或候选值；
4. 这个公共值是该分层唯一允许的 `exactDimensionValue`。`at-bound` 与 `plus-one`
   都复用同一基线配置，并分别设置
   `selectedLimitValue = exactDimensionValue` 与
   `selectedLimitValue = exactDimensionValue - 1`；
5. 两项限制证据必须保存两个预扫描 `runId`。独立验证器核对 workload/profile/
   `N`/`B`/scale、候选配置、二进制模式、全新进程、有效状态和峰值后才能接受配对；
   被评估候选不得提供自己的限制值。

失败用例标识符（failure case ID）冻结为：

- `limit/<dimension>/at-bound`：普通维度恰好等于限制时必须成功；`diagnostic-count`
  必须以稳定未知引用错误结束，完整保存 `N` 条诊断且不截断；
- `limit/<dimension>/plus-one`：对应输入值恰好比所设限制多一，必须在禁止的线性
  工作前失败；`diagnostic-count` 必须在第 `N` 条诊断面对 `N - 1` 上限时返回稳定
  诊断限制错误、保存 `N - 1` 条并设置截断标记；
- `semantic/missing-reference-per-unit`：只绑定 `LF-COMP-CORRIDOR-v1`，每个工作
  单元把规范顺序第一条路线出现项（route occurrence）改为未知 `LaneEdge`，必须产生
  按来源排序的有界诊断；
- `semantic/duplicate-owner-per-unit`：只绑定 `LF-COMP-CORRIDOR-v1`，每个工作
  单元把规范顺序第一个 `FacilityBand` 同时挂到第二个 `RoadCorridor`，必须拒绝
  多所有者；
- `diagnostic/cap-plus-one`：只绑定 `LF-COMP-CORRIDOR-v1`，设置
  `maxDiagnostics = N` 并构造 `N + 1` 个独立未知引用，必须只保存 `N` 个规范诊断和
  一个稳定的“诊断已截断”标记。

语义失败变体只改变列出的关系，其余生成清单字段保持相同，并保存独立输入摘要。
错误码、主诊断、截断标记和来源顺序属于候选等价比较；错误消息的非规范显示文本不
进入研究语义摘要。

每个 `sampleKind = failure` 样本都必须保存 closed 失败观察对象：失败用例标识符、
限制维度或明确的 `not-applicable`、变体输入摘要、预期/实际成败、稳定编译器错误码、
诊断数、诊断截断标志和部分输出记录数。`runId` 只标识一次运行，不能替代上述字段。
恰好等于限制的成功样本仍归入失败契约实验并记录 `expectedOutcome = success`；
这不会把成功结果误写成编译失败。

### 8.2 重复失败与恢复

在校准规模和压力规模分别执行：

1. 一次合法编译，保存摘要和保留容量；
2. 分别选择 `LF-COMP-ID-v1` 的 `limit/source-byte-count/plus-one` 作为最早资源
   预检，以及 `LF-COMP-CORRIDOR-v1` 的
   `semantic/missing-reference-per-unit`、`diagnostic/cap-plus-one`，三类失败各
   执行三十二次相同输入；
3. 每次验证稳定错误码、受限诊断数、无部分输出（partial output）；
4. 随后再次执行合法编译；
5. 验证摘要、记录数和错误状态与新实例（fresh instance）完全相同。

失败预热后的三十二次重复中，语义对象存续字节必须回到零；允许的暂存区保留容量
不得持续随失败次数增长。若发现台阶增长，必须保存每次原始容量并判定原因，不能用
最终进程退出掩盖泄漏。

同一清理实验使用稳定实验标识符关联基线成功、三十二次失败、恢复成功和新实例
判定基准；序号固定为基线成功 `0`、失败轮次 `1..=32`、恢复成功 `33` 和新实例
判定基准 `34`。每个清理样本还必须记录编译器实例身份
（compiler instance identity）。该身份由研究执行器在实例构造时一次性签发并随实例
存续，实例销毁后不得复用；不能从 `childPid`、内存地址或 `runId` 推断。同一 PID
内销毁并重建实例也必须获得新身份。序号 `0..=33` 必须共享同一实例身份，序号 `34`
必须使用不同身份；是否复用子进程不改变该规则。非清理样本必须显式记录
`phase = not-applicable`，且实验标识符、实例身份和序号都使用结构化不可用原因，
不能依赖 `runId` 命名约定推断。

三十二次是清理回归的固定观察窗口，不是生产工作量或性能预算。

## 9. 私有容器与哈希候选

### 9.1 按键域分组

容器选择不能跨安全域一刀切：

| 键域                                                         | 威胁/语义                               | 可进入比较的候选                                                                       |
| ------------------------------------------------------------ | --------------------------------------- | -------------------------------------------------------------------------------------- |
| 外部可控命名空间、符号、文档键                               | 哈希洪泛、长字符串、重复/碰撞输入       | 标准 `HashMap` + `RandomState`、`hashbrown` + `RandomState`、排序 `Vec` + 二分查找     |
| 已验证内部定长键、`StableId128`、有类型序号（typed ordinal） | 已受长度/基数限制，完整相等可低成本比较 | 上述候选、`hashbrown + XXH3`、`hashbrown + XXH64`、`hashbrown + FNV-1a 64`、`indexmap` |
| 规范输出顺序                                                 | 输出必须与哈希迭代顺序无关              | 稳定排序 `Vec`、确定性基数/桶排序候选                                                  |

XXH3、XXH64 和 FNV-1a 64 只作为非加密内部候选。完整键相等比较是必须条件，但不能
单独防止外部攻击者制造哈希洪泛；未通过资源上限、碰撞输入和模糊测试的快速哈希不得
用于外部可控字符串表。

机器清单中的 `candidateRegistry.revision = 1` 是候选身份的唯一注册表。每个候选
标识符精确绑定允许键域、哈希器种子策略、固定 seed 或算法常量，以及有序依赖组件
元组 `(role, implementationId, dependencyKind, dependencySource)`。证据中的
`candidate.components` 必须与注册表一一相等，不得缺失、追加或替换组件：

- 标准库组件的 `version` 必须等于证据环境中的 `rustc`，特性集合为空；
- 本地研究组件的 `version` 必须等于 `harnessCommit`，并由研究执行器工作树干净状态
  保证完整复现；
- crates.io / git 组件必须把精确 package ID、版本、启用特性和 checksum 绑定到同一
  `Cargo.lock`，同时完成许可证、MSRV 与安全公告审计。

未注册候选、候选 ID 与键域错配、快速非加密哈希进入 `external-string`、种子策略
错配或依赖组件集合不一致必须在候选排名前失败，不能通过自由字符串 ID 获得性能结论。

XXH3/XXH64 候选使用固定 seed `0x4c46_434f_4d50_0001`；FNV-1a 64 使用标准
偏移基数（offset basis）`14695981039346656037` 和质数（prime）
`1099511628211`。这些值只保证研究
可复现，不进入持久标识或规范输出。正确性测试必须另注入“所有键返回同一哈希值
（hash）”的受控构建器，证明碰撞时仍比较完整键、重复诊断和输出摘要不变；该受控
构建器不参加性能排名。

### 9.2 比较纪律

候选比较基线不是证据可选择字段。机器清单
`candidateRegistry.baselineByKeyDomain` 冻结：

| 键域（Key Domain）       | 唯一基线候选 ID              |
| ------------------------ | ---------------------------- |
| `external-string`        | `std-hashmap-randomstate-v1` |
| `validated-fixed-key`    | `std-hashmap-randomstate-v1` |
| `canonical-output-order` | `stable-vec-sort-v1`         |

- 标准随机化 `HashMap` 是前两个键域的功能与安全基线，不是假定性能赢家；稳定 `Vec`
  排序是规范输出顺序键域的基线；
- `baselineId` 必须等于所选键域的唯一登记值，`candidateId` 必须与它不同且在同一
  键域获准；把受测候选自身或其他优化实现当作基线必须在形成比值前失败；
- 完整管线基线配置在外部可控键和内部定长键都使用标准随机化 `HashMap`，规范输出
  使用稳定 `Vec` 排序；`B`、规模阶梯和拐点只由该配置发现；
- 候选比较每次只替换一个键域或排序组件，其他组件保持基线配置；组合赢家只有在各
  单项结果完成后才能另行复测，不能把多个同时变化的结果归因给一个候选；
- 工作负载生成种子（workload seed）与哈希器种子策略（hasher seed policy）是两个
  独立字段；所有候选消费相同生成清单、固定置换和 workload seed；
- 所有候选必须产生相同阶段计数、错误码、`StableId128` 向量和输出语义摘要；
- 同一键域有 `C` 个候选时，每批固定执行 `2C` 个平衡候选顺序轮次；前 `C` 轮的
  第 `r` 个顺序为 `candidate[(j + r) mod C]`，后 `C` 轮为
  `candidate[(r - j) mod C]`，其中 `r,j = 0..C-1` 且下标按 `C` 取模。这样每个
  候选在每个执行位置恰好出现两次，并同时覆盖正向/反向相邻关系；
- 每个候选样本使用一个全新子进程，每轮由全新父协调进程按冻结顺序依次启动；第二
  独立批次完整重复全部 `2C` 轮，不得只复测赢家；
- 标准 `RandomState` 的每个新进程密钥视为不可观测随机样本，只记录
  `hasherSeedPolicy = "random-state-process-random"`，不得把同轮结果称为同哈希种子
  的配对实验。XXH3/XXH64 等内部候选记录精确固定 seed，只有这些候选可以报告
  “相同哈希种子”；候选结论统一使用平衡轮次分层比值和跨轮分布；
- 每条候选比较必须保存候选比较分层（candidate comparison stratum）：键域、工作
  负载标识符/修订、模块图/字符串配置档、生成器版本、`N`、`B`、规模角色、当前
  样例用例、样本种类和二进制模式；不得跨任一字段聚合；
- `batch0` 和 `batch1` 分别固定
  `pairingMethod = same-batch-same-round-v1` 与
  `aggregationMethod = median-of-exact-round-ratios-v1`，并按 `round` 递增保存
  候选比较轮次对（candidate comparison round pair）。每个轮次对必须把同一批、同一
  `round`、同一完整分层中恰好一个基线轮次指标汇总与恰好一个候选轮次指标汇总配对；
  两个汇总必须由第 5.3 节的原始运行完整重算，且不能复用到该批的另一个轮次对。
  形成性能结论的每批必须覆盖 `r = 0..2C-1` 的全部 `2C` 个平衡轮次，作废轮次及其
  保留样本不得进入配对；
- 每个轮次对的比值方向固定为 `candidate round median / baseline round median`。
  `metric` 只允许
  映射到 `metrics.wallTimeNs`、`allocationCount`、`reallocationCount`、
  `allocatedBytes`、`peakLiveRequestedBytes`、`retainedCapacityBytes`、
  `workingSetBytes`、`privateBytes` 或 `commitPeakBytes` 的整数 `value`；分子、
  分母都必须严格大于零。比值以互素正整数 `numerator / denominator` 保存，不得先
  转换为浮点数或小数；
- 精确中位比值（exact median ratio）的算法固定如下：把该批 `2C` 个精确轮次比值用
  数学整数交叉乘法按数值非降序排列为 `x[0..2C-1]`，取
  `(x[C-1] + x[C]) / 2`，再以最大公约数约分为唯一互素正整数比值。排序、求和、
  除以二和约分使用无溢出的数学整数语义；证据生产者不能完成精确算术时必须失败
  关闭。该算法没有舍入步骤，候选决策与重复性包络比较也必须用精确交叉乘法；
- 独立验证器必须把每个轮次对的两个 `roundSummaryId` 唯一解析回轮次汇总及其原始
  运行，核对 batch、`round`、候选、完整分层、有效状态和指标，重算并约分每个比值，
  再独立重算
  `medianRatio`。正确性/安全拒绝必须引用造成拒绝的原始轮次对，并把不可计算比值
  保存为 `null + reason`；任何包含空比值、缺轮、重轮或非一一配对的批次只能形成
  拒绝或证据不足，不能形成性能赢家；
- 基线候选运行完整规模阶梯；其他候选至少运行 `B`、校准规模和压力规模；
- 单项内核（kernel）可作机制归因，但候选选择以完整研究管线结果为主；
- 候选的分层改善若未超过第 5.6 节同指标重复性包络，只能判为“噪声内无差异”；
  超过包络的候选必须在第二独立批次重现后才能形成选择建议；任一安全要求、语义
  摘要或失败清理不通过时，不论性能数值都淘汰；
- 不按统计离群值删除单个样本。只有出现预先声明的外部状态事件时才允许整轮作废：
  AC/电池、电源计划或厂商性能模式改变；系统睡眠/会话锁定；操作系统报告热/功耗
  throttling；非研究进程在该轮累计使用超过一个 CPU 秒或写入超过 100 MiB；监控
  缺样；子进程异常退出。作废事件、原始监控值和全部样本仍进入证据，并重跑该批的
  完整 `2C` 轮；环境无法提供某项监控时不得执行正式候选排名；
- 依赖版本、特性、许可证、MSRV、安全公告和锁文件摘要进入证据；
- 研究结果只向 #292 提供私有实现建议，不冻结公共哈希器或容器类型。

候选依赖审计对象无论结果是否合格都必须完整保存。缺失/不可识别许可证、未声明
MSRV、锁文件中缺少 package/checksum 或安全公告数据库不可用使用结构化原因和
`audit-unavailable` 状态记录；它们会使候选进入 `rejected-safety` 或
`insufficient-evidence`，但 Schema 不能为了只接受赢家而拒绝保存负面事实。只有
第三方候选才禁止把安全公告状态写成 `not-applicable`。

安全公告审计（security advisory audit）是闭合状态机：

- `no-known-advisories` 与 `advisories-present` 都是已完成状态，必须同时保存非空
  工具/版本、数据库快照身份和 UTC 观察时间；前者的 `advisoryIds` 必须为空，后者
  至少一项；
- 任一工具、数据库快照或观察时间不可用时，状态必须为 `audit-unavailable`，且至少
  一个对应观察使用 `null + reason`，不得保留已完成状态；
- `not-applicable` 只允许非第三方组件，三个来源观察都必须精确为
  `null + not-applicable`；
- 第三方组件只有许可证、MSRV、锁文件 package/checksum 均可验证且安全状态为
  `no-known-advisories` 时，才有资格形成 `noise-no-difference`、
  `repeatable-improvement` 或 `repeatable-regression`。`advisories-present` 必须
  `rejected-safety`；`audit-unavailable` 只能 `rejected-safety` 或
  `insufficient-evidence`。

独立验证器必须从绑定的 harness `Cargo.lock` 和审计制品重建上述来源，不得只信任
状态字符串。审计工具、数据库快照或观察时间全为空却自报成功，或者负面审计仍形成
性能赢家，都必须失败。

## 10. 机器可读证据

### 10.1 制品

G1 先发布非自指契约描述符：

```text
docs/reference/compiler-calibration-contract-v1.json
```

描述符从外部绑定证据 Schema 与工作负载清单的 `$id`/格式版本、路径、exact-byte
长度和 SHA-256；它不保存自身摘要，也不被证据 Schema 以 `const` 反向引用，因而
不存在自摘要循环。正式 runner 与独立验证器必须先从受信任 `sourceCommit` 加载该
描述符并计算其实际 SHA-256，再按描述符校验证据 Schema 和工作负载清单的精确字节，
最后才允许用该 Schema 解析证据。证据中的 `contractDescriptorSha256` 与
`evidenceSchemaSha256` 只记录实际计算结果；JSON Schema 只能检查其形状，独立
验证器必须在解析前把它们分别与外部描述符实际摘要和描述符登记值精确比较。任何
不一致都必须在读取派生结论前失败。

G1 候选冻结契约描述符 `1321` exact bytes，SHA-256 为
`a5899acace30e4e99bae09c070262054a73c7897f02debec24d0a73ccb9ea3f0`。该摘要是 PR/Gate
与独立验证器的外部启动输入，不写回描述符或证据 Schema。

G2/G3 研究交付拟生成：

```text
docs/reference/v0.10-compiler-budget-calibration-evidence.json
docs/reference/v0.10-compiler-budget-calibration-report.md
```

JSON 是原始与派生数值的权威证据；Markdown 只解释方法、图表、异常、候选裁决和
对 #292 的建议。JSON 绑定来源提交和全部外部分析制品；Markdown 记录同一来源提交、
JSON exact-byte 长度与 SHA-256，形成从报告到权威证据的单向绑定。JSON 不反向保存
报告摘要，避免循环摘要。

### 10.2 证据格式 v1（Evidence v1）必需字段

`../reference/compiler-calibration-evidence-v1.schema.json` 冻结对象层级、字段类型、
必需项、枚举、基数和 `null + reason` 表达；本节只解释主要语义，不替代 schema。
G1 候选冻结 schema `184794` exact bytes，SHA-256 为
`19723610659af78f934f498ade1f6caa6231c97fd8350c41785e29f5afcfa070`。
顶层格式标识：

```text
schema = "laneflow.compiler-calibration-evidence"
schemaVersion = 1
```

必需包含：

- `sourceCommit`、`harnessCommit`、必须为 `false` 的工作树脏状态（dirty-state）、
  `Cargo.lock` SHA-256、契约描述符标识符/版本/实际 SHA-256、研究工作负载清单
  SHA-256、证据 schema SHA-256；
- OS、CPU、物理内存、target triple、rustc、LLVM、AC/电池、厂商/电源模式、
  BIOS/firmware 和后台进程审计；
- 工作负载标识符/修订、模块图配置档、字符串配置、生成器版本、workload seed、
  `N`、`B`、规模角色（scale role）和当前夹具用例标识符；
- 每个合成工作负载/模块图的基准规模自然身份、冻结选择规则、从 `N = 1` 严格二倍
  递增的 pilot 摘要；每级精确七个有效运行 ID、墙钟中位数/MAD、时钟阈值、共同
  语义摘要、摘要/护栏布尔值、合格判断，以及选择 `B` 或终止 guard 运行；
- `workloadSeedHexU64` 必须精确等于工作负载清单的 `baseSeedHexU64`；证据 Schema
  以 `const` 绑定该值，禁止在保持清单摘要不变时替换置换和命名空间派生种子；
- 输入文件摘要/长度（仅当前等价用例）、生成清单摘要和全部精确领域计数；工作负载
  标识符必须条件约束其合法配置档、规模角色、`N`、用例和输入文件，不能只分别满足
  若干互不关联的枚举；
- 候选注册表修订、闭合候选标识符、键域、哈希器种子策略及可观测固定 seed，以及
  每个依赖组件的角色、实现身份、版本、特性（features）、依赖来源、许可证 SPDX
  表达式、MSRV、安全公告审计状态、工具/数据库快照/观察时间和锁文件 package
  identity/checksum；标准库或本地组件使用结构化不可用原因，不能省略审计对象；
- 每个原始运行的 `sampleOrdinal`，以及按候选、完整分层、指标、批次和轮次组织的
  轮次指标汇总；正式阶梯另须保存引用五个轮次汇总的批次汇总。两层都保存贡献
  `runId`/`roundSummaryId`、精确整数中位数和中位绝对偏差；
- 每条派生候选比较的完整候选比较分层、按两个独立批次拆分的同轮
  `roundSummaryId` 配对、精确轮次比值、中位比值和决策；汇总与运行引用必须能由
  独立验证器回算，不得只保存最终数；
- 每条限制配对的 `exactDimensionValue`、`selectedLimitValue`、值来源和来源
  `runId`；存续字节基准预扫描还必须保存基准测量标识符、副本序号和“仅运维硬上限”
  私有限制模式；
- 二进制 SHA-256 与模式（`timing` / `memory` / `attribution` / `profiler` /
  `oracle`）；
- batch/平衡轮次/执行位置/样本序号、父子进程 ID、结构化终止类别、条件化退出码、信号号与
  原始平台状态，以及每个样本的冷实例/稳定容量复用/失败时延、分配、存续/峰值/
  保留字节；
- 工作集、私有字节、提交峰值，以及来源输入、各 IR、诊断、暂存区和输出构造的
  逐阶段记录数、逻辑字节、归因时延与存续/峰值；
- 计时区外输出语义摘要、失败诊断摘要、时钟量子 `q` 和试运行稳定性；
- 每个失败契约样本的用例/维度、研究输入变体标识符、输入摘要、预期/实际结果、
  稳定错误码、诊断截断、部分输出，以及重复失败清理的实验标识符、编译器实例身份、
  阶段和一到三十二序号；
- 每轮电源/厂商模式、会话、热/功耗节流、后台 CPU/写入增量与监控缺样原始值，
  以及作废原因；
- 相邻级别的上下级完整测量分层、上下级正式阶梯批次汇总 ID、五个同批同轮
  `roundSummaryId` 配对、规范化基准、精确比值/中位数；每项可触发指标的 batch 0/1
  比值引用、候选/确认布尔值、自然身份与性能分析制品摘要；
- 每个适用指标唯一的重复性包络、产生最大双向比值的两批正式阶梯汇总、精确比值；
  每条 R0 预算建议的完整分层、闭合种类/单位/公式/舍入规则、两批汇总引用、包络引用、
  `observedUpper` 与精确建议值；
- 每项泰尔－森增长斜率的两批 `ladderBatchSummaryId` 全集、全部两两 Q16.16 斜率、
  批内精确有理中位数、上界公式标识符与精确有理建议上界；
- 停止护栏精确阈值、清单单缓冲区下界、三项预测依据、前一级/下一级主记录数、前一级
  墙钟/私有字节/存续字节、三项预测值、受控分配预占、是否触发、父进程监控快照和
  子进程退出状态；
- 性能分析制品的命令、字节长度、SHA-256 和保存位置。

所有字节字段使用完整十进制整数；GiB、秒或百分比只作为派生显示。未采集字段必须
使用 schema 定义的 `{ "value": null, "reason": "<稳定原因码>" }`，不得填零冒充
观测值。未知字段由 schema 的 `additionalProperties: false` 拒绝；改变字段含义或
层级必须提升 `schemaVersion`。

`counts` 按工作负载使用不同的闭合 Schema（closed schema），并要求共同模块/导入、来源文档/
来源位置、身份声明/身份字段/配置键、来源/字符串、诊断、语义载荷/输出/逻辑字节
计数及对应领域计数。当前固定用例还必须逐项保存二十二种实体、十一种关系记录与一种
几何记录，且 case 条件分支把这些计数冻结为精确常量（exact constants）。逐阶段记录数和
逻辑字节由必需的
`metrics.stageBreakdown` closed 对象承载，不在 `counts` 重复保存。JSON Schema
负责字段集合、类型、条件分支和基数，不能执行 `N × perUnitCounts` 之类跨字段算术；
G2 证据生成器与独立验证器必须分别从绑定摘要的工作负载清单重算公式，逐字段要求
精确相等，包括来源/字符串枚举、阶段聚合输入、记录数、载荷字节、逻辑字节、
`repr(C)` 大小和输出构造字节；固定用例的八阶段记录数/逻辑字节同时由 Schema 精确
约束（exact constraints）。验证器还要检查限制维度枚举、该维度绑定的唯一 workload、
研究输入变体、`caseId` 中的维度片段和独立 `dimensionId` 逐字节一致。只通过
Schema、没有通过该公式与交叉字段验证的
JSON 不是有效研究证据。

对研究语义记录流，独立验证器必须按
`semanticRecordEnvelopeRules + recordKinds[].envelopeBinding` 逐条重建 envelope：
先从身份/声明流重算每种实体的 StableId 排序与同类 `owner_ordinal`，再按记录种类
核对记录所有者、固定/动态 `entity_kind`、所有者 StableId 与 `local_index` 来源。
路线出现项不得以被引用 LaneEdge 代替所属 StaticRoute，其他关系/出现项同理；载荷
索引与 `local_index` 不等、停车位载荷 StableId 与 envelope 不等、哨兵使用越界或
所有者不能唯一解析时，完整输出与 `semanticDigest` 都必须判为无效。生产者与独立
预言机必须各自实现该映射，不能共享被测映射表后只比较同源结果。

独立验证器的启动根不是证据自报字段。它必须先取得 Gate 接受的契约描述符 exact-byte
身份，从 `sourceCommit` 读取描述符并核对实际摘要，再依次核对描述符登记的 Evidence
v1 Schema 与工作负载清单；此后才解析 evidence。解析后还要要求
`source.contractDescriptorSha256`、`source.evidenceSchemaSha256` 和
`source.workloadManifestSha256` 分别等于已验证的实际制品。任何字段只满足六十四位
十六进制形状、但不等于外部实物的证据都必须失败。

研究来源提交（research source commit；`sourceCommit`）绑定受信任契约描述符、清单、
Schema 与被测来源；研究执行器提交（research harness commit；`harnessCommit`）
绑定研究执行器、独立预言机以及研究包内的本地 adapter/sorter。两者可以不同，但两份
检出工作树（checkout）都必须干净，且 `cargoLockSha256` 必须来自构建
`harnessCommit` 研究执行器的锁文件，不能拿 `sourceCommit` 冒充本地组件版本。

候选验证必须从已绑定清单加载 `candidateRegistry`，要求证据候选的 registry revision、
ID、键域、种子策略/固定值和有序组件身份与唯一注册项精确一致；再按组件种类核对
rustc、`harnessCommit` 或 harness `Cargo.lock`
package/version/features/checksum。不能从自由 ID 推测算法，也不能把同名候选的不同
依赖实现合并比较。每条候选比较还必须按 `baselineByKeyDomain` 重算唯一
`baselineId`，要求候选与基线不同且两者都获准进入该键域。第三方组件的已完成安全
审计还必须有非空工具、数据库快照和观察时间；任一来源不可用时状态必须降为
`audit-unavailable`。验证器按许可证/MSRV/锁文件/公告结果约束候选决策，负面或不可用
审计不能形成性能赢家。

基准规模验证必须按 `baseScales[]` 自然身份扫描全部 pilot 运行，要求每条
`pilotLevels[]` 恰好引用同一基线候选、工作负载键和 `N` 的七个有效全新进程样本，
重算墙钟中位数/MAD、`10000 * clockQuantumNs`、语义摘要一致性、护栏清除和
`qualifies`。级别必须从一开始严格二倍递增；选中路径的 `B` 必须是第一条合格级别，
未选中路径必须引用下一候选级别的实际 guard preflight。漏记较早 pilot、混入作废/
重试运行或直接自报 `B` 必须失败。

轮次指标汇总验证必须按贡献 `runId` 重算。`cold-instance` 要求唯一
`sampleOrdinal = 0`；`stable-capacity-reuse` 要求同一子进程的
`sampleOrdinal = 0..6` 完整集合。正式阶梯批次汇总必须引用同一候选、分层、指标与
批次的 `round = 0..4` 五个轮次汇总，并重算跨轮中位数和中位绝对偏差。相邻级别
轮次对必须解析到除 `N`/规模角色外相同完整测量分层的上下级正式阶梯汇总，并重算
规范化比值和拐点阈值；相邻级别记录的上下级正式阶梯批次汇总 ID 必须与五个轮次对
闭合。每条拐点评估的 batch 0/1 引用必须分别唯一解析到相同候选、指标和上下级分层的
相邻比值；候选/确认值、上级规模和 profiler 制品引用都必须重算。候选比较轮次对则
必须解析到同批同轮的基线/候选轮次汇总。三者都不能绕过原始运行直接信任中位数。

增长斜率验证必须把每批 `levelBatchSummaryIds` 解析为同一基线候选、指标和除
`N`/规模角色外相同的完整测量分层，且分别来自 batch 0/1；按主记录数排序后枚举全部
级别对，使用第 5.6 节大整数幂比较算法重算每项 `slopeQ16_16`、批内精确有理中位数
和两批建议上界。缺少任一组合、引用拐点后级别、浮点数、未约分值或上界公式不一致
必须失败。

重复性包络验证必须按指标扫描全部已完成、未受护栏影响的基线正式阶梯分层，要求
batch 0/1 汇总分层相同，重算双向比值、约分结果和全局最大值。R0 预算建议必须把两项
`ladderBatchSummaryId` 解析为同一完整分层、指标和 batch 0/1，从其十个轮次汇总重算
`observedUpper`，再按自然身份解析同指标唯一包络，使用第 5.6 节公式和
`protocol.clockQuantumNs` 或字节量子 `1` 重算建议值。自由建议 ID、重复自然身份、
包络指标错配、跨分层汇总、错误单位/舍入规则或无来源整数必须失败。

停止护栏同样不能只通过单条 Schema：独立验证器必须按
`guardPredictionContract.primaryRecordCountByWorkload` 重算前后级主记录数，从八个
具名阶段操作数重算清单单缓冲区下界，并分别按首级恒等式或后续级别受检上取整公式
重算 `predictedCompilerControlledBytes`。证据中出现自由固定开销、下界操作数缺失、
跨工作负载主记录公式或不同整数舍入结果时，整条运行无效。

独立验证器还必须按清理实验标识符分组，要求每组恰好包含序号 `0`、`1..=32`、
`33`、`34`，且工作负载、候选、规模和失败用例在组内一致；序号 `0..=33` 的
`compilerInstanceId` 必须完全相同，序号 `34` 必须不同。验证器不得用相同
`childPid` 代替实例身份，也不得接受同一实例身份被不同构造事件复用；缺项、重号、
跨条件拼组、重复构造身份或 fresh oracle 身份未变化都必须失败。清理组只允许机器
清单登记的三个 case/workload/scale 组合，不能用另一个工作负载“等价替代”。它同时
检查运行状态与护栏事实相容：受检算术失败、监控缺样、异常退出或不可用预测可以留在
原始证据中，但不能被计入有效阶梯、重复性包络或候选改善结论。Schema 对单条记录的
closed 约束与独立验证器对整份证据的关系约束缺一不可。

对 `LF-COMP-ID-v1`，独立验证器必须重算二十二项逐实体计数并验证每项等于 `N`；
对 `compiler-controlled-live-byte-count`，它必须把两个 `basisRunIds` 唯一解析为
同一分层的两个有效 `limit-baseline` 运行，验证两者
`peakLiveRequestedBytes` 精确相等，并检查 at-bound/plus-one 的限制值分别为该值和
该值减一。任一来源运行缺失、重复、来自被评估候选或跨分层时，配对必须失败。

### 10.3 可复现命令

G2 必须提供锁定工具链和锁文件的单入口，形状为：

```powershell
cargo +1.96.0 run --release --locked `
  -p issue-308-compiler-budget-calibration-research `
  --no-default-features --features research-runner-full `
  --bin issue-308-compiler-budget-calibration-research -- `
  run --protocol compiler-calibration-v1 --output <evidence-path>
```

精确参数在实现 G1 不再重开本文语义的前提下由 G2 落地。正式结果不得来自 IDE
测试入口、debug 二进制或未锁定依赖。正式入口必须在启动任何测量前核验干净研究
工作树（clean research working tree）：`sourceCommit` 与 `harnessCommit` 所指向
的受测源码、runner、契约描述符、清单、Schema 和锁文件均不得有未提交修改；Schema 以
`dirty = false` 失败关闭。脏工作树只允许产生不使用 evidence v1、不能进入
`derived` 或报告的本地探索输出，不能通过绑定任意补丁摘要升级为权威证据。

## 11. 从研究证据到 #292

### 11.1 #308 必须输出的建议

研究报告至少给出：

- 三个合成工作负载 × 三种模块图配置档的 `B`、五级以上原始阶梯、校准规模、压力
  规模和拐点机制；
- 当前固定夹具等价矩阵和明确未比较项；
- 冷实例与稳定容量复用的时延、内存、分配和增长斜率；
- 失败关闭的最大前置工作量、重复失败清理和恢复结果；
- 各键域容器/哈希候选的相对结果、安全限制与候选依赖审计；
- 可直接被 #292 G1 引用的 R0 研究预算建议；
- 哪些问题仍需生产真实实现、产品硬件或中国特色城市工作负载证据。

### 11.2 #292 可以冻结的内容

#308 完成 G4 后，#292 G1 才可以基于精确证据提交（exact evidence commit）冻结：

- 具名 `CompileLimits` 的精确限制与失败顺序；
- 三个合成工作负载 × 三种模块图配置档的精确规模和回归用子集；
- 研究干净单工作线程编译的 R0 时延/内存证据，以及生产干净单工作线程编译必须重新
  测量的差额；
- 冷实例、稳定容量复用、失败清理和增长斜率门槛；
- 不泄漏到公共 API 的私有容器选择规则和拒绝服务边界。

这些仍是编译器实现研究/回归输入，不自动成为产品 SLA。#292 rebase 后必须：

1. 保留 glossary 中通用生产术语的统一定义，不用研究定义覆盖
   “干净单工作线程编译”；
2. 让 `compiler-foundation.md` 反向链接本设计和 exact evidence commit；
3. 让三个合成 workload ID 引用本设计/清单，不复制第二套计数；
4. 保留 `LF-COMP-CURRENT-EQUIV-v1` 的生产投影所有权，不与
   `LF-COMP-RESEARCH-CURRENT-FIXTURES-v1` 合并；
5. 在自身 G1 验收清单登记上述回写完成。

若 #292 的真实生产实现相对研究内核增加必要语义、验证或制品成本，必须在自身 G2/G3
重新测量，不得直接继承研究绝对时延。

### 11.3 不能冻结的内容

#308 不能单独裁决：

- 最低或推荐产品硬件；
- 真实城市可编制面积、道路长度或参与单元容量；
- 玩家在线编辑的可接受停顿；
- 中国特色城市工作负载的代表性；
- 增量/并行编译的产品预算；
- 可移植规范制品（portable artifact）、静态镜像构建或交通运行时（Traffic
  Runtime）切换成本。

这些需要后继产品工作负载、真实实现与对应 Gate。

## 12. G1 验收与推进

### 12.1 #308 G1 通过条件

- [ ] 本文 exact head 通过全面本地文档/架构审阅；
- [ ] 外部审阅未发现未回应的主要/阻断发现（Major/Blocking finding）；
- [ ] 研究工作负载清单 exact bytes 摘要已冻结，三个合成工作负载 × 三种模块图、
  字符串/来源位置类别、记录布局、计数对象、证据角色和禁止外推边界闭合；
- [ ] 候选注册表的闭合 ID、允许键域、种子策略、算法常量与依赖组件身份闭合；
- [ ] `B`、至少五级规模阶梯、拐点确认、校准/压力规模和停止护栏规则闭合；
- [ ] 冷实例、稳定容量复用、轮次/批次精确汇总、失败清理和内存记账协议闭合；
- [ ] 外部可控键与内部定长键的安全域、候选矩阵、进程隔离、平衡顺序和整轮作废
  纪律闭合；
- [ ] 非自指契约描述符、证据格式 v1 schema、研究/生产 workload ID 所有权与 #292
  rebase 回写边界闭合；
- [ ] 术语 SSOT、设计索引和相关 Agent Skill 阅读入口同步；
- [ ] Issue #308 留下可永久引用的 G1 判断评论。

### 12.2 G1 后的顺序

1. #308 G1 通过并合入本设计；
2. 用户明确授权 #308 G2；
3. 创建非生产研究包、正确性测试和证据写入器；
4. 先完成冒烟/试运行（smoke/pilot），再在 R0 执行正式阶梯；
5. 发布原始 JSON、报告、复现命令和审阅结论；
6. #308 按 G3/G4 收口；
7. #292 引用 exact evidence commit，冻结最终数值并继续自身 G1。

在第 2 步前不得实现研究代码；在第 6 步前不得用试跑数字解除 #292 的阻塞关系。
