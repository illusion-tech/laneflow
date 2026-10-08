# #834 远处限速下降准入筛选：冻结归档索引

候选未采用，见[结果报告](../admission-distant-results.md)。本索引只绑定封存后的制品身份，
不改写被测源码、原始记录或包内报告快照。

## 下载与身份

- [研究发布](https://github.com/illusion-tech/laneflow/releases/tag/research-evidence-834-admission-distant)。
- [admission-distant-834-ae53db85.tar.zst](https://github.com/illusion-tech/laneflow/releases/download/research-evidence-834-admission-distant/admission-distant-834-ae53db85.tar.zst)。
- 压缩包大小：`142317355` 字节。
- SHA256：`6f26cd5452fa25ce79142e7ad93a4f1d9063977e4285405339645a4c0543300a`。
- 被测 A：`2d92889b84022c65fddd1e1f440ac56d80eed6b0`。
- 被测 B：`d7624fb21da11566e8a4a4a624da3e84c45937c9`。
- 撤回：`9e38e3ed468b1462f44ecf22838cb27efc113c18`。
- 报告快照：`ae53db8513f84a9f2f807766a68a115404dd6493`，对应包内 `source/report.tar`。
- 标签 `research-evidence-834-admission-distant` 固定于上述报告快照，其祖先包含候选和撤回；
  后续 PR Rebase 或分支清理不会成为这些原始提交的唯一保留条件。

## 已执行的下载恢复核验

2026-10-08 已从 GitHub 发布资产实际下载至全新目录，先核对压缩包大小/SHA256，
再拒绝越界路径及链接条目并解压。`delivery-files-manifest.json` 的 131 个文件
全部通过大小/SHA256 校验，含 index/manifest 共 133 文件。
另核对索引绑定、A/B 源码包、三份二进制、全部输入、计划、诊断补丁/模块与报告快照。
[恢复核验记录](https://github.com/illusion-tech/laneflow/releases/download/research-evidence-834-admission-distant/restore-834-verification.json)
作为封存后独立收据交付，不加入或改变原压缩包。

包内顶层目录为 `admission-distant-834-20261008`。`measurement-freeze.json` 绑定运行身份，
`delivery-index.json` 绑定报告快照及文件清单，`delivery-files-manifest.json` 逐项给出字节数和 SHA256。
index/manifest 不自我哈希，由压缩包 SHA256 共同认证；不能只凭成功解压认定完整。

本索引在封存后独立提交，不在包内旧报告快照中。它不表示候选被采用，
不替代普通 ABBA 的停止结论，也不关闭 #707 正式认证义务。
