# 许可证与依赖安全基线

**文档状态**: Active  
**最后更新**: 2026-07-17
**适用范围**: LaneFlow 公开仓库的源代码许可、Rust/Cargo 依赖许可证、漏洞、来源与持续更新治理  
**关联 Issue**: `#56`

## 1. 目标

本文把 ADR 0002 的架构约束转换为可执行门禁，并明确以下长期事实：

- LaneFlow 公开仓库按什么许可证分发。
- 开放 Core/Data 与未来商业产品如何隔离。
- Rust 依赖允许哪些许可证和来源。
- cargo-deny、Dependabot、CI 与人工评审分别承担什么职责。
- 例外需要哪些证据，何时阻断合并或发布。

本文不是针对某个司法辖区或具体发行方案的法律意见。正式商业发行仍应按实际分发物、第三方材料和目标市场复核合规要求。

## 2. LaneFlow 源代码许可证

### 2.1 公开仓库

LaneFlow 公开仓库采用 **Apache License 2.0-only**：

- 根目录 `LICENSE` 保存未经修改的 Apache License 2.0 标准文本。
- workspace package 使用 SPDX expression `Apache-2.0`。
- 除非文件或目录另有明确声明，本仓库自有源代码、schema、文档、治理工具和示例均按 Apache-2.0 分发。
- 第三方代码、数据、字体、素材或生成物继续遵循其自身许可证；不得用根许可证覆盖第三方条款。

当前仓库不提供 MIT 双许可。调整本仓库源代码许可证属于所有者法律与商业决策，必须通过独立 Issue、ADR 修订和显式批准，不得只修改 Cargo metadata。

### 2.2 开放与商业边界

开放仓库长期承载可复用的 Runtime、编译器/格式、道路编辑来源、公共 API、测试和最小示例。高级编辑器、城市级或分布式仿真、优化分析、企业 Adapter、云服务和商业支持可以在独立产品、独立仓库或独立分发物中使用商业许可证。

依赖方向固定为：

```text
commercial products -> open laneflow-runtime / laneflow-format
open repository -X-> commercial implementation
```

商业产品可以组合、修改和分发 Apache-2.0 代码，但仍须满足 Apache-2.0 的许可证副本、修改说明、既有 notices 保留和其他适用义务。开放仓库不得依赖未公开的商业实现，否则会破坏公开构建、测试和贡献边界。

### 2.3 NOTICE 与贡献

- Apache-2.0 不要求项目创建空的 `NOTICE`。只有实际分发内容带来需要传递的 attribution notices 时才新增并维护该文件。
- 贡献采用 inbound = outbound：除非贡献者明确书面声明并获得维护者同意，提交到本仓库并被接收的 Contribution 按 Apache-2.0 许可。
- 当前不要求 CLA。若未来需要把贡献同时用于不能由 Apache-2.0 覆盖的商业授权，再通过独立治理 Issue 评估 CLA 或其他贡献协议。

### 2.4 Schema 分发与网络输入

本仓库自有 schema 与代码、文档一样按 Apache-2.0 分发。ADR 0011 的公共 schema 发布义务已被 ADR 0026 取代；current JSON Schema 已随 #301 删除。道路编辑 FlatBuffers 与 Runtime、Adapter、hermetic tests 不联网解析 `$id` 或 `$schema`。

Publication workflow 必须固定第三方 Action 完整 commit SHA，并以 catalog source revision/byte equality 防止 artifact 漂移。消费者下载 schema/package 时仍须在其边界处理来源、完整性、缓存、输入大小和网络失败；远端内容不能因 URL 位于本组织域名下就绕过不可信输入治理，也不得把网络 fetch 隐式加入生产 crate。

## 3. Cargo metadata

根 `Cargo.toml` 的 `[workspace.package]` 是 workspace package license 的单一事实源：

```toml
[workspace.package]
license = "Apache-2.0"
```

每个 workspace package 必须声明 `license.workspace = true`。新增 package 若不适用该许可证，必须先拆分分发边界并记录显式例外，不得静默省略 `license` 或使用模糊的 `license-file` 覆盖 workspace 决策。

不加入 workspace 的独立 research workspace 同样受本文约束，不得因脱离根依赖图而免于
许可证与 RustSec 审计；其归属、license 声明与审计入口要求见第 8 节。

## 4. 第三方依赖政策

### 4.1 允许列表

`deny.toml` 是机器可执行的精确允许列表。当前允许：

- `Apache-2.0`
- `MIT`
- `Unicode-3.0`
- `Zlib`

