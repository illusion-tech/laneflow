# 安全扫描基线

**文档状态**: Active  
**最后更新**: 2026-08-07
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

#### 2.1.1 Dependabot Cargo.lock-only PR 的适用性

CodeQL default setup 可以对只修改 `Cargo.lock` 的 PR 返回已完成的 aggregate
`CodeQL=neutral`（例如 `configurations not found`），也可能暂时未创建 current-PR analysis。
前者可进入窄例外判定，后者必须保持 `missing` 并等待明确结果。`check-codeql` 固定输出
`pass`、`not_applicable`、`pending`、`failed`、`missing` 或 `provider_error`：

- PR-bound rollup 选定、REST source App 精确为 `github-advanced-security` 的 aggregate `CodeQL=success` 才是常规 `pass`；每个未按 details URL 出现在 rollup 的官方 REST current-head CodeQL run 都必须补为候选，再进入同一唯一性与 PR/head/base/app 校验。该规则同时覆盖 rollup 只有同名 `github-actions` spoof，以及两次读取之间新增第二个官方失败/取消 run 的竞态，不能丢弃它们后误判 no-analysis 或沿用单个 success。任何官方候选因缺少或不匹配 `pull_requests` association 而无法绑定时都返回 `provider_error`，即使 lockfile-only policy 原本可能进入 `not_applicable`；普通源码 PR 的官方 success 则不依赖仅供 lockfile 例外使用的完整 commit/signature/force-push provenance，provenance 分页或 provider 错误只会使该窄例外不适用。已完成 run 必须提供有效 `completed_at`，G3 comment 必须严格晚于该时间，带小数秒时按秒与纳秒精确比较。OPEN target 的 check-run `pull_requests` association 还必须精确匹配目标 PR number、current head 与 current base；G4 对 MERGED PR 重放时按 append-only G3 comment 记录的 run URL 读取该具体 REST check-run，并复核 source App、head 与结论，允许 GitHub 清空 association，但存在的 association 必须仍精确匹配。同 head 的其他 PR/base 分析、合并后新增的 latest rerun 与同名 `github-actions` job 均不计入原 G3 证据；
- 只有 `dependabot-cargo-lock-only-v1` 能把已完成 aggregate `NEUTRAL` / no-analysis 判为
  `not_applicable`；空 rollup 仍为 `missing`。该 policy 与 external-review 机器替代路径共用同一组证明：Dependabot
  App / bot PR author、同 repository 的 `dependabot/cargo/*` head ref、唯一 current-head
  commit、GitHub `web-flow` verified signature、完整 Dependabot force-push provenance、精确
  Dependabot commit identity、非 breaking `build(deps):` 标题、完整且唯一的
  `MODIFIED Cargo.lock` changed path；`SKIPPED` 不等于无分析并继续失败关闭；
- 普通源码、Actions workflow、mixed-path、人工 lockfile commit、分页/身份歧义、
  `failure` / `cancelled` 或平台错误不得使用 `not_applicable`；
- `not_applicable` 只说明 CodeQL 对该 PR diff 无受支持源码分析输入，不替代
  cargo-deny、workspace tests、Dependabot/open-alert 审计或合并后 `main` 分析。

从 `2026-08-08T00:00:00Z` 创建的 current G3 comment 起，必须唯一记录
`- CodeQL：`：常规路径写 `pass` 与 Check URL；窄路径写 `not_applicable`、精确 policy
ID 与 neutral/no-analysis Check 证据 URL。状态必须是字段后的首个且唯一 backtick
状态，URL 必须与机器结果精确相等，不能附加数字、query 或 fragment。Gate 结果为 `G3 Waived` 时也不能把 waiver
冒充 CodeQL 结论；external-review waiver 不覆盖 CodeQL，waived review 仍必须独立满足
上述 `pass` / `not_applicable`。激活边界按解析后的 UTC 秒值比较，带小数秒的同秒时间不会
因 RFC3339 字符串排序而绕过字段或 live 校验。

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

#### Deployment 失败与重试边界

当前官方 `actions/deploy-pages` 路径把 `GITHUB_SHA` 作为 `pages_build_version`。当某一版本的 deployment 在超时后被取消或进入终态失败时，同一 SHA 的 workflow rerun 或独立 dispatch 不能作为新的部署身份证据；操作者不得把同一 SHA 的重复失败解释为 schema 内容漂移，也不得用无界重试替代调查。

处置顺序：

