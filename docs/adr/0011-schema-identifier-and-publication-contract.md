# 0011 Schema Identifier and Public Publication Contract

**状态**: Accepted
**日期**: 2026-07-17
**适用范围**: LaneFlow JSON Schema `$id`、public retrieval、immutable version、历史保留、网络与 CI/CD 边界

**关联文档**:

- 上游决策:
  - `0007-traffic-data-crate-and-loader-boundary.md`
  - `0008-pre-1.0-data-format-version-policy.md`
- 详细设计与治理:
  - `../design/data-format.md`
  - `../design/data-loading.md`
  - `../governance/documentation-policy.md`
  - `../governance/dependency-security.md`
  - `../../schemas/README.md`
  - `../../schemas/publication.json`

## 背景

LaneFlow 的 checked-in JSON Schema 使用 versioned HTTPS `$id`。v0.2/v0.3 的 `$id` 指向 `raw.githubusercontent.com` 的 `main` 路径；v0.4/v0.5 改为 organisation-owned GitHub Pages 路径。历史 schema 被当前版本替换后，v0.2/v0.3 raw 路径与 v0.4/v0.5 Pages 路径均出现 404，导致 closure 证据、消费者预期和实际发布状态漂移。

JSON Schema Draft 2020-12 把 `$id` 定义为 schema resource 的 canonical URI，并说明该 URI 不必是网络 locator。LaneFlow 可以选择 identifier-only，但公共生态中的 versioned HTTPS `$id` 会自然形成稳定 retrieval 预期。项目所有者在 #103 的 G1 明确选择 public canonical publication contract，因此需要把可达性、不可变性、历史保留、发布和监测作为长期治理责任。

ADR 0007 仍要求 production loader 只接收调用方提供的内存 bytes/string；本 ADR 增加 distribution/CD 能力，不改变 runtime I/O 边界。

## 决策

### 1. `$id` 同时是 canonical identifier 与 retrieval URL

每个列入 `schemas/publication.json` 的 schema `$id` 必须：

- 使用 HTTPS absolute URI，不含 fragment；
- 返回 HTTP 200；
- 返回可解析的 JSON Schema document；
- 与其受审、固定 source revision 中的内容逐字节一致；
- 在发布后保持 URL 和内容不可变。

JSON Schema Draft 2020-12 定义 `application/schema+json`；GitHub Pages/Raw 的静态服务可能返回兼容 JSON media type。自动监测接受 `application/schema+json` 或 `application/json`；v0.2/v0.3 的既有 Raw GitHub `$id` 可以保留 `text/plain` 作为历史平台例外，但响应体仍必须是合法 JSON 且逐字节一致。新 `$id` 不再使用 Raw GitHub URL。

### 2. 已发布 version 永久保留且不可原地修改

已列入 publication catalog 的版本不得删除、重命名或修改内容。格式修正必须提升 `formatVersion` 并发布新的 versioned `$id`；旧 loader 是否支持该版本与 schema 是否继续可下载是两个独立问题。

只有法律或安全紧急事件可以移除或替换已发布 artifact。例外必须先记录原因、影响、迁移路径、tombstone 行为、Cleanup owner 与批准证据；普通兼容性修复不得使用该例外。

### 3. 恢复历史 `$id` 并统一发布目录

v0.2/v0.3 重新保留在其原始 repository path，以恢复既有 Raw GitHub `$id`：

- `schemas/laneflow-data-v0.2.schema.json`
- `schemas/laneflow-data-v0.3.schema.json`

v0.4/v0.5 通过 GitHub Pages 发布：

- `https://illusion-tech.github.io/laneflow/schema/laneflow-data-v0.4.schema.json`
- `https://illusion-tech.github.io/laneflow/schema/laneflow-data-v0.5.schema.json`

Pages 同时提供全部 published versions 的副本和 machine-readable catalog，便于发现；每个 schema 内部 `$id` 仍保持其原始 canonical URI。

### 4. Publication catalog 固定 provenance

`schemas/publication.json` 是发布清单，记录 current version、永久保留策略，以及每个版本的 repository path、canonical URL、完整 source revision 和 Git blob OID。

