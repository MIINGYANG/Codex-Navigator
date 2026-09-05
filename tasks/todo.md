# Codex Navigator v1.0 实施计划

已完整阅读产品规格及使用指南。执行采用规格指定的 Rust 独立 TUI，不修改 Codex，不包装 Codex PTY，不写入 CODEX_HOME；无需阶段性确认。

- [x] Phase 0：检查工具链与真实会话结构，建立 crate 和兼容性记录，构建成功。
- [x] Phase 1：实现有界流式解析、领域模型、去重、状态与 rollback；通过 parser 自动测试。
- [x] Phase 2：实现近期发现、索引辅助、目录排序、配置和 doctor；通过 discovery 自动测试。
- [x] Phase 3：实现 Picker、Timeline、Viewer、搜索、响应式布局、键盘与安全终端恢复；通过状态及渲染测试。
- [x] Phase 4：实现后台增量读取、文件替换恢复、实时跟随与历史选择保护；通过增量测试。
- [x] Phase 5：验证大文件、Unicode、损坏记录、剪贴板降级及真实本机会话。
- [x] Phase 6：完成 README、架构文档、LICENSE、CHANGELOG 和发布构建。
- [x] Phase 7：运行 fmt、clippy、全部测试、release 及终端集成验收，记录结果。

## 设计约定

- 流式读取使用 File::open 和有界缓冲；后台线程分批更新，TUI 不执行全文件阻塞解析。
- 正常文本保存在有总量上限的 normalized model；超大记录跳过并累计诊断。
- 兼容 event_msg、response_item、task_started/task_complete、item_completed PascalCase 类型；优先 content_item_kinds 识别用户输入。
- 搜索索引随 Turn 变更增量更新。session_index 仅补充标题，真实 rollout 为准。
- 所有 fixtures 人工合成，不复制真实 Prompt、工具输出或认证数据。
- 核实计划与规格一致后直接实施；只有真实阻塞才暂停。

## 验收结果

全部 7 个阶段已完成。91 项自动测试通过；fmt、clippy（零 warning）、release 构建和伪终端交互验收通过。真实历史文件 hash 保持一致，真实活动会话观察到增量追加，无 reset；没有向 Codex 写入任何数据。

发布文件：target/release/codex-nav（1.0.0）。文档：README.md、docs/architecture.md、docs/session-format-notes.md、docs/qa.md。详细测试与平台限制记录在 docs/qa.md。

