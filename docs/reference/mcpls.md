# mcpls 可选开发工具

**文档状态**: Active
**适用范围**: 使用 Codex 在 LaneFlow Rust workspace 中进行只读语义导航

## 1. 定位与边界

LaneFlow 把 mcpls 作为可选的本地开发工具，用于通过 `rust-analyzer` 提供 hover、
定义跳转、引用查找、文档符号、工作区符号搜索和诊断。它不属于 LaneFlow 运行时、
Cargo 依赖或 CI 门禁，也不替代 `cargo check/test`、`rg` 和源码阅读。

仓库只保存可移植配置，不保存 mcpls 或 `rust-analyzer` 二进制。每台开发电脑独立
安装工具，并显式信任本地 LaneFlow checkout 后，Codex 才加载项目级
`.codex/config.toml`。该配置又以 `--trust-project-config` 显式允许 mcpls 读取仓库根目录
的 `mcpls.toml`；该文件可以决定要启动的语言服务器，因此只应信任自己已审阅的
checkout。

项目配置不设置 `mcp_servers.mcpls.cwd`，由 Codex 使用当前任务或 CLI 会话的工作目录
启动 mcpls。`mcpls.toml` 同样保持 `workspace.roots = []`，让 mcpls 从进程工作目录发现
当前 LaneFlow workspace；两处都不得写入用户名、盘符或某个 worktree 的绝对路径。

当前验证版本为 mcpls `0.3.9`，其 crates.io 元数据声明最低 Rust 版本 `1.88`、
许可证 `MIT OR Apache-2.0`。仓库通过 PATH 解析 `mcpls` 与 `rust-analyzer`，不保存用户名、
盘符或其他机器绝对路径。

## 2. 每台电脑的一次性安装

安装固定版本的 mcpls：

```powershell
cargo install mcpls --version 0.3.9 --locked
```

确保 `rust-analyzer` 可通过 PATH 启动；使用 rustup 时可执行：

```powershell
rustup component add rust-analyzer
```

验证两个命令：

```powershell
mcpls --version
rust-analyzer --version
```

安装或修改 PATH 后，重新启动 Codex。打开 LaneFlow checkout 时显式将项目标记为
trusted；不受信任的项目不会加载仓库中的 `.codex/` 配置。

## 3. 验证与日常使用

在仓库根目录检查 Codex 解析后的 MCP 配置：

```powershell
codex mcp get mcpls
```

应确认：

- `command` 为 `mcpls`，而不是某台电脑的绝对路径；
- `cwd` 显示为 `-`，表示项目配置没有显式覆盖工作目录；
- `required` 为 `false`；
- `enabled_tools` 只包含六个只读工具，且这些工具无需逐次批准。

`cwd: -` 不表示 mcpls 进程没有工作目录，也不能单独证明它位于正确的 checkout。
实际目录应通过已知符号的语义查询返回路径验证；排查目录问题时，再检查 mcpls 与
`rust-analyzer` 进程的工作目录，并确认都指向当前 worktree。切换 worktree，或修改
`.codex/config.toml`、`mcpls.toml`、PATH 后，重新启动 Codex Desktop；CLI 验证则启动
新的 `codex` 进程，避免沿用已经启动的 MCP server。

首次 Rust 语义查询需要等待 `rust-analyzer` 完成 workspace 初始化。冷启动期间的
“仍在初始化”或空结果不能解释为“没有定义/引用”；应等待一个已知 workspace symbol
能够返回后再查询。mcpls 返回的源码行号按一基编号（1-based）解释，不再额外加一。

职责分工：

- mcpls：类型感知的 hover、定义、活动配置引用、符号和诊断；
- `rg`：字面量、配置、注释、未启用 feature 和快速文本搜索；
- `cargo check/test`：编译、测试与最终正确性依据。

## 4. 回退与卸载

mcpls 被配置为非必需 MCP server。某台电脑尚未安装或启动失败时，继续使用 `rg`、
源码阅读和 Cargo 检查；不要把 MCP 可用性当作 LaneFlow 开工或合并条件。

卸载本机工具：

```powershell
cargo uninstall mcpls
```

卸载后重新启动 Codex。仓库配置仍会保留给其他电脑使用；需要在单次 CLI 会话中禁用时，
可以使用配置覆盖：

```powershell
codex -c 'mcp_servers.mcpls.enabled=false'
```

## 5. 供应链边界

当前配置不下载或分发二进制，因此个人开发机安装不要求仓库维护 SHA-256。若未来把
mcpls 下载脚本引入 CI、安装包或发布流程，必须另建治理任务，固定来源、版本与校验和，
并按 `docs/governance/dependency-security.md` 重新审计许可证、安全公告和分发影响。

参考：

- [OpenAI Docs：Codex Config basics](https://learn.chatgpt.com/docs/config-file/config-basic)
- [OpenAI Docs：Codex MCP](https://learn.chatgpt.com/docs/extend/mcp)
- [mcpls 0.3.9 文档与元数据](https://docs.rs/crate/mcpls/0.3.9)
- [mcpls source](https://github.com/bug-ops/mcpls)
