# Data Loading 设计

**文档状态**: Accepted  
**最后更新**: 2026-07-13  
**适用范围**: v0.2/v0.3 JSON package 的 Rust loader、严格版本分流、Core normalization、错误模型、测试与输入安全边界  
**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0006-vehicle-following-control-and-safety.md`
- `../adr/0007-traffic-data-crate-and-loader-boundary.md`
- `data-format.md`
- `vehicle-following.md`

## 1. 目标与非目标

目标：

- 为 v0.2/v0.3 external package 提供唯一 production JSON loader。
- 保持 `laneflow-data -> laneflow-core` 单向依赖。
- 在 public API 中严格区分格式版本。
- 让 Core constructors 成为 lane graph、route 和 Vehicle Profile domain invariant 的唯一事实源。
- 提供稳定、结构化且能定位输入位置的错误。
- 保证加载阶段的字符串和 handle 工作不进入 fixed tick hot path。

非目标：

- 不读取文件路径或定义同步/异步 asset I/O。
- 不创建 `CoreWorld`，不接收 fixed tick 或 initial vehicles。
- 不公开 raw wire DTO 或 Rust authoring serialization API。
- 不一次收集全部 authoring errors。
- 不为 hostile oversized JSON 提供完整 denial-of-service 防护。
- 不改变 v0.2 schema 或为 v0.2 合成 Vehicle Profile。

## 2. Crate 与模块边界

目标 workspace：

```text
crates/
  laneflow-core/
    VehicleProfile
    VehicleProfileHandle
    VehicleProfileRegistry
    InitialTrafficData
    CoreError
  laneflow-data/
    private v0.2/v0.3 wire DTO
    JSON loader
    version/unit validation
    DataError
```

Cargo 依赖只允许：

```text
laneflow-data -> laneflow-core
```

Core 不得通过 normal dependency 或 dev-dependency 反向依赖 data crate。需要同时验证 loader 和 Core 行为的 integration test 放在 `laneflow-data`。

## 3. Public Loader API

概念 public surface：

```text
from_json_slice(&[u8]) -> Result<LoadedPackage, DataError>
from_json_str(&str) -> Result<LoadedPackage, DataError>

LoadedPackage #[non_exhaustive]
  V0_2(LoadedV0_2)
  V0_3(LoadedV0_3)
```

`LoadedPackage` 和 public error enum 使用 `#[non_exhaustive]`。loaded structs 使用私有字段、只读 accessors 和受控 `into_*` 方法，不公开 raw Serde DTO。

`from_json_slice` 是主入口；`from_json_str` 只做 UTF-8 string convenience forwarding。loader 不提供 `load_file`、`Read`、async 或 engine asset overload。

调用方必须显式匹配 `V0_2` / `V0_3`。unknown、missing 或不匹配的 `formatVersion` 返回结构化 error，不猜测版本。

### 3.1 v0.2

`LoadedV0_2` 不具有 Vehicle Profile 数据语义。v0.2 wire package 不允许 `units.time` 或 `vehicleProfiles`，loader 不创建 default profile。

v0.2 的 `InitialTrafficData` 内部可以持有空 `VehicleProfileRegistry`，以保持 Core container 结构一致；该空 registry 不是格式版本判据，也不能把 v0.2 提升为 v0.3。格式语义只由外层 `LoadedPackage::V0_2` 决定。

### 3.2 v0.3

`LoadedV0_3` 必须来自 `formatVersion: "0.3"`、`units.distance: "meter"`、`units.time: "second"` 和显式 `vehicleProfiles`。`vehicleProfiles` 允许为空，但字段本身必填。

## 4. Core `InitialTrafficData`

Core public model：

```text
InitialTrafficData
  lane_graph: LaneGraph
  routes: Vec<Route>
  vehicle_profiles: VehicleProfileRegistry
```

所有字段保持私有。`InitialTrafficData::try_new` 校验：

- 初始 route external ID 唯一；
- route edge 引用存在；
- 相邻 route edge 连通；
- Vehicle Profile external ID 唯一；
- profile registry 与 lane graph/routes 都处于有效状态。

初始 route validation 与 `CoreWorld::register_route` 必须调用同一 Core 私有 helper。data crate 不复刻 duplicate/unknown/continuity 判断。

`InitialTrafficData` 不分配 `RouteHandle`。runtime route slot、generation、dynamic register/remove 和 route-in-use 规则仍由 `CoreWorld` 拥有。

`LaneGraph` 和 `VehicleProfileRegistry` 可以在进入 world 前分配各自 opaque handle；handle 只对随 `InitialTrafficData` 移入同一 world 的 registry 有效，不形成跨 world 数值身份。

## 5. Vehicle Profile Construction

Vehicle Profile 使用 IIDM 专用的命名输入，避免多个 `f64` 位置参数，也不提前公开 controller/model dispatch：

```text
IidmProfileSpec
  length
  desired_speed
  min_gap
  time_headway
  max_acceleration
  comfortable_deceleration
  emergency_deceleration

VehicleProfile::try_new_iidm(id, spec)
```

