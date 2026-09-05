# Codex Navigator

Codex CLI 的本地只读终端 Sidecar。快速定位历史 Prompt，浏览回复与命令，并实时跟随正在增长的会话。独立社区工具，与 OpenAI 官方项目无隶属关系。

```text
 Codex Navigator · WATCHING · MAIN · ~/project · 12 turns
┌ TIMELINE ─────────────────┬ TURN 12 · incomplete ──────────────────┐
│  10 ✓ 分析项目结构        │ USER                                    │
│  11 ✓ 检查配置            │ 帮我跑一下测试                          │
│▸ 12 … 帮我跑一下测试      │                                         │
│                          │ ACTIVITY · command                      │
│                          │ $ cargo test                            │
│                          │ OUTPUT                                  │
│                          │ exit code 0                             │
└──────────────────────────┴─────────────────────────────────────────┘
 / search  [/] turn  G latest  Tab focus  c copy  s sessions  ? help  q quit
```

## 功能

- 启动先进入可搜索的主会话 Session Picker，当前目录相关会话优先，确认后打开。
- 按 Turn 浏览完整 Prompt、公开回复、工具调用、文件活动和可靠失败信息。
- 中文、大小写无关 substring 与英文 fuzzy 搜索；同分时优先较新的 Turn。
- 后台流式加载，实时增量更新；查看历史时保留选择，显示 `+N new turns`。
- 宽终端双栏；不足 100 列时按 Tab 切换单栏。
- 支持复制、Viewer 滚动、帮助、手动刷新和诊断。
- 容忍格式变动、坏行、BOM、半行、超大图片记录和文件替换。

## 安装

需要 Rust 1.88 或更新版本、平台 C 链接器。首次构建需要获取 Cargo 依赖；运行程序无需网络。

```bash
cargo build --release --locked
./target/release/codex-nav --version

# 可选：安装到用户 Cargo bin
cargo install --path . --locked
```

若 cargo 未加入 PATH，先运行 `. "$HOME/.cargo/env"`。可执行文件为 Linux/macOS 的 `target/release/codex-nav` 或 Windows 的 `target/release/codex-nav.exe`。

## 快速开始

终端 A 正常使用 Codex：

```bash
cd /path/to/project
codex
```

终端 B 打开 Navigator：

```bash
cd /path/to/project
codex-nav
```

启动后先选择主会话：`j/k` 或方向键移动，Enter 打开，即使只有一个候选也不会自动进入。`/` 搜索会话；打开后 `/` 搜索 Prompt。在正文中按 `g/G` 跳到顶部/底部；按 Tab 切回时间线后，`G` 返回最新有效 Turn。`q` 或 `Ctrl+C` 退出。Navigator 不启动、不接管 Codex；退出不会影响另一个终端。

## 快捷键

| 按键 | 行为 |
|---|---|
| `j/k`、`↓/↑` | Timeline 选择；Viewer 逐行滚动；Picker 选择 |
| `[`、`]` | 任意主界面面板中选择上一/下一 Turn |
| `g`、`G` | 时间线：第一轮 / 最新未 rollback 的轮；正文：当前轮顶部 / 底部 |
| `f` | 定位当前轮最后一条明确标记的最终回复；宽窄切换保持位置 |
| `Enter`、`Tab` | 打开/聚焦 Viewer；切换面板 |
| `/` | 搜索 Prompt；Picker 中搜索标题、cwd、ID、身份、父会话、代理名 |
| `Esc` | 取消搜索并恢复选择；回到 Timeline；Picker 中退出 |
| `PgUp/PgDn`、`Home/End` | Viewer 翻页 / 顶部 / 底部 |
| `c`、`C` | 复制 Prompt / 该 Turn 当前保留的全部可见文本 |
| `r`、`s` | 增量刷新 / 返回 Session Picker |
| `?` | 帮助 |
| `q`、`Ctrl+C` | 安全退出 |

