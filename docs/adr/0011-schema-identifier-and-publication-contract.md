+++
schema_version = 1
id = "adr-0011"
kind = "adr"
status = "accepted"
summary = "记录历史 JSON Schema 标识与公开发布义务。"
scope = "历史记录：LaneFlow 多 family JSON Schema $id、source/publication 分层、immutable version、历史保留、网络与 CI/CD 边界。现行义务已取消；current JSON loader 已随 #301 从运行入口拆除。"
date = "2026-07-17"
applicability = "#applicability"

related = ["adr-0007", "adr-0008", "adr-0024", "governance-dependency-security", "governance-documentation-policy"]
[[supersedes]]
target = "adr-0008"
extent = "partial"
explanation = "#supersedes-adr-0008"
+++

# 0011 Schema Identifier and Public Publication Contract

<a id="applicability"></a>
## 适用边界与依据

目标态取代登记： 本文的公共发布义务已被 ADR 0026 取代（schema publication
退役）。现行静态数据契约是可移植规范制品（LFCA / LFSM / LFSD / LFCP v2，ADR
0024）；验证收据已取消。不可变发布、规范 URL、来源链和运行时不联网的原则仍
有效。本文 current 契约已随 #301 退出生产路径，只保留历史语义。

关联文档：

- 上游决策:
  - `0007-traffic-data-crate-and-loader-boundary.md`
  - `0008-pre-1.0-data-format-version-policy.md`
  - `0024-compiler-post-emission-check-and-minimal-publication-closure.md`
- 详细设计与治理:
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

`schemas/publication.json` 是 schema source 与发布清单。`contractVersion: 2` 以 `families[]` 分别记录 Traffic、Spatial 与 Scenario Manifest 的 `schemaFileStem`、`currentFormatVersion` 和 `publishedSchemas`。current source 表示 repository loader/tests 当前消费的 schema；只有 published entry 才记录 repository path、canonical URL、完整 source revision 和 Git blob OID，并进入 Pages 与 live monitor。

工作树中的 publication artifact 必须与 `sourceRevision:path` 完全一致。历史 schema 因 public retention 保留在 tree，但它们不是 current loader、fixture、compatibility 或 active data-format contract。ADR 0008 中“旧 schema 不留在 current tree”的部分由本 ADR 修订为“旧 schema 只可作为 immutable publication artifact 保留”。

新 schema family 或新 version 采用两阶段交付：第一阶段把 current source、loader、fixture 与 contract tests 合入 `main`，catalog 可以保留空的 `publishedSchemas`；第二阶段从已存在的 `main` commit 固定 source revision/blob，再把它加入 published set 并部署。这样 source 可在 PR 中完整评审，同时 rebase 不会使 provenance hash 漂移，也不会在部署前误宣称 public retrieval 已完成。一个格式只有在 canonical URL 部署成功并通过 live monitor 后才算完成 public publication。

### 5–6. 历史发布机制（已退役）

GitHub Pages 官方 Actions 部署路径、PR/main CI 的 publication contract 检查命令与
每日 scheduled monitor 的原始操作要求已随 ADR 0026 的 schema publication 退役，不再
是现行指令；操作细节见基线提交 3f80b65f 的本文副本。运行时不联网解析 `$id` /
`$schema`、不把外部发布可用性作为运行前置的边界继续有效（见第 7 节）。

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
- Pages/CD 成为需要维护的外部状态；部署失败必须在 G4 前修复。
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

- #103：恢复 Traffic v0.2-v0.4 publication artifacts，建立 catalog、Pages workflow、live monitor、CI 与文档边界。
- #134：把 catalog 提升为 multi-family contractVersion 2，登记 Spatial/Scenario Manifest v0.1 current source；后续唯一 Delivery PR 固定 `main` provenance、部署并完成 live monitor。
- 首次 1.0 格式冻结、迁移到自有域名/CDN、修改 retention SLA 或需要严格 `application/schema+json` headers 时，创建独立 Issue 和 ADR 修订。


## 取代范围

<a id="supersedes-adr-0008"></a>
### adr-0008

[adr-0008][replacement-adr-0008]：当时对已公开 schema 增加不可变发布与永久保留义务；该后继义务又由 ADR 0026 取消，不恢复旧 loader 兼容。


[adr-0026]: 0026-merge-governance-rebuild.md

[replacement-adr-0008]: 0008-pre-1.0-data-format-version-policy.md
