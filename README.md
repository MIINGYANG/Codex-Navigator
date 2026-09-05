# Codex Navigator

在本地网页或终端中，快速找回你的 Codex 历史问题与回复。

- **找会话**：默认只列主会话，按项目、标题或 ID 搜索。
- **读过程**：浏览问题、工具活动和最终回复，支持复制。
- **跟进进度**：实时更新，翻阅历史时不打断阅读位置。

不修改 Codex、不上传会话，不需要额外账号或 API Key。

## 让你的 Agent 帮你安装

把**本项目的源码目录**（或发布后的仓库链接）和下面这句话交给你的编程 Agent：

> 请阅读这个项目的 README 和 docs/agent-setup.md，按指南在我的机器上安装 Codex Navigator、运行自检并启动本地 Web 界面，最后给我访问地址和停止方式；不要修改或上传我的 Codex 会话，遇到需要管理员权限或覆盖已有安装时先询问我。

你不需要提前了解 Rust；Agent 会检查依赖。当前仓库尚未配置公开下载地址，请先将源码交给 Agent，不要使用来源不明的同名安装包。

## 自己安装

目前已验证 Linux x86_64；macOS / Windows 尚未完成实机验证。源码构建需要 Rust 1.88+ 和 C 链接器，建议使用当前 stable Rust。

拿到源码后，在项目目录运行：

```bash
cargo install --path . --locked
codex-nav --web --port 0
```

浏览器会自动打开，`--port 0` 自动选择空闲端口。若提示找不到 `codex-nav`，确认 Cargo 的 bin 目录已加入 PATH（Linux 通常为 `~/.cargo/bin`）。安装时需要下载依赖，运行时不需要外网，也不需要 Node.js。

## 怎么用

1. 在你平时使用 Codex 的机器上启动 Navigator。
2. 选择主会话，搜索并点击想回看的问题。
3. 点击“最终回复”或按 `f` 定位结果，展开活动查看执行过程。

常用快捷键：`/` 搜索、`j/k` 移动、`g/G` 当前区域首尾、`f` 最终回复、`s` 返回会话列表、`?` 帮助。点击目录或正文后，导航键会操作对应区域。

```bash
codex-nav                              # 终端模式
codex-nav --web --port 0 --all          # 网页查看更早的主会话
codex-nav --web --port 0 --no-open      # 不自动打开浏览器，打印访问地址
codex-nav doctor                       # 环境诊断
```

启动 Web 后请保留启动它的终端，按 `Ctrl+C` 停止；关闭网页不会停止服务，停止 Navigator 不影响 Codex。如果由 Agent 后台启动，让它告诉你如何停止该进程。

## 使用前知道这些

- **读取你自己的数据**：默认读取 `~/.codex`，也支持 `CODEX_HOME`。没有会话时显示空列表；找不到旧记录可用 `--all`。
- **只在本机访问**：使用程序打印的完整地址，其中带有临时令牌，不要分享。不能直接从其他电脑或手机连接。
- **不判断答案正确性**：正常结束与活动错误分开显示，最终结果由你阅读回复判断。
- **长历史可能省略**：为控制内存，部分旧正文会显示省略提示，原文件不变；没有可靠标记时不会猜测最终回复。Markdown 为安全子集，不加载图片附件。

## 更多

[Agent 安装与自检](docs/agent-setup.md) · [详细使用指南](Codex-Navigator-User-Guide.md) · [验收记录](docs/qa.md) · [架构与限制](docs/architecture.md) · [版本记录](CHANGELOG.md)

[MIT License](LICENSE)。独立社区工具，与 OpenAI 官方项目无隶属关系。
