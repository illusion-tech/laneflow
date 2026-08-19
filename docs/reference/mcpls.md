# mcpls 可选开发工具

**文档状态**: Active<br>
**适用范围**: 在 Windows Codex Desktop 中对 LaneFlow Rust workspace 做只读语义导航

## 1. 定位与边界

LaneFlow 把 mcpls 作为可选的本地开发工具，用于通过 `rust-analyzer` 提供 hover、
定义跳转、引用查找、文档符号、工作区符号搜索和诊断。它不属于 LaneFlow 运行时、
Cargo workspace 依赖、CI 门禁或发布产物，也不替代 `cargo check`、`cargo test`、`rg`
和源码阅读。

Windows 正式拓扑为“每个 Git worktree 一个原生 Streamable HTTP mcpls”：

- 同一 worktree 的多个 Codex 任务和 MCP session 复用一个 mcpls 进程及其
  `rust-analyzer`；
- 不同 worktree 使用不同 worktree ID、进程、loopback 端口、状态目录和
  `rust-analyzer`，不共享每进程全局的 LSP 状态；
- mcpls 只监听 `127.0.0.1`，不绑定局域网或公网地址，也不提供远程服务；
- mcpls 以对应 worktree 为工作目录，并显式读取该 worktree 的 `mcpls.toml`；其中
  `workspace.roots = []` 继续从当前工作目录发现 workspace。

仓库跟踪 `.codex/config.template.toml`，不跟踪动态 endpoint。Windows setup 在每个
worktree 本地生成被 `.gitignore` 忽略的 `.codex/config.toml`。生成文件包含管理标记、
schema version 和模板 SHA-256；模板变化会在下次 setup 时可见地重新生成。脚本拒绝
覆盖没有管理标记的既有配置，也不使用 `skip-worktree`、`assume-unchanged` 或其他隐藏
Git 变更的机制。

## 2. 每台 Windows 电脑的一次性安装

固定安装 mcpls `0.3.9`，并显式启用非默认的 `transport-http` feature：

```powershell
cargo install mcpls --version 0.3.9 --locked --features transport-http --force
```

确保 `rust-analyzer` 可通过 PATH 启动；使用 rustup 时可执行：

```powershell
rustup component add rust-analyzer
```

验证版本和 HTTP 参数：

```powershell
mcpls --version
mcpls --help
rust-analyzer --version
```

`mcpls --version` 必须返回 `0.3.9`，帮助中必须同时出现 `--listen` 和 `--http-path`。
setup 也会执行相同检查，避免把没有 HTTP feature 的同版本二进制误判为可用。

setup 脚本不会联网、安装、升级或下载任何工具。mcpls 缺失、版本不符或没有 HTTP
feature 时，`Ensure` 会给出警告、生成禁用配置并成功结束；LaneFlow 的其他开发工作
不受影响。人工 `Start` 则以非零状态严格失败。

当前固定工具的来源为 crates.io，上游为 `bug-ops/mcpls`，许可证表达式为
`MIT OR Apache-2.0`。该本机工具不进入仓库依赖图或分发物；如果未来需要自动下载、
进入 CI 或随产品分发，必须另建治理任务并按 `docs/governance/dependency-security.md`
重新审计来源、校验和、许可证、安全公告和分发影响。

## 3. Codex Desktop Local Environment setup

在 Codex Desktop 的 Windows Local Environment setup 中配置以下命令：

```powershell
pwsh -NoLogo -NoProfile -File .codex/setup-mcpls-worktree.ps1 -Action Ensure
```

Codex 为新任务创建 worktree 时会运行 setup。`Ensure` 会：

1. 规范化 `git rev-parse --show-toplevel` 结果，保留文件系统返回的真实大小写，并派生
   SHA-256 worktree ID；
2. 按持久游标轮转、有界检查历史状态；自动清理不会因为单次 HTTP 探测失败而停止有效
   worktree 的已归属服务；
