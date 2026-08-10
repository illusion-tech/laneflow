# 当前 JSON 退役与编译器测试边界

**文档状态**：Accepted（取代原“当前包迁移导入前端”设计）<br>
**调整日期**：2026-08-10<br>
**关联议题**：#297、#294

## 1. 结论

Traffic v0.10、SpatialPackage v0.1 与 ScenarioManifest v0.1 JSON 是当前
`laneflow-data` 加载器使用的仓库内部格式。项目尚未发布 1.0，也从未通过 Release、
Pages、包分发或直接用户交付发布这些 JSON 数据；当前可见输入仅是仓库示例、测试夹具
和生成器输出。

因此：

- current JSON 不进入 `laneflow-compiler`；
- 不新增 `current-v0_10-import` 特性、`CurrentSourceInput`、
  `ValidatedCurrentImportBundle` 或 `laneflow-current-import`；
- 不提供批量迁移工具、已发布资产清单、迁移报告或长期离线兼容路径；
- 编译器正确性使用编译器原生的有类型模块测试，不以旧 JSON loader 为预言机；
- 仓库内部 JSON 夹具在新编制来源可表达相同场景后一次性转换或删除；
- `laneflow-data` 与 `laneflow-current-source` 只作为当前态加载实现保留到 #294 切换，
  不形成新的兼容承诺。

## 2. current JSON 的精确定义

本设计中的 current JSON 只包括：

- Traffic v0.10：当前 Core 静态交通输入；
- SpatialPackage v0.1：当前空间框架和边几何输入；
- ScenarioManifest v0.1：把 Traffic/Spatial 制品与长度、SHA-256 和媒体类型绑定的清单。

它们不是运行时快照、存档、目标静态镜像或长期编制单一事实源。`current` 表示“当前态
加载器所消费”，不表示“必须长期兼容的最新产品格式”。

## 3. 架构边界

当前切换完成前保留两条互不耦合的验证边界：

```text
current JSON -> laneflow-data -> current Core/Spatial
                （只验证旧加载器自身）

compiler-native typed module -> Typed AST -> HIR -> MIR -> Canonical LIR
                                                        |
                                                        v
                                      integration-only Core/Spatial projection
                                      （只验证编译器与投影）
```

禁止建立下列路径：

```text
current JSON -> compiler frontend -> HIR/MIR/LIR
```

把同一 JSON 交给旧加载器和编译器只能证明新入口模仿旧序列化格式，不能独立证明标识、
所有者关系、规范排序、LIR 语义或投影正确。它还会把 Serde 解析细节、资源配置和来源
位置维护成本带入一个没有迁移对象的临时前端。

## 4. 包职责

### 4.1 `laneflow-data`

继续拥有当前态 JSON 到 `InitialTrafficData` / `SpatialRegistry` 的加载与规范化，直到
#294 完成主代码路径切换。它的测试只验证仓库当前仍使用的接受集合、错误分类和构造
结果，不作为编译器的语义预言机。

### 4.2 `laneflow-current-source`

作为未发布的内部实现包，暂时为 `laneflow-data` 集中当前 wire DTO、版本、摘要和清单
配对。它不提供严格编译导入策略、编译器资源余额、编译器来源位置表或可升级能力包，
也不得成为 `laneflow-compiler` 的依赖。

除修复当前加载路径的实际正确性问题外，不再为逐项复刻未承诺的 Serde 边缘行为增加
手写解析复杂度。

### 4.3 `laneflow-compiler-test-support`

只消费 `ValidatedCanonicalLir` 并投影到当前 Core/Spatial，用于切换期集成验证。测试
直接构造编译器原生有类型模块，并对投影实体、关系、几何和确定性做显式断言；该包
不依赖 `laneflow-data` 或 `serde_json`。

### 4.4 不建立的包和 API

- 不建立 `laneflow-current-import`；
- 不建立编译器 `current-v0_10-import` 特性；
- 不建立 public `CurrentSourceInput` / `CurrentImportProvenance`；
- 不建立 `CurrentSourceLimits` / `CurrentSourceLocationTable`；
- 不建立 `ValidatedCurrentImportBundle`；
- 不建立 current JSON 的批量迁移、资产报告或发布兼容工具。

## 5. 正确性与测试

### 5.1 旧加载器

旧加载器测试保持在 `laneflow-data` / `laneflow-current-source`：

- 版本和 JSON shape；
- ScenarioManifest 与 Traffic/Spatial 长度、摘要、媒体类型和引用配对；
- 当前 Core/Spatial 构造结果；
- 已在仓库内使用的错误类别和定位。

这些测试不得被表述为外部兼容性或资产迁移保证。

### 5.2 编译器与投影

编译器和投影测试直接构造代表性 `SyntheticModule` 或后继正式编制前端模块，覆盖：

- 稳定标识、所有者关系与规范顺序；
- 路网、横断面、路口、信号、等待区、停车、准入和车辆配置；
- Canonical LIR 到当前 Core/Spatial 的投影；
- 几何采样、出现项表和重复编译确定性。

测试夹具不得从 current JSON 字段动态生成编译器输入。旧 JSON 数据与编译器原生夹具
即使描述相似场景，也属于两个独立测试输入。

## 6. 仓库夹具处理

新编制来源覆盖相应示例后，对仓库内 current JSON 逐项执行以下二选一：

1. 场景仍有独立示例价值：一次性重写为新编制来源；
2. 仅服务旧 loader 回归：随旧 loader 一起删除。

因为不存在已发布或用户持有的 current JSON，转换不需要稳定工具、通用诊断、失败
清单或可重复迁移协议。若未来在删除前首次发现真实外部资产，应以新事实重新立项，
不得恢复本设计已取消的预防性兼容层。

## 7. 性能、收益与代价

移除 current JSON 编译器接入：

- 正确性收益：编译器测试只证明编译器语义，避免旧 loader 行为掩盖或限定目标设计；
- 性能收益：不再为同一 JSON 增加严格导入解析、位置收集、摘要和中间转换；
- 维护收益：移除测试内 JSON→Synthetic 转换器和未来五类迁移 API/工具；
- 代价：仓库内少量示例需要在切换时手工重写，历史 current JSON 不再有通用导入器；
- 风险：新原生夹具可能遗漏旧示例覆盖的领域组合，必须通过显式代表性实体/关系矩阵
  和 Core/Spatial 投影断言补足，而不是重新引入序列化耦合。

该取舍符合当前事实：一次性内部夹具整理的代价显著低于长期维护临时前端的代价。

## 8. 实施与治理

1. 停止并关闭只服务严格 current 导入的实现 PR；不继续手写单遍 JSON 平价解析。
2. 删除测试内 current JSON→编译器模块转换器及其 `laneflow-data`/`serde_json` 依赖。
3. 用编译器原生代表性夹具维持 LIR→Core/Spatial 投影覆盖。
4. 从 #297 的范围、验收标准和 Gate Ledger 中移除迁移前端、资产报告与导入 API；原
   G1/G2 基于已发布资产迁移前提，不得继续作为调整后范围的通过证据。
5. 由新范围重新完成 G1/G2；#294 切换时删除旧 loader、source 包和不再需要的 JSON
   夹具。

本设计不改变当前 Core API、目标 Traffic Runtime API、JSON wire shape 或 Adapter API；
它删除尚未交付的迁移能力，并收紧测试和包依赖边界。
