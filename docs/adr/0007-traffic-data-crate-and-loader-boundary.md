# 0007 Traffic Data Crate and Loader Boundary

**状态**: Accepted  
**日期**: 2026-07-13  
**适用范围**: LaneFlow 外部数据格式、Rust crate 依赖方向、production loader 与 Core domain normalization 边界  
**后续修订**: ADR 0008 已替代本文的 v0.2/v0.3 双版本 loader/result 决策；crate 依赖、private DTO、normalization、I/O 与 validation 分层继续有效  
**关联文档**:

- 上游决策:
  - `0001-project-scope.md`
  - `0002-dependency-and-licensing-constraints.md`
  - `0004-core-implementation-language.md`
  - `0005-core-identity-and-handle-model.md`
  - `0006-vehicle-following-control-and-safety.md`
  - `0008-pre-1.0-data-format-version-policy.md`
- 详细设计:
  - `../architecture.md`
  - `../design/data-format.md`
  - `../design/data-loading.md`
  - `../design/vehicle-following.md`

## 背景

LaneFlow v0.2 已冻结 JSON-compatible lane graph 和 route package，并提供 checked-in JSON Schema。当前 Rust workspace 只有 `laneflow-core`；JSON DTO、Serde 和 schema validation 只存在于 integration test，不构成 production loader。

v0.3 将新增 Vehicle Profile、严格的 `0.3` package 和 profile handle registry。若直接把 JSON DTO 和 loader 放入 `laneflow-core`，Core 会同时拥有 wire format、JSON parser、domain model 和 runtime orchestration，后续 Adapter、authoring tool、importer 或 Web asset pipeline 都会被迫依赖这一混合边界。

另一方面，若 data crate 自行实现 duplicate route、unknown edge、route continuity 或 profile invariant，会与 Core 形成两套 domain validation。两套规则会在版本演进中漂移，使“schema 通过”“loader 通过”和“Core 可运行”产生不同结论。

## 决策

### 1. 新增独立 `laneflow-data` crate

Cargo package 名为 `laneflow-data`，Rust crate 名为 `laneflow_data`。依赖方向固定为：

```text
laneflow-data -> laneflow-core
laneflow-core -X-> laneflow-data
```

`laneflow-data` 负责 external package、JSON parsing、严格版本分流、units、wire shape、数据路径诊断和向 Core domain types 的转换。

`laneflow-core` 负责 lane graph、route、Vehicle Profile、typed handle、registry/resolver、runtime state 和全部 Core domain invariant。Core 不依赖 Serde、JSON、JSON Schema、文件系统或 Engine Adapter API。

### 2. Wire format 与 Core domain 只通过已验证类型衔接

data crate 的 wire DTO 保持私有。public loader 返回严格区分 v0.2/v0.3 的结果，调用方必须显式匹配版本；不得用统一 package、optional profile 或空 registry 作为格式版本判据，也不得为 v0.2 合成 default profile。

Core 新增 `InitialTrafficData`，作为 lane graph、初始 routes 和 immutable profile registry 组合不变量的唯一 public 入口。初始 route validation 与 runtime route registration 复用同一个 Core 内部规则，data crate 不重复实现 Core domain validation。

### 3. Loader 不拥有文件系统或 world lifecycle

production loader 只接收调用方提供的内存 JSON bytes/string，不读取路径、不选择同步或异步 I/O，也不直接创建 `CoreWorld`。

fixed tick、initial vehicles、spawn schedule、runtime route generation 和 Adapter asset metadata 不属于 data crate。Adapter、example 和后续 authoring tool 可以按需同时依赖 data/core 两个 crate。

### 4. Schema 与 production validation 分层

checked-in Draft 2020-12 schema 是外部结构契约。Serde/`serde_json` 负责 production JSON syntax 和 wire shape，Core constructors 负责 domain invariant。`jsonschema` 只用于 schema、fixture 和 integration test，不进入 production loader 或 fixed tick hot path。

`DataError` 负责 JSON path、line/column、version、units 和 Core source context；`CoreError` 负责 domain/runtime invariant。public error 保持结构化并允许未来扩展。

### 5. Authoring 与不可信输入延后到真实需求

v0.3 不公开 raw wire DTO 或 Rust serialization API，不设置固定 edge/route/profile 数量上限，也不承诺针对 hostile oversized JSON 的完整 denial-of-service 防护。

未来出现稳定 authoring、网络上传或不可信 mod 数据需求后，再单独设计 versioned builder、batch validator、`LoadLimits`、隔离 process 或流式 ingestion，不静默扩大本 loader 契约。

## 后果

正向后果：

- Core 保持 engine-agnostic，并与 JSON/Serde/文件系统解耦。
- v0.2/v0.3 通过类型边界显式分流，不会隐式补 default profile。
- Core 只有一套 route/profile domain invariant，schema、loader 和 runtime 不会各自解释。
- Adapter 和 Web 可以复用内存 loader，而不被固定路径或同步 I/O 绑定。
- 所有字符串解析和 profile handle 分配停留在加载阶段，不增加 tick hot path 成本。

成本与风险：

- workspace 增加一个 crate 和跨 crate integration tests。
- `InitialTrafficData` 会改变当前 Core 初始化 API，#73 必须明确 pre-1.0 breaking impact。
- private DTO 与 checked-in schema 可能漂移，需要 fixture/contract tests 锁定。
- fail-fast loader 不一次返回所有 authoring errors；批量诊断属于后续 tooling 设计。
- 不可信输入在进入 loader 前仍需调用方或后续安全层限制资源。

## 替代方案

### JSON loader 直接进入 `laneflow-core`

文件较少，但会把 Traffic Data wire format、Serde 和 parser 依赖带入 runtime crate，并扩大所有 Adapter 的依赖面，因此拒绝。

### `laneflow-core` 依赖 `laneflow-data`

可以让 Core 直接消费 DTO，但会让 runtime 依赖外部格式版本，并使 JSON/schema 变化反向影响 Core，因此拒绝。

### data crate 重复实现 domain validation

表面上可独立返回完整错误，但会形成两套 route/profile 规则并产生漂移，因此拒绝。Core constructors 和 `InitialTrafficData` 必须是 domain invariant 的唯一事实源。

### 一个统一结果类型表示所有格式版本

使用 optional profile 或空 registry 较简洁，但调用方容易把 v0.2 当作不含 profile 的 v0.3，削弱严格版本闸口，因此拒绝。

### Production loader 始终执行 JSON Schema validator

可以复用 schema，但会增加生产依赖、启动成本和第二套错误语义。Serde shape validation 加 Core domain validation 足以构成 runtime loader；完整 schema validation 保留在测试和 authoring validator，因此拒绝。

### 立即公开 wire DTO 与 serialization

当前没有稳定的 Rust authoring API 需求。提前公开会冻结构造方式和 Serde 布局，因此拒绝。

## 实施与复核

- #73：实现 `laneflow-data`、v0.2/v0.3 loader、Vehicle Profile、registry/resolver、`InitialTrafficData`、schema 和测试迁移。
- #74：让 VehicleState、spawn input 和 CoreWorld 初始化消费 profile handle 与 `InitialTrafficData`。
- API、validation、error、测试与输入安全的可执行约束见 `../design/data-loading.md`。

若未来改变 crate 依赖方向、让 Core 依赖 wire format、公开 raw DTO，或把 schema validator 带入 production loader，应新增或 supersede 本 ADR，不得静默修改。