SPDX `OR` expression 只要至少一个分支位于允许列表即可通过；`AND` expression 的每个组成许可证都必须被允许。即使某个依赖表达式还列出 BSD、MIT-0、Unlicense 或 LLVM exception，只要 cargo-deny 当前通过的是已允许分支，也不代表这些许可证已经被全局预先批准。允许列表的新增不是普通依赖升级，必须说明分发影响并更新 ADR 0002 或记录其无需 ADR 的依据。

### 4.2 默认阻断

以下情况默认阻断合并和公开发布：

- cargo-deny 报告未允许或无法确定的许可证。
- GPL、AGPL 或其他强 copyleft 依赖进入发布依赖图。
- LGPL、MPL 等弱 copyleft 未经逐项分发审阅和显式例外。
- crates.io 之外的 registry 或任意 Git dependency 未进入明确允许范围。
- wildcard dependency requirement。
- RustSec vulnerability advisory 未修复且没有合格例外。

重复版本当前为 warning，不单独阻断。维护者应在升级 PR 中判断重复版本是否造成二进制体积、编译时间或安全修复分叉；需要长期保留时，应留下原因。

## 5. 自动化职责

### 5.1 cargo-deny

LaneFlow 固定 cargo-deny `0.20.2` 作为当前依赖政策执行器。CI 直接下载官方 `x86_64-unknown-linux-musl` release asset，并在解压执行前校验仓库固定的 SHA-256；不执行浮动 tag 或未经 checksum 校验的安装脚本。CI 运行：

```powershell
cargo deny --locked --all-features check advisories bans licenses sources
```

CI job 名称固定为 `Dependency policy`，并由 `main` ruleset 的 required status checks 强制要求成功。Merge Queue
启用后，`strict_required_status_checks_policy=false`：current exact `H_pr` 必须在合并前通过该检查，GitHub 还必须
在最新 `main` 与队列前序变更形成的真实 `H_mg` 上重新运行并成功后才可合并；目标分支单纯前进不要求作者
更新 PR 分支。仅新增 workflow job 而不维护 ruleset required context，或只验证 `H_pr` 而不验证 `H_mg`，
都不算建立硬门禁。

检查职责：

- `advisories`：读取 RustSec Advisory Database；未忽略的 vulnerability、直接依赖的 unmaintained advisory 与完整依赖图的 unsound advisory 阻断。
- `licenses`：验证 workspace 与完整依赖图的 SPDX expression。
- `bans`：拒绝 wildcard dependency，并报告重复版本。
- `sources`：只允许 crates.io registry，拒绝未知 registry 与 Git source。

本地工具版本必须与 CI 保持一致。升级 cargo-deny 时应单独审查 release notes、配置兼容性、release asset URL 与 SHA-256，不得只修改版本号或跳过 checksum 更新依据。

### 5.2 Dependabot

仓库必须保持以下状态：

- Dependabot vulnerability alerts 可用。
- Dependabot security updates 为 `enabled`。
- `.github/dependabot.yml` 每周检查 Cargo 与 GitHub Actions version updates。
- 每个 ecosystem 最多同时打开 5 个 version update PR，避免维护队列失控。

Dependabot PR 仍必须通过测试与 cargo-deny，并按 `security-scanning.md` 与 `development-gates.md` 完成 CodeQL 和 GitHub 原生对话处理。包括精确满足 `dependabot-cargo-lock-only-v1` 的 PR 在内，都必须让 `Analyze (actions)` 与 `Analyze (rust)` 实际成功；该窄 profile 不再提供 CodeQL `not applicable`。自动生成不等于自动批准或自动合并。

Dependabot 无法生成 LaneFlow 的完整治理正文，因此 commit 校验器仅对同时满足以下条件的机器提交提供窄例外：

- Git author name 精确为 `dependabot[bot]`。
- Git author email 精确为 `49699333+dependabot[bot]@users.noreply.github.com`。
- 标题为非 breaking 的 `build(deps): <description>`。

该例外不是身份认证机制，也不适用于人工依赖提交、其他 bot 或其他 scope。PR 审阅和安全检查仍须完整执行。

### 5.3 sccache

LaneFlow 固定 sccache `0.19.0` 作为 CI 编译缓存（内容寻址编译缓存，#1074）。CI 直接下载官方 `x86_64-unknown-linux-musl` release asset，并在解压执行前校验仓库固定的 SHA-256（`aa73c7270c997894038c0d8f0d297865f57569e1687883322bc431713d4bb9dd`，与 GitHub 发布 API 的 asset digest 交叉核对）；不执行浮动 tag 或未经 checksum 校验的安装脚本。升级 sccache 时必须单独审查 release notes、release asset URL 与 SHA-256，并复核 GHA 后端行为语义，不得只修改版本号。

