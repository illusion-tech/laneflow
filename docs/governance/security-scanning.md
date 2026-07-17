# 安全扫描基线

**文档状态**: Active  
**最后更新**: 2026-07-17
**适用范围**: LaneFlow 仓库的 Code Scanning、Secret Scanning、Dependabot 状态审计与公开发布阻断  
**关联 Issue**: `#88`、`#56`

## 1. 目标

本文定义 LaneFlow 的最小安全扫描基线，回答以下问题：

- 哪些 GitHub 安全能力必须启用。
- “零告警”“未配置”“已禁用”“无分析”和“不可用”如何区分。
- PR 合并与公开发布前需要哪些证据。
- 哪些依赖安全与许可证职责继续由 `#56` 承担。

GitHub 仓库设置和实时告警属于平台状态，不由仓库文件直接声明。文档只定义期望状态与判断规则；每次变更仍须通过 Issue、PR、Gate Ledger 和 GitHub API 保存可追踪证据。

## 2. 基线配置

### 2.1 Code Scanning

LaneFlow 使用 GitHub CodeQL default setup：

- 自动识别并分析 `actions` 与 `rust`。
- 使用 `default` query suite 和 GitHub-hosted standard runner。
- Rust 使用 GitHub 支持的 `none` build mode，不额外维护手工 build workflow。
- 由 default setup 负责默认分支、受保护分支、PR 和每周调度扫描。
- `main` ruleset 使用原生 `code_scanning` 规则要求 CodeQL 提供结果；分析未配置、仍在运行或发现 `high` / `critical` security alert 时阻断合并。
- 只有实际出现覆盖率、查询、构建方式或 runner 限制时，才通过独立 Issue 评估 advanced setup。