- 保存首次 deployment、失败 job rerun、独立 dispatch 和 live monitor 的运行链接、状态与 SHA；
- 通过 Pages、environment、deployment 和 Actions queued / in-progress API 区分内容问题、排队状态与终态失败；
- 没有残留 deployment 或队列证据时，不删除或重建 Pages site / environment，也不删除历史 deployment；
- 等待合法的后续不同 `main` SHA；若 path filter 未触发，从该 SHA 对应的 `main` ref 手工 dispatch；
- 只有 build、deploy、canonical verify 全部成功且 Pages deployment status 为 `succeed`，才能把 publication 恢复写入 G4 证据；独立 monitor 成功只证明线上内容当前可用且 byte equality 成立，不证明失败 deployment 已恢复；
- 后续不同 SHA 仍失败时，停止同类重试，回到 G1 评估 workflow 变更，或携带已保存的平台证据升级调查。

## 3. 状态语义

安全审计必须记录能力状态、最近运行状态和开放告警结果，不得只写“通过”。

| 状态               | 判定要求                                                             | 可否写“零开放告警”                          |
| ------------------ | -------------------------------------------------------------------- | ------------------------------------------- |
| 已配置且成功       | 功能已启用；CodeQL 最近适用分析成功；alerts API 成功返回空集合       | 可以，同时记录时间、分支或 commit、运行链接 |
| 已配置，分析待完成 | 功能已启用，但首次或最近适用分析仍为 `pending` / `queued`            | 不可以                                      |
| 已配置，无分析     | 功能已启用，但找不到适用分析或分析未覆盖目标语言 / commit            | 不可以                                      |
| 明确不适用         | PR 精确满足 `dependabot-cargo-lock-only-v1`；仅该 PR diff 无适用分析 | 不可以；仍须单独读取 open alerts API        |
| 分析失败或降级     | 最近适用分析失败、取消、超时，或预期语言缺失                         | 不可以                                      |
| 未配置             | 平台返回 `not-configured` 或没有对应 setup                           | 不可以                                      |
| 已禁用             | 平台明确返回 `disabled`                                              | 不可以                                      |
| 无权限或不可用     | API 返回权限、plan、组织策略或平台可用性错误                         | 不可以；必须记录显式例外                    |

对三类能力分别采用以下最低证据：

- Code Scanning：default setup 为 `configured`，预期语言存在，最近适用 run / analysis 成功，open alerts API 成功返回。
- Secret Scanning：功能与 push protection 均为 `enabled`，open alerts API 成功返回。
- Dependabot：vulnerability alerts 可用，security updates 为 `enabled`，open alerts API 成功返回；version updates 配置存在且适用。设置状态、空告警和 cargo-deny 结果必须分别记录。

API 返回空集合只表示该次查询范围内无开放告警。未配置、已禁用、404、403、无分析或命令失败都不能解释为零告警。

## 4. 阻断规则

### 4.1 G3

- 修改安全设置、扫描 workflow、依赖策略或安全治理规则的 PR，必须在 G3 前验证受影响配置，并等待对应首次或最新扫描完成。
- GitHub 为当前 PR 产生的 CodeQL aggregate check 必须成功；或由 `check-codeql` 对精确 Dependabot `Cargo.lock`-only PR 返回 `not_applicable`。`pending`、`failure`、`cancelled`、普通 PR 缺少预期语言分析均不能作为通过。
- 当前 PR 没有产生预期扫描时，必须运行并记录 `check-codeql`；不满足精确 `not_applicable` policy 时，配置、权限或平台异常必须记录显式例外，不得静默忽略。
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
cargo +1.96.0 run --locked -p xtask -- check-codeql --repo illusion-tech/laneflow --pr <number> --format json
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
- 新增 required workflow / required Check 时必须保存完整 before / after ruleset JSON，确认 workflow 或 expected source App 的身份绑定，并用同名 Actions spoof canary 和无 `--admin` 合并验证证明真实阻断。仅新增 non-required shadow workflow 不得同时改写为“已启用强制门禁”；shadow 对 review/thread、Related→Delivery 级联、Draft/closed 失效和 marker 时序的刷新只能作为 R1 telemetry 证据，不能替代 expected source 绑定。
- organization ruleset、required workflow 或安全设置 API 返回 `403` / `404` 时，记录 plan / permission 限制和原始错误；不得把不可读取解释为零规则、零告警或能力已关闭，也不得据此覆盖现有 ruleset。
- 降低或关闭本基线能力属于安全例外，必须在操作前记录原因、风险、到期条件和 Cleanup owner。
- 临时 bypass 只处理被明确阻断的操作，不改变扫描结论；永久 bypass 授权也不能把失败、无分析或开放告警记为通过。
- G4 必须复核文档与 GitHub 实际设置一致，并确认未留下临时权限、临时 workflow 或未跟踪告警。
