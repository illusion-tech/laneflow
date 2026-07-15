# Data Loading 设计

**文档状态**: Accepted  
**最后更新**: 2026-07-15  
**适用范围**: 当前 v0.3 JSON package 的 Rust loader、版本闸口、Core normalization、错误模型、测试与输入安全边界  
**关联文档**:

- `../architecture.md`
- `../roadmap.md`
- `../adr/0005-core-identity-and-handle-model.md`
- `../adr/0006-vehicle-following-control-and-safety.md`
- `../adr/0007-traffic-data-crate-and-loader-boundary.md`
- `../adr/0008-pre-1.0-data-format-version-policy.md`
- `../adr/0009-signal-indication-gate-and-policy-separation.md`
- `data-format.md`
- `vehicle-following.md`
- `signal-system.md`

> **Planned v0.4 提示**：#93 已接受 Signals loader/Core normalization 设计，见 `signal-system.md`。production loader、`CURRENT_FORMAT_VERSION`、private DTO 和 public `LoadedPackage` 当前仍只实现 v0.3；#94 原子切换全部 current artifacts 前，本文件不得提前改写为已实现的 0.4 loader。

## 1. 目标与非目标

目标：

- 为当前 `formatVersion: "0.3"` external package 提供唯一 production JSON loader。
- 保持 `laneflow-data -> laneflow-core` 单向依赖。
- 使用必填版本闸口拒绝旧版、未来版和缺失版本，不在 production loader 中保留历史兼容分支。
- 让 Core constructors 成为 lane graph、route 和 Vehicle Profile domain invariant 的唯一事实源。
- 提供稳定、结构化且能定位输入位置的错误。
- 保证加载阶段的字符串和 handle 工作不进入 fixed tick hot path。

非目标：

- 不兼容加载 v0.2，也不提供 v0.2 -> v0.3 自动迁移。
- 不读取文件路径或定义同步/异步 asset I/O。
- 不创建 `CoreWorld`，不接收 fixed tick 或 initial vehicles。
- 不公开 raw wire DTO 或 Rust authoring serialization API。
- 不一次收集全部 authoring errors。
- 不为 hostile oversized JSON 提供完整 denial-of-service 防护。

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
    private current wire DTO
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
CURRENT_FORMAT_VERSION: &str = "0.3"

from_json_slice(&[u8]) -> Result<LoadedPackage, DataError>
from_json_str(&str) -> Result<LoadedPackage, DataError>

LoadedPackage
  initial_traffic_data: InitialTrafficData
```

`LoadedPackage` 使用私有字段、只读 accessor 和受控 `into_initial_traffic_data`，不公开 raw Serde DTO。它只表示当前格式，因此不使用 version enum、历史 variant 或 optional profile 区分格式。

`from_json_slice` 是主入口；`from_json_str` 只做 UTF-8 string convenience forwarding。loader 不提供 `load_file`、`Read`、async 或 engine asset overload。

### 3.1 版本闸口

Loader 先读取只包含 `formatVersion` 的最小 header，再决定是否进入当前 wire DTO：

```text
version header -> current version check -> strict current DTO -> Core normalization
```

- 只有精确的 `"0.3"` 可以继续。
- 缺失、`null` 或非字符串版本是 JSON shape error。
- `"0.2"`、未来版本或其他字符串返回 `UnsupportedFormatVersion { expected, actual }`。
- 不支持版本的其余字段不按 v0.3 规则校验，确保版本错误优先且不产生误导性 units/unknown-field 诊断。
- 不猜测版本，不自动升级，不合成默认 profile。

完整兼容策略见 ADR 0008。1.0 前每次破坏性格式演进直接替换当前 DTO/schema/fixture；只有真实外部迁移需求出现后，才单独设计离线迁移工具或限期兼容层。

### 3.2 当前 v0.3 shape

当前 package 必须包含：

- `formatVersion: "0.3"`；
- `units.distance: "meter"`；
- `units.time: "second"`；
- `laneGraph`；
- `routes`；
- 显式 `vehicleProfiles`。

`vehicleProfiles` 允许为空，但字段本身必填。空 registry 是当前格式的合法 domain state，不表示历史版本。

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

Vehicle Profile 使用 IIDM 专用命名输入，避免多个 `f64` 位置参数，也不提前公开 controller/model dispatch：

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

1. JSON syntax 与最小 version header；
2. current version check；
3. 当前 wire shape；
4. distance/time units；
5. Vehicle Profile wire records；
6. lane graph 与 route Core construction；
7. Core `InitialTrafficData` normalization。

阶段 1 和 3 的 shape error 按 parser traversal 返回准确路径；成功反序列化后的 validation 按上述 canonical 顺序和数组输入顺序执行。

Serde/`serde_json` 负责 syntax、required field、type 和 unknown field。private current DTO 使用 `#[serde(deny_unknown_fields)]`，与 schema 的 `additionalProperties: false` 对齐。

