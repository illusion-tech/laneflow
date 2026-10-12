# 已关闭研究 crate 代码退役归档

Refs #897。冻结主线 `bcd6918ab22e65b6619624ee9b96885e46db8ffa`，源码树
`fb23f99b6b9ea85095322791aac2a68ea0eef251`。此次退役 6 个已关闭研究目录的 Cargo
manifest、lockfile、源码与构建脚本，共 28 个文件、361915 个 Git blob 字节。当前
源码树删除这些文件，旧提交历史保留；各目录的 README 结论、结果表与既有证据文件
保留在原位。

`retirement-manifest.json` 逐项绑定大小、SHA-256 和原 Git blob，随源码 tar 保存；
`SHA256SUMS` 绑定清单自身。另有独立 Git bundle 保存该主线的 2145 个可达提交，
用于离线恢复原提交 SHA 和 Git 文件模式。

## 下载与复核

制品存放在 [Research Evidence prerelease](https://github.com/illusion-tech/laneflow/releases/tag/research-evidence-research-crate-retirement-bcd6918a)。
这不是产品版本或性能认证。制品及清单摘要见[小型 TOML 索引](2026-10-12-research-crate-retirement.toml)。

| 制品                                                                                                                                                                         |   字节数 | 用途                     |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------: | ------------------------ |
| [退役源码 tar.gz](https://github.com/illusion-tech/laneflow/releases/download/research-evidence-research-crate-retirement-bcd6918a/issue-897-retired-source-bcd6918a.tar.gz) |    96286 | 原始文件及逐文件清单     |
| [源码 SHA256SUMS](https://github.com/illusion-tech/laneflow/releases/download/research-evidence-research-crate-retirement-bcd6918a/issue-897-source-SHA256SUMS.txt)          |     3508 | 复核 28 个源码文件及清单 |
| [主线 Git bundle](https://github.com/illusion-tech/laneflow/releases/download/research-evidence-research-crate-retirement-bcd6918a/issue-897-main-history-bcd6918a.bundle)   | 26826387 | 离线恢复旧提交历史       |

在仓库外的空目录下载，用本机 SHA-256 工具与 TOML 索引核对全部制品，再解包：

```text
gh release download research-evidence-research-crate-retirement-bcd6918a --repo illusion-tech/laneflow --dir <仓库外归档目录>
tar -xzf <归档目录>/issue-897-retired-source-bcd6918a.tar.gz -C <新建的解包目录>
```

在解包根目录使用 SHA-256 校验工具复核，校验清单中的相对路径以解包根为基准：

```text
sha256sum --check <归档目录>/issue-897-source-SHA256SUMS.txt
```

退役文件位于 `source/research/<原目录>/<原路径>`。需要 Git 上下文时可从 bundle
恢复，再检出原冻结提交或该主线中的早期提交：

```text
git clone --branch codex/897-frozen-main-history <归档目录>/issue-897-main-history-bcd6918a.bundle <新的历史检出目录>
git -C <新的历史检出目录> fsck --full --strict
```

## 退役范围

| 研究目录                      | 退役文件数 | 退役内容                                                                          |
| ----------------------------- | ---------: | --------------------------------------------------------------------------------- |
| issue-123-spatial-prototype   |          3 | Cargo.toml、src/、tests/；同时退出主 workspace                                    |
| issue-543-cn-urban-capacity   |          3 | Cargo.toml、Cargo.lock、src/                                                      |
| issue-681-pose-extraction     |          7 | Cargo.toml、Cargo.lock、src/、analyze/run/test-analyze 脚本                       |
| issue-682-sparse-cost         |          3 | Cargo.toml、src/；同时退出主 workspace                                            |
| issue-711-spatial-buffer-swap |          5 | .gitignore、Cargo.toml、Cargo.lock、src/、run.ps1                                 |
| issue-712-borrowed-sources    |          7 | .gitignore、Cargo.toml、Cargo.lock、src/、run 脚本、combination-observation.patch |

各目录保留 README、结果表（results、summary-table、measurements）与既有
`evidence/` 文件。`issue-762-post-p3-hotspots` 不在本次范围：它是 P2/P3/P5
性能研究链的代码载体，相关研究仍在引用。

这两套（主 workspace 退出的）成员身份只存在于冻结提交之前；复核历史构建命令时
使用归档制品或原提交，不要把退役 crate 重新加回生产 crate 或 workspace 成员。

## 已完成的验证与边界

冻结归档与全部制品都已实际下载，长度和 SHA-256 与发布身份一致。退役源码每个
文件的字节数和 Git blob 身份均与原树一致，另在全新目录解包逐文件复核 SHA-256。
Git bundle 已离线恢复，`git fsck --full --strict` 通过，恢复的 HEAD 等于冻结提交。

这次保存的是当时已提交的退役文件。各报告另指向的外部采集原包、二进制和输入
制品仍由原报告的身份与地址约束；没有把这些外部文件补进此次归档，也没有重跑
历史试验或重新认证其观察窗口。退役代码不是 Git checkout；依赖固定提交或 Git
文件模式的重建应使用 bundle 或原 Git 提交。

保留旧提交意味着普通 Git clone 中的历史对象占用不会因当前树清理而缩小；本次
清理减少当前检出的维护面（Dependabot 更新目标与主 workspace 构建成员），符合
维护者保留历史的选择。
