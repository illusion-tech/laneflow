# 研究源码与结论

`research/` 保存研究源码、实验计划、结论报告和复现输入。目录中的实验与
诊断结论沿用各报告的观察边界，不等同于产品性能或交通质量认证。

有用的是研究结论，不是研究过程。长期引用的结论说明研究问题、关键结果、主要
条件、已知限制及关联源码或 Issue / PR 入口即可；结论引用的聚合数字（中位数、
范围、计数）属于结论本身。结论报告与复现输入（输入声明、协议与环境说明、少量
代表性样本）可以入仓，单个生成文件不超过 1,000 行、单个研究目录的生成文件合计
不超过 5,000 行，超出即按过程数据处理。完整规则以
[文档边界政策][documentation-policy-research]为准。

逐区间或逐样本原始测量、搜索轨迹、全量结果集、重跑中间态、环境转储、二进制和
采集原包是过程数据，一律不进入源码树：本地生成写到已忽略的 `target/` 或仓库外
目录。不以「证据保存」「完整回归证据」名义提交过程数据；Agent 不自发生产清单、
manifest、逐字节一致性声明或 SHA-256 登记等归档物，任务明确要求除外；不需要的
输出不必生成或保存。

清理旧证据时，可明确退役并修复引用与依赖入口，无须先制作外部归档；若声称迁移后
仍可恢复，则按承诺范围验证下载和恢复。Git 历史只保存实际提交过的材料，不代表
外部原包也有备份；不再保留完整证据时应说明恢复边界。

生产契约、规范输入和现有测试 fixture（固定测试环境）按实际用途保留。比如
`issue-679-route-query/evidence/` 是当前 Runtime 完整性测试与研究编译使用的 fixture，
仍随源码交付。

研究 crate 的 workspace 归属（workspace 成员或独立 `[workspace]`）由立项 Issue 决定，
并在各目录 `README.md` 声明理由；两种形态都必须 `publish = false`、声明非生产身份，
且第三方依赖必须纳入许可证与 RustSec 审计。完整规则见
[许可证与依赖安全基线][dependency-security] 第 8 节。

## 本机路径占位

研究文档、索引、脚本和提交的工具输出不写采集机的本机布局路径；系统工具链的标准
安装位置（如 `Program Files` 下的 MSVC 与 Windows Kits）不在此列。以下占位只表示位置类别，
不是可访问的入口：`<evidence-root>` 为仓库外证据目录，`<checkout>` 为主 checkout，
`<worktree>` 为某个工作树根，`<cargo-home>` 为 Cargo 主目录。研究来源使用关联源码
或 Issue / PR 入口说明；选择制品交付时提供可访问地址，按任务需要补充冻结身份或
SHA-256。

2026-10-10 起已提交的工具日志、构建 JSONL 与 WPR 统计只把上述路径前缀替换为占位，
其余字节不变；替换前原件可在 `7e4fc6ea` 及更早提交读取。

## 历史归档

以下保留既有迁移事实和恢复入口，不作为后续研究的默认归档要求。

- [2026-10-01 历史诊断 JSON 迁移][2026-10-01-json-migration]：
  冻结主线中 22 个研究目录的 313 份 JSON、完整源码及主线可达 Git 历史，保留原始
  字节与旧提交 SHA；清理当前源码树。逐文件清单存放在归档内。
- [2026-10-12 已关闭研究 crate 代码退役][2026-10-12-research-crate-retirement]：
  退役 #123/#543/#681/#682/#711/#712 六个已关闭研究目录的 Cargo manifest、
  lockfile、源码与构建脚本共 28 个文件；#123/#682 同时退出主 workspace。各目录
  保留 README 结论、结果表与既有证据；退役文件逐字节冻结在归档制品中。
- [2026-10-13 #285 复杂路口有界性能程序退役][2026-10-13-junction-scale-retirement]：
  退役 `laneflow-bevy` 中 4 个 `junction_scale*` example、4 个私有支持模块与
  `issue-285-junction-scale` 的 4 个脚本共 12 个文件；README 保留协议与结论，
  退役文件按冻结提交的固定原始文件地址下载并以 SHA-256 复核，不另发 Release。

历史报告里的 JSON 链接指向冻结提交；旧命令如果依赖已迁移文件，应在归档源码
目录运行。各报告另外引用的采集原包、二进制或输入制品仍须按对应身份单独取得，
不能将这次存量迁移当作那些外部原包已经重新验证。

#711/#712 的旧分析与拒绝测试入口已退役，原件及原始证据在同一归档中。当前保留
简短结论、冻结身份、下载地址和摘要，不安排例行复核；相关回归或结论争议由调查者
按需恢复归档。生产回归 fixture 和其他仍使用的研究源码保留。

[documentation-policy-research]: ../docs/governance/documentation-policy.md#研究结论与过程数据边界
[dependency-security]: ../docs/governance/dependency-security.md
[2026-10-01-json-migration]: archives/2026-10-01-json-migration.md
[2026-10-12-research-crate-retirement]: archives/2026-10-12-research-crate-retirement.md
[2026-10-13-junction-scale-retirement]: archives/2026-10-13-junction-scale-retirement.md