3. 在 `%LOCALAPPDATA%\LaneFlow\mcpls\worktrees\.locks\` 下，以禁止共享打开的锁文件
   实现跨 Windows 会话的同 worktree 串行化和全局端口分配串行化；
4. 从 `41000..48999` 按 worktree ID 确定性选择 loopback 端口并线性探测冲突；
5. 同时验证 PID、进程启动时间、可执行路径、命令行中的 `mcpls.toml`/endpoint、
   `mcpls.toml` 内容 SHA-256 以及 HTTP MCP `initialize`；配置内容变化会停止已归属的旧
   服务并启动新服务，不复用旧 `rust-analyzer`；
6. 用 `StartupTimeoutSeconds` 的单一截止时间约束二进制校验、锁等待、端口绑定和 HTTP
   健康检查；
7. 只有状态、健康检查、启用配置与生命周期日志全部提交成功后才保留新进程；任一记账
   步骤失败都会回收该进程并保持配置禁用。

Codex 官方文档没有保证 Local Environment setup 一定早于当前任务的 MCP 配置读取。
因此首次生成配置后，如果当前任务没有加载 mcpls，应 Restart Codex 或新建任务；不要
把当前任务热重载当作保证。修改模板后也采用相同处理；修改 `mcpls.toml` 后，下次
`Ensure`/`Start` 会重启 worktree 服务，但当前任务是否重新读取生成配置仍遵循上述边界。

Local Environment 的 UI 设置属于用户本机配置，不是仓库单一事实源。仓库只维护脚本、
模板和本文档。只应对自己已经审阅并信任的 checkout 启用项目配置和 setup；
`mcpls.toml` 能决定脚本显式启动的语言服务器命令，不能把不受信任分支当作安全输入。

## 4. 生命周期命令

所有命令都在目标 LaneFlow worktree 根目录执行：

```powershell
# setup 使用：失败时禁用 mcpls，但不阻断任务创建
pwsh -NoLogo -NoProfile -File .codex/setup-mcpls-worktree.ps1 -Action Ensure

# 人工严格启动或复用：失败时返回非零
pwsh -NoLogo -NoProfile -File .codex/setup-mcpls-worktree.ps1 -Action Start

# 输出 worktree、endpoint、PID 身份、健康和 rust-analyzer 后代数量
pwsh -NoLogo -NoProfile -File .codex/setup-mcpls-worktree.ps1 -Action Status

# 仅在完整身份和 MCP 健康均匹配时停止该 worktree 服务及其后代
pwsh -NoLogo -NoProfile -File .codex/setup-mcpls-worktree.ps1 -Action Stop