选择 default setup 是为了保留 GitHub 的自动语言识别与低维护升级路径，避免在没有定制需求时自行维护 CodeQL workflow。使用原生 ruleset merge protection 是为了让分析缺失、未完成和高危结果成为机器可执行阻断，而不是只依赖人工阅读 Checks。GitHub 官方说明见 [配置 Code Scanning](https://docs.github.com/en/code-security/how-tos/find-and-fix-code-vulnerabilities/configure-code-scanning/configure-code-scanning)、[Code Scanning setup types](https://docs.github.com/en/code-security/concepts/code-scanning/setup-types) 和 [Code Scanning merge protection](https://docs.github.com/en/code-security/how-tos/find-and-fix-code-vulnerabilities/manage-your-configuration/set-merge-protection)。

### 2.2 Secret Scanning

以下免费公开仓库能力必须保持启用：

- Secret Scanning user alerts。
- Secret Scanning push protection。

任何 push protection bypass 都必须在 G3 前复核；若推送内容包含真实 secret，必须撤销或轮换凭据并清理提交历史，不能只用 bypass 理由关闭告警。

以下能力不属于当前基线，不能在审计中写成“已覆盖”：

- non-provider patterns。
- validity checks。
- AI detection。
- delegated bypass。

这些能力未启用不等于存在安全告警，也不等于已经完成检测；需要时应先确认 GitHub plan、组织策略和仓库权限，再通过独立 Issue 扩展基线。

### 2.3 Dependabot 与依赖政策

本基线要求：

- Dependabot vulnerability alerts 可用。
- Dependabot security updates 保持启用。
- Cargo 与 GitHub Actions version updates 由 `.github/dependabot.yml` 每周执行。
- 审计时读取 open alerts，并按严重级别执行第 4 节阻断规则。
- cargo-deny 在 CI 中检查 RustSec advisories、许可证、依赖约束和 crate 来源。

源代码许可证、允许的第三方许可证、cargo-deny 配置、Dependabot 更新策略和例外字段以 `dependency-security.md` 为事实源。本文只定义 GitHub 安全能力的实时状态语义及合并/发布阻断；两份文档必须同时满足，不能以一方的空告警替代另一方的门禁。

`#88` 建立 GitHub 扫描基线，`#56` 建立源代码许可证与依赖安全基线；对应长期规则已分别进入本文与 `dependency-security.md`，后续任务不得只引用已关闭 Issue 代替当前文档和实时 API 证据。

### 2.4 Schema publication availability

ADR 0011 把 catalog 中的 JSON Schema `$id` 定义为 public retrieval URL。Schema publication workflow 与 scheduled monitor 负责 HTTP 200、media type、合法 JSON 和 byte equality；失败阻断 #103 G4 或后续受影响 release 的 publication 判断。

该 availability 证据不替代 Code Scanning、Secret Scanning、Dependabot 或 cargo-deny，也不能把这些安全能力的失败解释为 schema hosting 问题。消费者主动下载 schema 时，网络来源、revision/content pin、完整性和输入限制仍由其部署边界负责。

## 3. 状态语义

安全审计必须记录能力状态、最近运行状态和开放告警结果，不得只写“通过”。

| 状态 | 判定要求 | 可否写“零开放告警” |
| --- | --- | --- |
| 已配置且成功 | 功能已启用；CodeQL 最近适用分析成功；alerts API 成功返回空集合 | 可以，同时记录时间、分支或 commit、运行链接 |
| 已配置，分析待完成 | 功能已启用，但首次或最近适用分析仍为 `pending` / `queued` | 不可以 |
| 已配置，无分析 | 功能已启用，但找不到适用分析或分析未覆盖目标语言 / commit | 不可以 |
| 分析失败或降级 | 最近适用分析失败、取消、超时，或预期语言缺失 | 不可以 |
| 未配置 | 平台返回 `not-configured` 或没有对应 setup | 不可以 |
| 已禁用 | 平台明确返回 `disabled` | 不可以 |
| 无权限或不可用 | API 返回权限、plan、组织策略或平台可用性错误 | 不可以；必须记录显式例外 |

对三类能力分别采用以下最低证据：

- Code Scanning：default setup 为 `configured`，预期语言存在，最近适用 run / analysis 成功，open alerts API 成功返回。
- Secret Scanning：功能与 push protection 均为 `enabled`，open alerts API 成功返回。
- Dependabot：vulnerability alerts 可用，security updates 为 `enabled`，open alerts API 成功返回；version updates 配置存在且适用。设置状态、空告警和 cargo-deny 结果必须分别记录。

API 返回空集合只表示该次查询范围内无开放告警。未配置、已禁用、404、403、无分析或命令失败都不能解释为零告警。

## 4. 阻断规则

### 4.1 G3

- 修改安全设置、扫描 workflow、依赖策略或安全治理规则的 PR，必须在 G3 前验证受影响配置，并等待对应首次或最新扫描完成。
- GitHub 为当前 PR 产生的 CodeQL check 必须成功；`pending`、`failure`、`cancelled` 或缺少预期语言分析均不能作为通过。
- 当前 PR 没有产生预期扫描时，必须说明原因；若属于配置、权限或平台异常，应记录显式例外，不得静默忽略。
- 任何与当前变更相关且仍为 open 的 Secret Scanning alert 默认阻断 G3。
- CodeQL 或 Dependabot 的 `high` / `critical` 开放告警默认阻断 G3；若确认与本次变更无关，仍须链接修复 Issue 或按 `development-gates.md` 记录显式例外。
- 修改 Cargo dependency、许可证、`deny.toml` 或依赖更新配置时，cargo-deny 的 advisories、licenses、bans 和 sources 检查必须成功；规则见 `dependency-security.md`。

普通 PR 不要求在正文重复完整 API 快照；Checks、扫描链接和异常判断写入 PR G3 comment。改变仓库设置的治理 PR 还应在 Issue 或 PR 中保留设置变更前后证据。

### 4.2 公开发布或对外分发

公开发布前必须重新读取三类开放告警和能力状态：

- 任何 open Secret Scanning alert 阻断发布。
- CodeQL / Dependabot `high` 或 `critical` 开放告警阻断发布。
- 其他开放告警必须完成分诊，并链接修复 Issue、接受依据或显式例外。
- 未配置、已禁用、目标发布 commit 的适用分析失败或无分析、无权限和 API 不可用均视为未通过，不得用历史零告警替代。

## 5. 可复现验证

使用已认证且具备仓库读取权限的 `gh`。以下命令只读取状态，不应输出或记录 token：

```powershell
gh api repos/illusion-tech/laneflow/code-scanning/default-setup
gh api 'repos/illusion-tech/laneflow/code-scanning/analyses?per_page=100'
gh api 'repos/illusion-tech/laneflow/code-scanning/alerts?state=open&per_page=100'
gh api 'repos/illusion-tech/laneflow/secret-scanning/alerts?state=open&per_page=100'
gh api 'repos/illusion-tech/laneflow/dependabot/alerts?state=open&per_page=100'
gh api repos/illusion-tech/laneflow
cargo deny --locked --all-features check advisories bans licenses sources
```

验证时至少记录：

- 仓库、时间与目标分支或 commit SHA。
- CodeQL setup 的状态、语言、query suite、调度和 runner 类型。
- 最近适用 CodeQL run / analysis 的结论与链接。
- Code Scanning、Secret Scanning、Dependabot 的开放告警数量。
- Secret Scanning、push protection 和 Dependabot security updates 的独立状态。
- `.github/dependabot.yml` 对 Cargo / GitHub Actions 的适用配置，以及 cargo-deny 的版本和四类检查结果。
- 命令失败、权限不足或 plan 限制，以及对应显式例外。

GitHub API 版本或返回结构变化时，应在验证脚本或命令中固定当前受支持版本，并通过治理 PR 更新本文；不得靠忽略字段或错误保持表面通过。

## 6. 配置变更治理

- GitHub 网页或 API 中的设置变更属于仓库外状态变更，必须关联治理 Issue，并记录 G1 决策、G2 开工和实施证据。
- 修改现有 ruleset 时必须保留目标分支、既有规则和 bypass actor；变更后重新读取完整 ruleset，确认只改变预期安全规则。
- 降低或关闭本基线能力属于安全例外，必须在操作前记录原因、风险、到期条件和 Cleanup owner。
- 临时 bypass 只处理被明确阻断的操作，不改变扫描结论；永久 bypass 授权也不能把失败、无分析或开放告警记为通过。
- G4 必须复核文档与 GitHub 实际设置一致，并确认未留下临时权限、临时 workflow 或未跟踪告警。
