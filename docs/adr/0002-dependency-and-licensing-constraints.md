# 0002 Dependency and Licensing Constraints

**状态**: Accepted  
**日期**: 2026-06-18  
**适用范围**: LaneFlow Core、Adapter、Authoring 工具的运行时依赖与第三方许可证边界  
**关联文档**:

- 上游决策:
  - `0001-project-scope.md`
- 相关治理:
  - `../governance/development-gates.md`
  - `../governance/documentation-policy.md`

## 背景

LaneFlow 的核心定位之一是“商用可控”，并明确不把 SUMO / CARLA / libsumo 作为客户端核心依赖。

`0001-project-scope.md` 已在范围层面记录该原则，但没有把它变成可执行、可评审、可阻断的依赖与许可证约束。缺少这层约束时，重型交通仿真依赖或带强 copyleft 的库可能在实现过程中被悄悄引入 Core 或 Adapter，破坏商用可控性和引擎无关性。

## 决策

### 1. 客户端核心依赖红线

LaneFlow Core 和随产品分发的 Adapter **不得**把以下内容作为运行时核心依赖：

- SUMO、CARLA、libsumo 或等价的重型交通 / 自动驾驶仿真器。
- 任何要求运行时联网到外部仿真服务才能产生车流的组件。
- 任何无法满足商用分发的许可证组件（见第 3 节）。

### 2. 允许的集成方式

上述系统**可以**用于以下非客户端用途，但必须与 Core 物理隔离：

- 离线数据生成或转换（例如把外部路网导出为 LaneFlow 数据格式）。
- Authoring / 工具链阶段的一次性导入。
- 研究与对照实验（`Research` 类 Issue），不进入发布产物。

隔离要求：此类集成只能存在于独立工具或独立包中，Core 与发布 Adapter 不得在编译期或运行期依赖它们。

### 3. 第三方许可证政策

引入任何第三方依赖前，必须确认其许可证可商用分发。

- 默认允许：宽松许可证（MIT、BSD、Apache-2.0、Zlib 等）。
- 需要显式评审：弱 copyleft（LGPL、MPL 等），仅在动态链接且不污染 Core 时按例外流程接受。
- 默认禁止进入随产品分发的 Core / Adapter：强 copyleft（GPL、AGPL 等）。
- 任何例外必须按 `development-gates.md` 第 8 节记录原因、范围、清理责任与后续 Issue。

### 4. 评审与阻断

- 新增运行时依赖的 PR 必须说明依赖名称、用途、许可证和分发影响。
- 违反第 1 节红线，或引入未评审的禁止类许可证，默认阻断 `G3` 合并。
- 依赖约束相关的破坏性变更（如移除某依赖导致 API 变化）需要新增或更新 ADR。

## 后果

- Core 与发布 Adapter 的依赖面保持轻量、可商用、引擎无关。
- 与重型仿真器的集成被限定在离线工具或研究范围，不影响客户端可控性。
- PR 评审获得明确的依赖与许可证阻断依据，而不是仅凭口头约定。
- 后续若要调整许可证政策（例如接受某弱 copyleft 库），应新增后续 ADR，而不是静默放宽。
