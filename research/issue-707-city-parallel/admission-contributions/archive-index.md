# 准入贡献融合证据归档

关联：#836；[完整结果](../admission-contributions-results.md)。

- [研究发布](https://github.com/illusion-tech/laneflow/releases/tag/research-evidence-836-admission-contributions)。
- [压缩归档](https://github.com/illusion-tech/laneflow/releases/download/research-evidence-836-admission-contributions/admission-contributions-836-012dd8f6.tar.zst)：173196707 字节；SHA256 `10050612f6f471284dc7397cc8d1e5b251e9b3a0ec887f4cd4f33746deb6b95f`。
- [外部封套](https://github.com/illusion-tech/laneflow/releases/download/research-evidence-836-admission-contributions/archive-836.json)绑定压缩包大小、SHA256 与内部索引。
- [实际下载恢复回执](https://github.com/illusion-tech/laneflow/releases/download/research-evidence-836-admission-contributions/restore-verification.json)记录全文件核验结果。
- 归档根：`local-performance-lab-20261008`；清单 211 文件，另含 index/manifest 两个文件。
- 归档报告原始字节（CRLF）SHA256：`fb6c99a7f80f964f0f0fe8526974b6f6f47e2a3dd6a8b31eac0038c9da8b08de`。
- 仓库报告规范 LF 字节 SHA256：`b80ec1bf5887cef344396d0842c38e2c537ea461cbe79efc5d8608f2cd26e6b6`；仅换行表示不同，正文逐行一致，避免提交与归档哈希循环依赖。
- 参考源码：`2d92889b84022c65fddd1e1f440ac56d80eed6b0`；候选测量源码：`012dd8f60fb9be374075d51184d76f45678f80ec`。

普通 A/B、两份诊断和探索构建各有源码 tar、二进制、commit 与 SHA256；确认轮单独
预登记。源码 Git bundle 保留本地实验的真实提交，基于公开参考祖先可恢复原身份。
最终 PR 的产品文件与候选冻结 blob 相同，最终提交还包含设计说明、报告和本索引。

归档保留输入、需求、原始结果/逐拍样本、CPU/内存记录、审计、准备阶段日志、
分析及恢复工具。`REPRODUCE.md` 说明干净 checkout、构建、运行和路径重定位。
外部研究脚本不作为仓库新增非 Rust 实现。所有确认轮和后台活动均保留。

压缩包只读；后续验收回执、PR 机器检查与发布安全快照作为发布侧或独立证据追加，
不回写封存包。实际发布与下载恢复状态以研究发布的资产及回执为准。
