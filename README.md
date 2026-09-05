# Codex Navigator

快速找回你的 Codex 历史问题与回复。**同一个程序，支持终端版和网页版，安装一次即可任选：**

```bash
codex-nav                 # 终端版：直接在终端里使用，无需浏览器
codex-nav --web --port 0   # 网页版：在本机浏览器里阅读
```

安装并配置好 PATH 后，**在任意目录输入上述命令即可，不需要进入 Codex Navigator 源码文件夹**。

- **找会话**：默认只列主会话，按项目、标题或 ID 搜索。
- **读过程**：浏览问题、工具活动和最终回复，支持复制。
- **跟进进度**：实时更新，翻阅历史时不打断阅读位置。

不修改 Codex、不上传会话，不需要额外账号或 API Key。

## 让你的 Agent 帮你安装

把[本项目仓库链接](https://github.com/MIINGYANG/Codex-Navigator)（或已有源码目录）和下面这句话交给你的编程 Agent：

> 请阅读这个项目的 README 和 docs/agent-setup.md，帮我安装 Codex Navigator；我授权你备份并最小修改当前用户的 shell 启动配置，持久加入正确安装路径，验证新终端在任意目录直接运行 codex-nav 无需手动 source，再让我选择终端版或网页版并交付使用方式；不要修改或上传 Codex 会话，需要管理员权限、覆盖已有安装或其他系统变更时先询问我。

你不需要提前了解 Rust；Agent 会检查已有工具链，完成安装和持久 PATH 配置，并验证新终端可用。上面的指令已授权必要的用户级 shell 配置修改，不需要每次手动 `source`。请使用本仓库源码，不要使用来源不明的同名安装包。

## 自己安装

目前已验证 Linux x86_64；macOS / Windows 尚未完成实机验证。源码构建需要 Rust 1.88+ 和 C 链接器，建议使用当前 stable Rust。

仅首次从源码安装时，需要在项目目录运行。先确认 Rust 与 Cargo 均为 1.88 或更新版本，再安装；命令不存在或版本过旧时先看下方“安装常见问题”：

```bash
rustc --version
cargo --version
cargo install --path . --locked
```

这会安装一个独立的 `codex-nav` 命令，而不只是生成项目里的构建文件。以后打开任意终端、在任意目录启动即可，无需 `cargo run`，也无需回到源码目录。

若提示找不到命令，Linux 默认安装的 Bash/Zsh 用户可先在当前终端运行：

```bash
export PATH="$HOME/.cargo/bin:$PATH"
codex-nav --version
```

要让新终端自动生效，默认 rustup 安装且 `~/.cargo/env` 存在时，Bash 用户在 `~/.bashrc` 中添加一次以下内容（Zsh 使用实际的 `.zshrc`，默认在 `~/.zshrc`）：

```bash
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
```

保存后新开终端运行 `codex-nav --version`，无需再次手动 `source`。已有终端不会自动更新，可重新打开。其他 shell 或自定义安装目录应配置其实际 PATH；使用上面的 Agent 指令时，这些步骤由 Agent 完成。

网页版会尝试自动打开浏览器，`--port 0` 自动选择空闲端口。安装时需要下载依赖，运行时不需要外网，也不需要 Node.js。

## 怎么用

1. 在你平时使用 Codex 的机器上启动 Navigator；终端版在另一个交互式终端运行，不占用 Codex 的终端。
2. 选择主会话：终端版用 `j/k` 或方向键选择、`Enter` 打开；网页版直接点击。
3. 按 `/` 搜索问题，按 `f` 定位最终回复；网页版也可点击“最终回复”。

常用快捷键：`/` 搜索、`j/k` 移动、`g/G` 当前区域首尾、`f` 最终回复、`s` 返回会话列表、`?` 帮助。终端版用 `Tab` 切换时间线/正文焦点；网页版点击目录或正文切换操作区域。

```bash
codex-nav --all                        # 终端查看更早的主会话
codex-nav --web --port 0 --all          # 网页查看更早的主会话
codex-nav --web --port 0 --no-open      # 不自动打开浏览器，打印访问地址
codex-nav doctor                       # 环境诊断
```

终端版按 `q` 退出（搜索时先按 `Esc`），也可按 `Ctrl+C`。网页版请保留启动它的终端，按 `Ctrl+C` 停止；关闭网页不会停止服务。如果由 Agent 后台启动，让它告诉你如何停止该进程。两种模式退出都不影响 Codex。

## 安装常见问题

**提示 `cargo: command not found`，或 `lock file version 4 requires…`？**

前者可能是尚未安装工具链，也可能只是 PATH 未配置；后者表示当前调用的 Cargo 太旧。不要直接照系统提示安装 `apt` 里的旧版本：例如 Cargo 1.75 不满足本项目要求。

如果已通过 rustup 安装工具链，且 `~/.cargo/env` 存在，Bash/Zsh 用户先执行：

```bash
source "$HOME/.cargo/env"
command -v cargo
cargo --version
rustc --version
```

确认两者均为 1.88+ 后，再在源码目录执行 `cargo install --path . --locked`。如果仍过旧，需要通过 rustup 升级工具链；如果 env 文件不存在，先检查是否使用了自定义安装目录，确实未安装时再通过 rustup 安装，也可交给 Agent 处理。

不要删除或改写 `Cargo.lock`，也不要加 `-Znext-lockfile-bump` 绕过错误。已有系统 Rust 不必因此卸载；关键是当前终端选中了符合要求的工具链。若仅当前终端生效，请按上方 PATH 说明配置新终端。

## 使用前知道这些

- **读取你自己的数据**：默认读取 `~/.codex`，也支持 `CODEX_HOME`。没有会话时显示空列表；找不到旧记录可用 `--all`。
- **网页版只在本机访问**：使用程序打印的完整地址，其中带有临时令牌，不要分享。不能直接从其他电脑或手机连接；终端版不启动网页服务。
- **不判断答案正确性**：正常结束与活动错误分开显示，最终结果由你阅读回复判断。
- **长历史可能省略**：为控制内存，部分旧正文会显示省略提示，原文件不变；没有可靠标记时不会猜测最终回复。Markdown 为安全子集，不加载图片附件。

## 更多

[Agent 安装与自检](docs/agent-setup.md) · [版本记录](CHANGELOG.md)

[MIT License](LICENSE)。独立社区工具，与 OpenAI 官方项目无隶属关系。
