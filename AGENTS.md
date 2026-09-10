# Codex Navigator 开发约定

- 本项目是独立 Sidecar；浏览、解析与终端保持只读，不修改 Codex 源码、不包装 Codex PTY。用户明确授权的 Web 会话管理为例外：重命名仅通过官方 app-server 元数据接口，删除仅移到系统回收站并在 Web 再次确认；不发送模型 turn，不永久删除。开发验收只使用隔离合成数据，不操作真实会话。
- 修改前阅读相关源码、tasks/todo.md、tasks/lessons.md 和 docs/learning.md；保持最小改动。
- 导航按当前焦点分发；宽窄布局切换不改变快捷键语义。解析器和状态行为必须有自动测试。
- 用户授权的开发迭代按独立本地 commit 保存，提交说明写清具体变化。不要混合无关需求，不自动 push。
- 版本迭代同步 Cargo.toml、Cargo.lock、CHANGELOG.md；修复使用 patch 版本，新功能使用 minor 版本，不兼容变更使用 major 版本。
- 发布前执行 fmt、clippy、全量测试、release build；涉及终端交互时执行 scripts/terminal_qa.py。验收通过后创建对应版本标签，不移动或覆盖既有发布标签。
- 在 tasks/todo.md 记录本次计划和验收结果；用户纠正后更新 tasks/lessons.md，验证完成后向 docs/learning.md 追加知识记录。
- 说明、日志与经验使用中文；代码与路径使用英文。