# 清理 root 已不存在、已不是有效 Git worktree 或服务已失效的状态
pwsh -NoLogo -NoProfile -File .codex/setup-mcpls-worktree.ps1 -Action Prune
```

`Ensure` 和 `Start` 在同一 worktree 中串行或并发调用都必须复用同一个健康 PID 和
endpoint；锁文件采用 `FileShare.None`，因此不同桌面会话或终端也进入同一临界区。
`Stop` 不按进程名批量停止；PID、启动时间、可执行路径、命令行或 HTTP 健康任一不满足
时都会拒绝普通停止。`Prune` 先校验“状态目录名 = `worktree_id` = 规范化 root 哈希”，
再判断进程身份；三者不一致、状态损坏或 schema 不支持时都拒绝停止和删除并保留证据。
人工 `Prune` 对有效 worktree 的失效服务做两次 HTTP 探测后才可停止；`Ensure` 内部的
自动清理不探测、不停止有效 worktree 的已归属存活服务。只有结构化身份仍匹配，或记录
PID 已不存在且状态所有权一致时，才清理失效状态。

本地状态位于：

```text
%LOCALAPPDATA%\LaneFlow\mcpls\worktrees\<worktree_id>\
```

`state.json` 当前为 schema 2，记录规范化 root、PID、启动时间、可执行路径、命令摘要、
端口、endpoint、模板哈希、`mcpls.toml` 内容哈希和状态；`lifecycle.log` 记录本脚本的
启动、复用、失败、停止与清理事件。两者都不提交到仓库。状态、轮转游标和生成配置都
通过同目录临时文件原子替换。旧 schema 或不完整状态会被明确报告为 `invalid-state`，
不会被当作“无状态”而启动第二个服务；需在核对对应 PID 后人工处理该状态目录。

## 5. 验证

先运行不依赖 mcpls/Pester 的脚本契约测试：

```powershell
pwsh -NoLogo -NoProfile -File .codex/test-setup-mcpls-worktree.ps1
```

setup 完成并 Restart 或新建任务后，检查 Codex 解析结果：

```powershell
codex mcp get mcpls
```

应确认：

- transport 为 HTTP，URL 与 `Status` 的 loopback endpoint 一致；
- `required` 为 `false`；
- `enabled_tools` 只包含 `get_hover`、`get_definition`、`get_references`、
  `get_document_symbols`、`workspace_symbol_search` 和 `get_diagnostics`；
- rename、format 等写能力没有启用。

`Status` 中 `identity_matched` 和 `healthy` 应同时为 `true`。首次 Rust 语义查询需要等待
`rust-analyzer` 完成 workspace 初始化；冷启动期间的空结果不能解释为没有定义。可让
Codex 查询当前 worktree 中已知的 `CoreWorld` 定义和引用，并核对返回路径确实属于当前
worktree。至少两个 worktree 并行验收时，还应确认它们的 worktree ID、endpoint、PID、
`rust-analyzer` 后代和语义查询路径彼此不同。

mcpls 返回的源码行号按一基编号（1-based）解释，不再额外加一。职责分工保持为：

- mcpls：类型感知的 hover、定义、活动配置引用、符号和诊断；
- `rg`：字面量、配置、注释、未启用 feature 和快速文本搜索；
- `cargo check`、`cargo test`：编译、测试与最终正确性依据。

## 6. 迁移、回退与跨平台边界

旧原型或旧 checkout 如果对 `.codex/config.toml` 设置过 `skip-worktree`，先显式恢复
Git 可见性：

```powershell
git update-index --no-skip-worktree -- .codex/config.toml
git ls-files -v -- .codex/config.toml
```

随后审阅并移动或删除旧的本地 STDIO/原型配置，再运行 `Ensure`。脚本不会替用户覆盖
没有管理标记的配置。不要重新设置 `skip-worktree`；共享变化应通过
`.codex/config.template.toml` 的正常 Git diff/review 进入每个 worktree。

需要临时停用 HTTP 服务时运行 `Stop`；生成配置会保留但标记为 `enabled = false`。
mcpls 完全不可用时，继续使用 `rg`、源码阅读和 Cargo 检查。卸载本机工具前应先逐个
worktree 执行 `Stop` 或在审阅 `Status` 后执行 `Prune`，再运行：

```powershell
cargo uninstall mcpls
```

本 Issue 的受管 HTTP 生命周期首轮只支持 Windows PowerShell 7。Linux/macOS 上 mcpls
仍为可选工具：可以不启用，或在被忽略的本地 `.codex/config.toml` 中手工配置 STDIO；
后者仍是每任务进程，不具备本文的 worktree 级复用保证。跨平台 HTTP 生命周期需要
独立后续任务，不应把 Windows 脚本描述为已经跨平台。

参考：

- [OpenAI Docs：Codex Config basics](https://learn.chatgpt.com/docs/config-file/config-basic)
- [OpenAI Docs：Codex MCP](https://learn.chatgpt.com/docs/extend/mcp?surface=cli)
- [OpenAI Docs：Codex Local environments](https://learn.chatgpt.com/docs/environments/local-environment)
- [mcpls 0.3.9 文档与元数据](https://docs.rs/crate/mcpls/0.3.9)
- [mcpls source](https://github.com/bug-ops/mcpls)