`IidmProfileSpec` 是未经验证的 construction input；只有 fallible constructor 能产生有效 `VehicleProfile`。有效 profile 字段保持私有。data crate 必须先验证 external `model` 为 `"iidm"`，再调用该 constructor。

v0.3 不公开 `VehicleFollowingModel` enum、controller trait 或通用 model registry。若后续增加第二种内置模型，按 ADR 0006 先评估私有 enum/static dispatch，再决定是否需要新的 public construction API。

v0.3 不要求为每个 profile 参数立即公开独立 numeric newtype。constructor 必须执行 `vehicle-following.md` 已冻结的 finite、positive、epsilon 和 cross-field validation。

`VehicleProfileHandle` 只实现并承诺 `Clone + Copy + Debug + Eq + Hash`。registry 按 profile 输入顺序分配 handle，不公开 index、数值或排序语义，也不提供 runtime register/remove/mutate。

## 6. Loader Pipeline

production loader 使用 fail-fast，阶段顺序为：

1. JSON syntax 与 wire shape；
2. `formatVersion` 与 units；
3. lane graph wire records；
4. route wire records；
5. Vehicle Profile wire records；
6. Core `InitialTrafficData` normalization。

阶段 1 的 shape error 按 parser traversal 返回准确路径；成功反序列化后的 validation 按上述 canonical 顺序和数组输入顺序执行。

Serde/`serde_json` 负责 syntax、required field、type 和 unknown field。private DTO 使用 `#[serde(deny_unknown_fields)]` 属性，与 schema 的 `additionalProperties: false` 对齐。

external ID、edge length、route topology 和 Vehicle Profile invariant 通过 Core constructors 校验。data crate 可以附加输入 index/path context，但不得用第二份正则、epsilon 或 route algorithm 重新判断。

## 7. Error Model

概念错误分层：

```text
DataError #[non_exhaustive]
  JsonSyntax
  JsonShape
  UnsupportedFormatVersion
  InvalidUnit
  CoreDomain { path, source: CoreError }
```

要求：

- JSON syntax/shape error 保留 path、line、column 和 underlying source；
- domain error 保留 external ID、field、可用的数组 index/path 和 underlying `CoreError`；
- `DataError` 与 `CoreError` 实现 `std::error::Error + Send + Sync`；
- `Display` 使用中文说明，机器判断使用 enum variant 和字段；
- 不为测试实现包含 `f64` 的脆弱全量 `PartialEq`，测试优先使用 `assert_matches!` 和字段断言。

`serde_path_to_error` 用于反序列化 path。它不负责反序列化后的 duplicate、unknown reference 或 cross-field path，后者由 normalization 调用点附加 context。

## 8. Schema 与 Contract Tests

checked-in Draft 2020-12 schema 是 external shape contract。`jsonschema` 保持 `laneflow-data` 的 dev-dependency，不进入 production dependencies。

测试所有权：

- `laneflow-core`：Vehicle Profile、handle、registry、`InitialTrafficData`、route helper 和 domain invariant；
- `laneflow-data`：v0.2/v0.3 JSON、严格版本、units、unknown field、shape path 和 Core normalization；
- data crate integration test：加载 fixture 后构造/消费 Core input；
- 当前 Core integration test 中的私有 Serde DTO 迁移到 data crate，避免两套 loader view。

Fixture 规则：

- 每个合法 fixture 同时通过对应 schema 和 production loader；
- shape-invalid fixture 同时被 schema 与 loader 拒绝；
- domain-only invalid fixture 可以通过 schema，但必须被 loader/Core normalization 拒绝；
- 必测 cross-version contamination，例如 v0.2 携带 `vehicleProfiles` 或 v0.3 缺少 `units.time`；
- schema、private DTO 和 Core constructors 的 validation 变化必须在同一 PR 更新测试。

不使用 `schemars` 自动生成并替代已审阅 schema。

## 9. Input Safety 与 Performance

v0.3 不设置固定 edge、route 或 profile count limit。调用方负责在进入 loader 前限制 input byte size；loader 不承诺完整抵御恶意超大、深层或资源耗尽输入。

若后续开放网络上传或不可信 mod 数据，应新增 `LoadLimits`、隔离 validator process 或流式 ingestion 设计。

JSON parsing、external ID resolution、Core normalization 和 handle 分配只发生在加载阶段。fixed tick 只读取 Core handle 和 compact profile data，不访问 wire DTO、JSON path 或 external ID hash lookup。

## 10. #73 实施输入

#73 必须交付：

- `crates/laneflow-data` workspace member、crate docs 和 workspace lints；
- v0.2/v0.3 private DTO 与严格 version dispatch；
- `from_json_slice` / `from_json_str` 和结构化 `DataError`；
- Core `VehicleProfile`、`VehicleProfileHandle`、immutable registry/resolver；
- Core `InitialTrafficData` 和共享 route validation helper；
- `schemas/laneflow-data-v0.3.schema.json`；
- v0.2 regression、v0.3 schema/loader、cross-version、domain error 和 resolver tests；
- 当前测试私有 DTO 向 data crate 的迁移；
- Core API/data-format breaking impact 与 #74 handoff 说明。

#73 不得在本切片中实现 VehicleState profile migration、occupancy、IIDM 或 safety projection。
