# 0002 Dependency and Licensing Constraints

**状态**: Accepted  
**日期**: 2026-06-18  
**最近修订**: 2026-07-14  
**适用范围**: LaneFlow 公开仓库、Core、Data、Adapter、Authoring 工具的源代码许可、运行时依赖与第三方许可证边界  
**关联文档**:

- 上游决策:
  - `0001-project-scope.md`
- 相关治理:
  - `../governance/development-gates.md`
  - `../governance/dependency-security.md`
  - `../governance/documentation-policy.md`
  - `../governance/security-scanning.md`

## 背景

LaneFlow 的核心定位之一是“商用可控”，并明确不把 SUMO / CARLA / libsumo 作为客户端核心依赖。

`0001-project-scope.md` 已在范围层面记录该原则，但没有把它变成可执行、可评审、可阻断的依赖与许可证约束。缺少这层约束时，重型交通仿真依赖或带强 copyleft 的库可能在实现过程中被悄悄引入 Core 或 Adapter，破坏商用可控性和引擎无关性。

项目同时需要明确自有源代码的分发许可证。只约束第三方依赖而没有根许可证、Cargo metadata 和自动审计，会让开放 Core/Data 与未来商业产品的边界无法被使用者、贡献者和 CI 一致理解。

## 决策

### 1. 源代码许可证与产品边界

LaneFlow 公开仓库采用 **Apache License 2.0-only**，不提供 MIT 双许可。根 `LICENSE` 保存标准条款，workspace package 使用 SPDX `Apache-2.0`。

开放仓库长期承载 Core、Data、公开 schema/spec、公共 API、测试和最小示例。高级编辑器、城市级或分布式仿真、优化分析、企业 Adapter、云服务和商业支持可以作为独立商业产品交付。依赖方向只允许商业产品依赖开放 Core/Data；开放仓库不得反向依赖商业实现。

第三方材料继续遵循其自身许可证。当前没有需要传递的 attribution notices，因此不创建空的 `NOTICE`；未来出现实际 notices 时再按 Apache-2.0 分发义务维护。

### 2. 客户端核心依赖红线

LaneFlow Core 和随产品分发的 Adapter **不得**把以下内容作为运行时核心依赖：

- SUMO、CARLA、libsumo 或等价的重型交通 / 自动驾驶仿真器。
- 任何要求运行时联网到外部仿真服务才能产生车流的组件。
- 任何无法满足商用分发的许可证组件（见第 4 节）。

### 3. 允许的集成方式

上述系统**可以**用于以下非客户端用途，但必须与 Core 物理隔离：

- 离线数据生成或转换（例如把外部路网导出为 LaneFlow 数据格式）。
- Authoring / 工具链阶段的一次性导入。
- 研究与对照实验（`Research` 类 Issue），不进入发布产物。

隔离要求：此类集成只能存在于独立工具或独立包中，Core 与发布 Adapter 不得在编译期或运行期依赖它们。

### 4. 第三方许可证政策

引入任何第三方依赖前，必须确认其许可证可商用分发。

- 默认候选：宽松许可证；实际可接受集合必须显式进入 `dependency-security.md` 与 `deny.toml`。当前全局允许 Apache-2.0、MIT、Unicode-3.0 和 Zlib，其他宽松许可证也需要先评审分发影响。
- 需要显式评审：弱 copyleft（LGPL、MPL 等），仅在动态链接且不污染 Core 时按例外流程接受。
- 默认禁止进入随产品分发的 Core / Adapter：强 copyleft（GPL、AGPL 等）。
- 任何例外必须按 `dependency-security.md` 和 `development-gates.md` 记录原因、范围、复核条件、Cleanup owner 与后续 Issue。

### 5. 自动审计、评审与阻断

- 新增运行时依赖的 PR 必须说明依赖名称、用途、许可证和分发影响。
- cargo-deny 必须检查 RustSec advisories、许可证、依赖约束和 crate 来源；Dependabot 负责安全更新和周期性版本更新。
- 违反第 2 节红线、引入未评审的禁止类许可证、未知来源或未处理 vulnerability advisory，默认阻断 `G3` 合并和公开发布。
- 依赖约束相关的破坏性变更（如移除某依赖导致 API 变化）需要新增或更新 ADR。

## 后果

- Core 与发布 Adapter 的依赖面保持轻量、可商用、引擎无关。
- 使用者和贡献者可以从根许可证与 Cargo metadata 一致确认开放仓库的 Apache-2.0-only 条款。
- 未来商业产品可以在独立许可证下构建，但不能让开放仓库依赖闭源实现。
- 与重型仿真器的集成被限定在离线工具或研究范围，不影响客户端可控性。
- PR 评审获得明确的依赖与许可证阻断依据，而不是仅凭口头约定。
- 后续若要调整源代码许可证或接受新的 copyleft 类别，应新增或修订 ADR，而不是静默放宽。
