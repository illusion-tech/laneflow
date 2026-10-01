# 历史研究诊断 JSON 归档

Refs #812。冻结主线 `bc1bf666a54aebc50a2b7efa50fb1bc3b05ba567`，源码树
`8d10e9d22628be644e75186c3ec2b97cef9817b0`。此次迁移原有 22 个研究目录的
313 份诊断 JSON，共 256176 行、13649924 个 Git blob 字节。当前源码树删除这些
存量文件，旧提交历史保留；五份位于 `research/` 外的契约、配置或测试 JSON 未迁移。

完整源码归档保存冻结提交的 1763 个文件，包括全部被移走的 JSON、研究源码和
历史分析器。`migration-manifest.json` 逐项绑定大小、SHA-256 和原 Git blob，
`SHA256SUMS` 绑定清单自身；清单不在当前源码树重复提交。另有独立 Git bundle
保存该主线的 1920 个可达提交，用于离线恢复原提交 SHA 和 Git 文件模式。

## 下载与复核

制品存放在 [Research Evidence prerelease](https://github.com/illusion-tech/laneflow/releases/tag/research-evidence-json-migration-bc1bf666)。
这不是产品版本或性能认证。制品及清单摘要见[小型 TOML 索引](2026-10-01-json-migration.toml)。

| 制品                                                                                                                                                                   |   字节数 | 用途                       |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------: | -------------------------- |
| [冻结源码 tar.zst](https://github.com/illusion-tech/laneflow/releases/download/research-evidence-json-migration-bc1bf666/issue-812-frozen-source-bc1bf666.tar.zst)     |  5927695 | 原始文件及完整迁移清单     |
| [源码 SHA256SUMS](https://github.com/illusion-tech/laneflow/releases/download/research-evidence-json-migration-bc1bf666/issue-812-source-SHA256SUMS.txt)               |   258732 | 复核 1763 个源码文件及清单 |
| [主线 Git bundle](https://github.com/illusion-tech/laneflow/releases/download/research-evidence-json-migration-bc1bf666/issue-812-frozen-main-history-bc1bf666.bundle) | 25423011 | 离线恢复旧提交历史         |

在仓库外的空目录下载，用本机 SHA-256 工具与 TOML 索引核对全部制品，再解包：

```text
gh release download research-evidence-json-migration-bc1bf666 --repo illusion-tech/laneflow --dir <仓库外归档目录>
tar --zstd -xf <归档目录>/issue-812-frozen-source-bc1bf666.tar.zst -C <新建的解包目录>
```

在解包根目录使用 SHA-256 校验工具，保留校验清单中的相对路径。例如 GNU 工具：

```text
sha256sum --check <归档目录>/issue-812-source-SHA256SUMS.txt
```

JSON 位于 `source/research/<原目录>/<原路径>`。当前报告的文件链接指向冻结提交，
原先的相对 JSON 路径按该 `source/` 目录解释；历史测量数值与校验条件不改写。
需要 Git 上下文时可从 bundle 恢复，再检出原冻结提交或该主线中的早期提交：

```text
git clone --branch codex/812-frozen-main-history <归档目录>/issue-812-frozen-main-history-bc1bf666.bundle <新的历史检出目录>
git -C <新的历史检出目录> fsck --full --strict
```

## 迁移范围

| 原研究目录                      | JSON 文件数 |  行数 |
| ------------------------------- | ----------: | ----: |
| issue-216-exact-path            |           6 | 13385 |
| issue-583-runtime-profile       |           4 |   711 |
| issue-681-pose-extraction       |           1 |    37 |
| issue-682-sparse-cost           |          65 | 13130 |
| issue-707-city-parallel         |          51 | 36060 |
| issue-711-spatial-buffer-swap   |          12 |   378 |
| issue-712-borrowed-sources      |           6 |   192 |
| issue-713-selected-presentation |           1 | 11386 |
| issue-757-current-scope         |           3 |  8726 |
| issue-759-p3-runtime            |           1 |  1550 |
| issue-762-post-p3-hotspots      |           5 |  3807 |
| issue-763-p3-cache              |           2 | 11598 |
| issue-768-p2-cost               |           1 |  5986 |
| issue-770-p2-scope              |           2 | 11801 |
| issue-772-p5-cost               |           1 |  8081 |
| issue-775-p5-direct-active      |           1 |  4365 |
| issue-777-eligibility-commit    |         119 | 29848 |
| issue-779-empty-ranges          |           3 | 43270 |
| issue-793-current-hotspots      |           3 | 13394 |
| issue-801-p5-chunk-grain        |          11 | 15421 |
| issue-805-p5-iidm-simd          |           6 | 11366 |
| issue-808-hot-input-simd        |           9 | 11684 |

`issue-757-current-scope/` 中六个依赖本批固定 JSON 或生成旧身份的 Python 入口
也从当前树移除，原件随完整源码保存。复核这批旧记录时使用归档中的入口及其
对应外部运行包；其他可用于重建、采集的存量源码不在这次迁移中改写。

`issue-711-spatial-buffer-swap/` 与 `issue-712-borrowed-sources/` 的 `analyze.ps1`、
`test-analyze.ps1` 共四个旧脚本退役，两个原始 `evidence/` 目录中的 72 份非 JSON
日志及 CSV 也迁入同一冻结归档。四个脚本和这些原始记录的 Git blob、字节长度及
全新解包的 SHA-256 均与原树一致，无需重采旧性能实验。当前保留简短结果表、
研究源码、历史结论和观察边界；旧复现命令仅在归档源码中查阅。

这两套旧工具不安排例行复核。以后出现相关回归或结论争议，由调查该问题的人按需
恢复归档；新调查另行报告，不覆盖或追加到旧批次，不建立持续维护的当前入口。

## 已完成的验证与边界

冻结归档与全部制品都已实际下载，长度和 SHA-256 与发布身份一致。冻结源码每个
文件的字节数和 Git blob 身份均与原树一致，另在全新目录解包逐文件复核 SHA-256。
Git bundle 已离线恢复，`git fsck --full --strict` 通过，恢复的 HEAD 等于冻结提交。

这次保存的是当时已提交的源码与研究 JSON。各报告另指向的逐拍原始日志、封存
EXE、输入制品和不在主线中的研究分支仍由原报告的身份与地址约束；没有把这些
外部文件补进此次源码归档，也没有重跑历史试验或重新认证其观察窗口。源码 tar
不是 Git checkout；依赖固定提交或 Git 文件模式的重建应使用 bundle 或原 Git
提交，并按历史报告取得外部输入。

保留旧提交意味着普通 Git clone 中的历史对象占用不会因当前树清理而缩小；本次
清理减少当前检出文件与后续 diff 的生成数据负担，符合维护者保留历史的选择。