作用域与信任面边界（依据 #1071 评估记录 §2/§7）：

- `RUSTC_WRAPPER` 只在 `Rust checks`、`Evidence fixture tests`、`Placement fixture tests`、`Urban harness tests` 四个 job 启用；`Wire toolchain audit` 不启用。wire 审计的 hermetic 子编译经 `HERMETIC_SCRUB_ENV_EXACT` 剔除包装器环境，`SCCACHE_*` 配置对审计编译惰性；审计器引导构建不经缓存路径，门禁信任面不扩大。
- 后端为 GitHub Actions 缓存（`SCCACHE_GHA_*`，键前缀 `sccache-v1-`）：写只在 `main` push，PR 与 `merge_group` 只读；失败语义为默认失败关闭，不设置 `SCCACHE_IGNORE_SERVER_IO_ERROR`。缓存内容无签名的残余风险按 #1071 评估记录 §4 的缓解清单处置（编译器身份入键、命中率统计进日志、`SCCACHE_GHA_VERSION` 换代清场、配额纳入运维观察）。
- sccache 以 Apache-2.0 发布，落在本文允许列表内；作为 workspace 外二进制工具，不新增 crate 依赖面，RustSec / cargo-deny 审计归属不变。

## 6. 例外治理

`deny.toml` 中每个 advisory ignore、license exception、crate skip 或 source allow 都必须至少记录：

- 精确 advisory ID、crate/version、许可证或 source。
- 业务必要性与替代方案。
- 对发布物的法律、安全和维护影响。
- 接受范围与复核/到期条件。
- Cleanup owner。
- 跟踪 Issue。

没有理由的裸字符串 ignore、无到期边界的通配例外或仅为“让 CI 通过”的放宽均不接受。若工具配置无法承载全部字段，应在邻近注释与关联 Issue 中共同保存。

## 7. 合并与发布验证

修改依赖、Cargo manifest、许可证、`deny.toml`、Dependabot 或依赖 CI 的 PR，至少运行并记录：

```powershell
cargo +1.99.1 metadata --locked --format-version 1
cargo deny --locked --all-features check advisories bans licenses sources
cargo +1.99.1 test --workspace --locked
```

还必须按 `security-scanning.md` 读取 GitHub 的实际 Dependabot setting 与 open alerts。API 失败、功能 disabled、cargo-deny 未运行或 policy check 失败都不能记为零告警或通过。

合并前还必须确认 GitHub 上的 `Dependency policy` required check 已成功完成；missing、pending、skipped、cancelled 或 failure 均阻断。

公开发布前重新验证目标 commit，不得复用历史截图或旧 PR 的空告警结果。发布分发物若包含仓库外第三方材料，还必须单独生成/复核 attribution 清单。

## 8. 研究与非生产 crate 的归属与审计

研究代码（调研 spike、基准工具、诊断执行器）不是生产包、公共应用程序接口（API）或
已接受的数据格式，但其第三方依赖、许可证与来源同样受本文约束，不因不加入 workspace
而脱离本文范围。

### 8.1 workspace 归属

research crate 可以是 workspace 成员，也可以用独立 `[workspace]` 自隔离。归属由立项
Issue 决定，并在该目录 `README.md` 声明理由；两种形态都不是默认合规，也不是默认违规：

- **workspace 成员**：必须声明 `license.workspace = true` 并继承 workspace lints，由
  根 `cargo deny check` 与 CI `Dependency policy` 覆盖。
- **独立 `[workspace]`**：脱离根依赖图，必须显式声明 license，并另行提供等价审计入口，
  不得默认逃逸 `cargo deny` 与 Dependabot；逐目录审计命令必须写进该目录 `README.md`。

两种形态都必须 `publish = false`，并在 `README.md` 声明非生产身份（不是生产包、不是公共
API、不是已接受的数据格式）。

### 8.2 审计覆盖

任何形态的研究 crate 的第三方依赖都必须进入许可证与 RustSec 审计，使审计范围与实际
依赖范围一致。若再次出现独立 research workspace，必须在 `Dependency policy` required
check 中逐目录执行 `cargo deny check`（或等价方案），并由 Dependabot 逐目录维护；不得把
「未执行」「不可读取」记为零告警。

### 8.3 历史禁令的适用范围

`#308` 的「研究执行器不得加回工作区」只约束 #308 自身（原文见
`docs/design/compiler-budget-calibration.md`），不是项目级通则。不得把它扩写成
「研究代码不得进入生产 crate 或 workspace 成员」一类的全局规则；引用历史禁令时必须
保留其原本的适用范围。