搜索模式下 `j/k` 是输入字符，使用方向键移动结果。没有剪贴板服务或在 SSH/headless 环境中复制失败，会显示短暂提示。复制不使用 OSC 52，不自动将内容写入终端宿主剪贴板。

宽窄终端切换只改变布局；`j/k` 和 `g/G` 始终操作当前聚焦栏。正文中要切换到最新轮，先按 Tab 聚焦时间线，再按 G。

## 会话发现

优先使用 `CODEX_HOME`；未设置则使用用户目录下 `.codex`。读取 `sessions/YYYY/MM/DD/rollout-*.jsonl`，默认扫描最近 7 天。启动、`s` 返回及 `r` 刷新均进入主会话 Picker，不再自动打开。列表只显示明确识别为 MAIN 的会话；SUBAGENT 和 UNKNOWN 均隐藏。当前 cwd 精确匹配优先，父/子目录其次，同级别按更新时间排序；无匹配时展示近期主会话。`--all` 扩展到全部日期，仍只列主会话。

Picker 显示主会话标题、目录、短 ID、轮数和更新时间。需要检查非主会话时，可显式使用 `--session ID/PATH` 打开其独立日志；不会重定向父会话。缺失或未知来源不猜测身份；单独 fork 不等于子代理。会话头部仍显示真实身份。`WATCHING` 仅表示 Navigator 正在监控文件，不表示 Codex 正在执行任务；`STATIC` 表示未启用监控。

按 `f` 定位当前轮的 `FINAL ANSWER`，不会切换到最新轮或清除历史新增提示。最终回复只依据 `phase=final_answer` 或 completion 的 `last_agent_message`；缺少标记或正文已淘汰时会提示不可定位，不把最后一条进度消息当最终答案。

`session_index.jsonl` 只补充标题和时间，索引缺失或过期不影响从 rollout 恢复。发现阶段每个文件最多读取 1 MiB，Picker 的大文件轮数先显示 `?`，随后后台补齐；可以立即选择打开，无需等候统计。近期目录为空时会有限回退到最近有数据的日期目录。

## CLI

```bash
codex-nav
codex-nav --session SESSION_ID_OR_UNIQUE_PREFIX
codex-nav --session /path/to/rollout.jsonl
codex-nav --cwd /path/to/project
codex-nav --all
codex-nav --no-watch
codex-nav doctor
codex-nav --help
codex-nav --version
```

`--no-watch` 禁用自动更新，`r` 仍可增量读取。TUI 需要交互终端；管道环境请使用 `doctor` 或 `--help`。

可选配置路径：Linux `~/.config/codex-nav/config.toml`（尊重 XDG_CONFIG_HOME）；macOS `~/Library/Application Support/codex-nav/config.toml`；Windows 平台配置目录下 `codex-nav/config.toml`。应用不会自动创建配置。

```toml
preview_width = 36
watch = true
recent_days = 7
max_record_bytes = 4194304
```

## 隐私与只读保证

Navigator 只读本机 Codex session，不上传数据，不要求账号或 OpenAI API key，不包含 AI API、云服务或 telemetry。不会修改 Codex 源码、配置、rollout 或 session_index，不支持 resume、fork、回滚、删除或发送 Prompt。配置也只读。

所有显示内容都会过滤 ANSI/OSC/control 字符。解析器不展示 reasoning、隐藏推理、加密内容或压缩上下文。诊断只打印环境可用性及路径，不打印认证信息、完整环境或原始会话。

会话本身可能包含敏感代码、命令和路径；不要随意分享 raw rollout。按 `c/C` 是向系统剪贴板复制本地文本的明确操作。

## 排错与兼容性

先运行 `codex-nav doctor`。找不到会话时确认 CODEX_HOME 与 Codex 一致，尝试 `--all` 或直接指定文件。不可读文件会提示，不会尝试修复权限或改写数据。

