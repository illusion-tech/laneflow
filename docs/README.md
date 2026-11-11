# LaneFlow 文档

本目录保存 LaneFlow 的长期设计依据、架构决策、治理规则、参考资料和开发约定。

GitHub 用于管理当前任务、协作状态和合并证据；仓库文档用于保存可版本化、可审查、可被 AI Agent 稳定读取的长期事实。

ADR 0021 将 LaneFlow 定位为可嵌入、引擎无关、确定性的道路交通运行时与工具链。
应用业务、出行需求和路线选择策略由宿主拥有；地区交通规则与城市工作负载用于
通用技术验证。当前唯一可运行特化是道路机动车，更多执行域与城市级扩展是需要
独立证据的长期方向，不构成交付时间承诺。
`laneflow-runtime` / `TrafficWorld` 是唯一可运行交通世界。current Core 与 JSON
运行时入口已拆除。契约见
[共享静态路网消费][shared-consumption]。
已提交一维几何合同见 [ADR 0028][adr-0028]
（#496；整数毫米 / `mm/s`）。
编译器 IR 交通一维整数毫米统一表示见 ADR 0028 / #500（准入后 Typed AST / HIR / MIR / LIR 存整数毫米）。
路网产品不声明路线。`TrafficWorld` 路线入口是 `register_route`，见
[ADR 0029][adr-0029]。道路编辑来源
`format_version = 4`；现行可移植对象为 LFCA 5 / LFSM 4 / LFSD 4 / LFCP 2。
LF-CN-URBAN 的正式静态输入、路线目录与两档重建命令见
[`laneflow-urban-generator`][laneflow-urban-generator-readme]。

## 目录结构

```text
docs/
  README.md
  architecture.md
  roadmap.md
  guides/
  adr/
  design/
  governance/
  reference/
```

## 目录职责

- `docs/adr/`：记录高影响、难回退的架构决策，重点回答“为什么这样定”。
- `docs/design/`：记录 Traffic Runtime、数据格式、Adapter、运行时规则等设计，重点回答“具体怎么做”。
- `docs/guides/`：操作性接入教程。引用真实示例，不替代 design 与 ADR。
- `docs/governance/`：记录 GitHub 工作流、开发阶段评审、AI Agent 开发规则和文档边界。
- `docs/reference/`：记录术语、模板、校验矩阵和长期复用的辅助规范。
- `.agents/`：记录跨 Agent 的执行工作流；工具专用入口只应薄包装这些工作流。

语言约定：长期设计、模板与治理规范以中文为准，英文仅作辅助理解；双语术语
以 [glossary][glossary] 为 SSOT，详见
[文档边界政策][documentation-policy] 第 6 节。

仓库安全扫描的期望配置、状态语义和发布阻断规则见
[安全扫描][security-scanning]。

默认实现语言及非 Rust 代码入库前的代码语言立项审查见
[代码语言立项审查][code-language-policy]。研究与脚本同样适用。

源代码许可证、开放/商业边界、Cargo 依赖许可证、RustSec 与 Dependabot 基线见
[依赖安全][dependency-security]。

## 推荐阅读顺序

1. [README.md][readme]
2. [从源码接入无窗口交通运行时][source-integration]（程序：[`headless.rs`][headless-rs]）
3. [架构说明][architecture]
4. [当前能力与限制][capabilities]
5. [路线图][roadmap]
6. [ADR 0021][adr-0021]
7. [AGENTS.md][agents]
8. [Agent 工作流][agents-readme]
9. [文档边界政策][documentation-policy]
10. [GitHub 工作流][github-workflow]
11. [开发阶段评审][development-gates]
12. [Agent 开发指南][agent-development-guide]
13. [安全扫描][security-scanning]
14. [依赖安全][dependency-security]
15. [ADR 索引][adr-readme]
16. [设计索引][design-readme]
17. [参考资料索引][reference-readme]

[shared-consumption]: design/traffic-runtime-shared-consumption.md
[adr-0028]: adr/0028-integer-millimeter-traffic-geometry.md
[adr-0029]: adr/0029-retire-precompiled-static-route.md
[laneflow-urban-generator-readme]: ../tools/laneflow-urban-generator/README.md
[glossary]: reference/glossary.md
[documentation-policy]: governance/documentation-policy.md
[security-scanning]: governance/security-scanning.md
[code-language-policy]: governance/code-language-policy.md
[dependency-security]: governance/dependency-security.md
[readme]: ../README.md
[source-integration]: guides/source-integration.md
[headless-rs]: ../crates/laneflow-runtime/examples/headless.rs
[architecture]: architecture.md
[capabilities]: reference/capabilities.md
[roadmap]: roadmap.md
[adr-0021]: adr/0021-traffic-infrastructure-and-host-boundary.md
[agents]: ../AGENTS.md
[agents-readme]: ../.agents/README.md
[github-workflow]: governance/github-workflow.md
[development-gates]: governance/development-gates.md
[agent-development-guide]: governance/agent-development-guide.md
[adr-readme]: adr/README.md
[design-readme]: design/README.md
[reference-readme]: reference/README.md