external ID、edge length、route topology 和 Vehicle Profile invariant 通过 Core constructors 校验。data crate 可以附加输入 index/path context，但不得用第二份正则、epsilon 或 route algorithm 重新判断。

## 7. Error Model

概念错误分层：

```text
DataError #[non_exhaustive]
  JsonSyntax
  JsonShape
  UnsupportedFormatVersion { expected, actual }
  InvalidUnit
  UnsupportedVehicleProfileModel
  CoreDomain { path, source: CoreError }
```

要求：

- JSON syntax/shape error 保留 path、line、column 和 underlying source；
- version error 同时暴露 expected current version 与 actual version；
- domain error 保留 external ID、field、可用的数组 index/path 和 underlying `CoreError`；
- `DataError` 与 `CoreError` 实现 `std::error::Error + Send + Sync`；
- `Display` 使用中文说明，机器判断使用 enum variant 和字段；
- 不为测试实现包含 `f64` 的脆弱全量 `PartialEq`，测试优先使用 `assert_matches!` 和字段断言。

`serde_path_to_error` 用于反序列化 path。它不负责反序列化后的 duplicate、unknown reference 或 cross-field path，后者由 normalization 调用点附加 context。

## 8. Schema 与 Contract Tests

checked-in Draft 2020-12 schema 是当前 external shape contract。`jsonschema` 保持 `laneflow-data` 的 dev-dependency，不进入 production dependencies。

测试所有权：

- `laneflow-core`：Vehicle Profile、handle、registry、`InitialTrafficData`、route helper 和 domain invariant；
- `laneflow-data`：当前 v0.3 JSON、版本拒绝、units、unknown field、shape path 和 Core normalization；
- data crate integration test：加载当前 fixture 后构造/消费 Core input；
- Core 不保留私有 Serde DTO 或 schema tests，避免两套 loader view。

Fixture 规则：

- 当前合法 fixture 同时通过当前 schema 和 production loader；
- shape-invalid fixture 同时被 schema 与 loader 拒绝；
- domain-only invalid fixture 可以通过 schema，但必须被 loader/Core normalization 拒绝；
- 旧版和未来版必须在 strict current DTO 前返回 version error；
- schema、private DTO 和 Core constructors 的 validation 变化必须在同一 PR 更新测试。

不使用 `schemars` 自动生成并替代已审阅 schema。旧 schema/fixture 不留在 active tree；历史审计使用 immutable Git permalink 和 v0.2 收口文档。

## 9. Input Safety 与 Performance

v0.3 不设置固定 edge、route 或 profile count limit。调用方负责在进入 loader 前限制 input byte size；loader 不承诺完整抵御恶意超大、深层或资源耗尽输入。

若后续开放网络上传或不可信 mod 数据，应新增 `LoadLimits`、隔离 validator process 或流式 ingestion 设计。

JSON parsing、external ID resolution、Core normalization 和 handle 分配只发生在加载阶段。fixed tick 只读取 Core handle 和 compact profile data，不访问 wire DTO、JSON path 或 external ID hash lookup。

## 10. #73 实施输入

#73 必须交付：

- `crates/laneflow-data` workspace member、crate docs 和 workspace lints；
- 当前 v0.3 private DTO、两阶段 version gate 与单一 `LoadedPackage`；
- `from_json_slice` / `from_json_str` 和结构化 `DataError`；
- Core `VehicleProfile`、`VehicleProfileHandle`、immutable registry/resolver；
- Core `InitialTrafficData` 和共享 route validation helper；
- `schemas/laneflow-data-v0.3.schema.json`；
- v0.3 schema/loader/fixture、旧版/未来版拒绝、domain error 和 resolver tests；
- 当前 Core 测试私有 DTO 向 data crate 的迁移；
- active v0.2 schema/fixture 和 production compatibility path 的移除；
- Core API/data-format breaking impact 与 #74 handoff 说明。

#73 不得在本切片中实现 VehicleState profile migration、occupancy、IIDM、safety projection 或历史格式迁移工具。