当前已按本机 Codex 0.153.2 真实格式验证，并用合成测试覆盖旧 `event_msg.user_message`、新 `item_completed`、显式边界与无边界格式。详细记录见 [session-format-notes](docs/session-format-notes.md)。rollout 不是稳定 API，未知 record 会跳过。

轮次生命周期与过程警告分开显示，均不判断答案是否正确：

| 标记 | 含义 |
|---|---|
| `✓` | 收到正常完成事件，不代表任务结果正确 |
| `✕` | 收到明确的轮次执行错误，不是某个工具失败 |
| `⊘` | 收到轮次中断事件 |
| `…` | 尚未记录结束事件，不保证模型仍在运行 |
| `?` | 状态未知 |
| `↶` | 已 rollback |
| `!N` | 独立的活动错误记录数，与上述生命周期并列 |

例如 `✓ !1` 表示该轮正常结束，过程中检测到过一次活动错误；失败后重试成功也保留警告。晚到的工具错误不会把已完成轮改成失败。错误数是已检测的活动记录数，不代表未解决问题数。退出码 0 不推断“所有测试通过”，最终结果由用户按 `f` 查看最终回复后判断。

超大图片记录默认超过 4 MiB 即流式跳过。每轮 Prompt 最多 256 KiB，独立正文预算 16 MiB；工具输出不会挤占 Prompt。活动正文预算 48 MiB，单条回复/工具正文最多 64 KiB（另有少量截断标记/字段，均计入活动预算）。预算用满时淘汰较早正文，保留新输入与新回复；旧 Prompt 保留预览供搜索/复制，正文明确提示省略，旧活动合并显示省略提示。搜索不会继续命中已淘汰正文。总正文预算 64 MiB 不等于进程 RSS，索引、预览、模型和后台更新另占内存。

仍限制 100,000 Turn / 200,000 活动；超过条数上限的后续内容不再进入模型。省略计数会显示。原文件不变，完整文本仍在 rollout；本版本不从磁盘按需恢复历史全文。只有换行结束的 JSONL 才提交，静态文件末尾无换行也会等待。

文件变化使用通知和 100 ms 轮询后备。替换/截断会安全重载；无法识别刻意保持身份、前缀、长度和时间戳的原地重写。退出 raw mode 使用恢复 guard，panic 也会恢复；像所有终端应用一样，SIGKILL 无法执行清理，可在 shell 运行 `reset`。

## 开发与验证

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build --release
```

所有测试使用合成 fixture / 临时目录，不修改真实 Codex 数据。Linux 终端集成验收：

```bash
python3 scripts/terminal_qa.py target/release/codex-nav
```

只读统计与性能工具（不输出 Prompt）：

```bash
cargo run --release --example audit -- /path/to/rollout.jsonl
cargo run --release --example benchmark
```

参见 [架构](docs/architecture.md)、[验收结果](docs/qa.md)、[变更记录](CHANGELOG.md)。本地 Linux x86_64 完成构建和终端验收；其他平台使用跨平台依赖，但未经本次机器验证。

## Web 查看

当前只支持终端查看，不包含 Web 页面或 HTTP 服务。核心解析、发现、监控和搜索可以复用于未来本机 Web 版，但仍需新增只读接口、事件推送及网页前端；详见 [Web 支持边界](docs/architecture.md#web-支持边界)。

## 本地开发时间线与版本

仓库使用 `main` 分支，每个完成验收的迭代保留独立 commit，并通过版本标签标记发布点。Navigator 的时间线用于浏览 Codex 对话；Git 用于查看代码变更及版本演进。

```bash
# 查看提交时间线和版本标签
git log --graph --decorate --oneline --all
git tag -n

# 对比本次修复与初始版本
git diff v1.0.0..v1.0.1 -- src/app.rs
```

后续迭代遵循 [开发约定](AGENTS.md)，同步版本号、CHANGELOG 和验收记录。当前仅管理本地仓库，不自动推送到远程。