工作树中的 publication artifact 必须与 `sourceRevision:path` 完全一致。历史 schema 因 public retention 保留在 tree，但它们不是 current loader、fixture、compatibility 或 active data-format contract。ADR 0008 中“旧 schema 不留在 current tree”的部分由本 ADR 修订为“旧 schema 只可作为 immutable publication artifact 保留”。

新 schema 的 source revision 必须已经存在于 `main`，再由 publication 变更加入 catalog；这样 rebase 不会使 provenance hash 漂移。一个格式只有在 canonical URL 部署成功并通过 live monitor 后才算完成 public publication。

### 5. GitHub Pages 使用官方 Actions 发布路径

GitHub Pages 配置为 `build_type=workflow` 且强制 HTTPS。部署 workflow：

- 只从 `main` push 或以 `main` 为 ref 的人工 dispatch 发布；其他 ref 只能构建 artifact，不能部署；
- 使用官方 `configure-pages`、`upload-pages-artifact` 和 `deploy-pages` actions，并固定完整 commit SHA；
- 使用 `github-pages` environment、`pages: write`、`id-token: write` 与单并发 deployment；
- 从 validated publication catalog 构建 `_site/schema/`，不手写第二套页面内容；
- deployment 完成后重新读取 canonical URLs 并验证内容。

### 6. CI 与定时监测

PR/main CI 运行：

```powershell
cargo +1.96.0 run --locked -p xtask -- check-schema-publication-contract
cargo +1.96.0 run --locked -p xtask -- build-schema-publication target/schema-publication-site
```

检查覆盖 catalog shape、版本顺序、current version、文件名、`$schema`、`$id`、source revision/blob OID、working-tree/source byte equality、runtime source 无 canonical URL 硬编码，以及 Pages artifact 构建。

每日 scheduled monitor 对 catalog 的全部 canonical URLs 检查 HTTPS、最终 HTTP 200、兼容 media type、合法 JSON 与 byte equality。失败日志必须列出 URL、HTTP/media type、期望/实际 content digest 和重新部署命令。404、网络错误、内容漂移或解析失败都不是“零问题”。

### 7. Runtime 与安全边界不增加网络依赖

Core、`laneflow-data` production loader、Adapter 与 hermetic runtime tests 不联网解析 `$id` 或 `$schema`，也不把 Pages/Raw GitHub 可用性作为 load、world creation 或 tick 的前置条件。

调用方主动下载 schema/package 时，网络内容属于调用方引入的不可信输入，应限制大小、验证 revision/content、缓存并处理故障。Publication monitor 属于 distribution availability evidence，不替代 CodeQL、Secret Scanning、Dependabot、cargo-deny 或 release security audit。

## 后果

正向后果：

- `$id`、retrieval URL 与受审内容形成稳定、机器可验证的公共契约。
- 历史版本不会再因 active format 升级而静默 404。
- Pages deployment、catalog provenance 与 scheduled monitor 提供可操作的漂移证据。
- runtime 继续 hermetic，不因外部网络故障改变行为。

成本与风险：

- 仓库必须永久保留已发布 schema artifact 和对应 URL。
- Pages/CD 成为需要维护的外部状态；部署失败必须在 G4 前收口。
- v0.2/v0.3 既有 Raw GitHub URL 的 media type 不是新版本的推荐基线，但修改 `$id` 会破坏已发布 identity，因此只记录历史例外。
- 新格式采用两阶段 publication：先让 schema revision 进入 `main`，再固定 provenance 并发布。

## 替代方案

### Identifier-only

最省维护成本且符合 JSON Schema 核心规范，但无法兑现项目所有者要求的公共 retrieval 契约，也会继续保留 versioned HTTPS `$id` 的 404，因此拒绝。

### 只发布 current schema

实现简单，但 versioned URL 会在下一次格式升级后失效，不符合 public immutable version 预期，因此拒绝。

### 修改历史 `$id` 统一到 Pages

可以统一 hosting/media type，但会修改已发布 schema identity 与 bytes。v0.2/v0.3 保留原始 `$id`，Pages 只提供额外发现副本，因此拒绝原地修改。

## 实施与复核

- #103：恢复 v0.2-v0.4 publication artifacts，建立 catalog、Pages workflow、live monitor、CI 与文档边界。
- 首次 1.0 格式冻结、迁移到自有域名/CDN、修改 retention SLA 或需要严格 `application/schema+json` headers 时，创建独立 Issue 和 ADR 修订。
