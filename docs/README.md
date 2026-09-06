# LaneFlow 文档

本目录保存 LaneFlow 的长期设计依据、架构决策、治理规则、参考资料和开发约定。

GitHub 用于管理当前任务、协作状态和合并证据；仓库文档用于保存可版本化、可审查、可被 AI Agent 稳定读取的长期事实。

ADR 0021 将 LaneFlow 定位为可嵌入、引擎无关、确定性的道路交通运行时与工具链。
应用业务、出行需求和路线选择策略由宿主拥有；地区交通规则与城市工作负载用于
通用技术验证。当前唯一可运行特化是道路机动车，更多执行域与城市级扩展是需要
独立证据的长期方向，不构成交付时间承诺。
`laneflow-runtime` / `TrafficWorld` 是唯一可运行交通世界。current Core 与 JSON
运行时入口已拆除。契约见 `docs/design/traffic-runtime-shared-consumption.md`。
已提交一维几何合同见 ADR 0028（#496；整数毫米 / `mm/s`）。
编译器 IR 交通一维收口见 ADR 0028 / #500（准入后 Typed AST / HIR / MIR / LIR 存整数毫米）。
路网产品不声明路线。`TrafficWorld` 路线入口是 `register_route`，见 ADR 0029
（对象 `formatVersion = 5`）。

## 目录结构

```text
docs/
  README.md
  architecture.md
  roadmap.md
  adr/
  design/
  governance/
  reference/
```

## 目录职责

- `docs/adr/`：记录高影响、难回退的架构决策，重点回答“为什么这样定”。
- `docs/design/`：记录 Core、数据格式、Adapter、运行时规则等设计，重点回答“具体怎么做”。
- `docs/governance/`：记录 GitHub 工作流、开发闸口、AI Agent 开发规则和文档边界。
- `docs/reference/`：记录术语、模板、校验矩阵和长期复用的辅助规范。
- `.agents/`：记录跨 Agent 的执行工作流；工具专用入口只应薄包装这些工作流。

语言约定：长期设计、模板与治理规范以中文为权威事实，英文仅作辅助理解；双语术语
以 `docs/reference/glossary.md` 为 SSOT，详见
`docs/governance/documentation-policy.md` 第 6 节。

仓库安全扫描的期望配置、状态语义和发布阻断规则见 `docs/governance/security-scanning.md`。

源代码许可证、开放/商业边界、Cargo 依赖许可证、RustSec 与 Dependabot 基线见 `docs/governance/dependency-security.md`。

## 推荐阅读顺序

1. `README.md`
2. `docs/architecture.md`
3. `docs/roadmap.md`
4. `docs/adr/0021-traffic-infrastructure-and-host-boundary.md`
5. `AGENTS.md`
6. `.agents/README.md`
7. `docs/governance/documentation-policy.md`
8. `docs/governance/github-workflow.md`
9. `docs/governance/development-gates.md`
10. `docs/governance/agent-development-guide.md`
11. `docs/governance/security-scanning.md`
12. `docs/governance/dependency-security.md`
13. `docs/adr/README.md`
14. `docs/design/README.md`
