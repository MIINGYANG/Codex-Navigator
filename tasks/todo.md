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

## v1.0.1 — 本地版本历史与正文首尾跳转

需求已确认：g/G 按焦点生效；时间线继续选择第一轮/最新轮，正文跳到当前轮顶部/底部，宽窄布局切换后行为一致。建立本地 main 分支，以独立 commit 和版本标签保留迭代历史。

- [x] 初始化 Git，提交现有 v1.0.0 基线并建立标签。
- [x] 补充宽窄切换与正文 g/G 回归测试，再修复按键分发。
- [x] 更新帮助、使用文档、CHANGELOG 和版本号至 1.0.1。
- [x] 通过 fmt、clippy、全量测试、release 与终端交互验收。
- [x] 记录验收与经验，核对 v1.0.1 提交内容和版本标签命名。

### 本次验收结果

基线 commit e9fd58a，标签 v1.0.0。新增测试先在旧实现上复现 g/G 错误切换 Turn，修复后 4 个导航回归测试全部通过。全量 95 tests、fmt、clippy、release 构建和真实伪终端宽窄切换验收均通过。

正文 g/G 只改变当前正文滚动位置，保留所选 Turn 和新增消息提示；时间线 g/G 语义不变。版本号、帮助、README、使用指南、CHANGELOG 与本地迭代约定均已同步。交付通过独立修复 commit 和 v1.0.1 标签记录，提交历史可用 git log --graph --decorate --oneline --all 查看。

## v1.1.0 — 最新输入保护、会话辨认、最终回复定位

三个已授权需求作为同一可验收版本推进；只读取真实 Codex 文件，所有写入与测试数据留在 Navigator 仓库/测试目录。

- [x] 建立内存耗尽回归：历史输出不能挤占 Prompt；连续大量 Prompt 后最新输入仍可见、可搜索，内存保持有界。
- [x] 将保留文本拆成 Prompt 与活动两个滚动预算；淘汰旧正文时保留时间线摘要和明确省略提示，增量更新搜索。
- [x] 按真实 metadata 识别主会话、子会话、未知来源，显示父会话/代理标识及更新时间，排序优先相关主会话，监控状态不暗示模型正在工作。
- [x] 使用 f 定位当前轮最后一条可靠最终回复；标注 FINAL ANSWER，没有明确结果时提示，不猜测，不切换 Turn。
- [x] 覆盖 parser/discovery/state、宽窄渲染、增量 worker 和实际终端交互；验证真实本机会话的识别信息，不记录敏感内容。
- [x] 更新 README、CHANGELOG、版本及验收记录；通过 fmt、clippy、全量 tests、release build 后准备本地提交和 v1.1.0 标签。

设计边界：每条 Prompt 仍保留 256 KiB 上限，Prompt 总预算 16 MiB，活动总预算 48 MiB；超限时让位给新内容。更早的 Prompt 保留预览，历史全文省略必须有明确标识。最终回复定位只针对已保留且有可靠标记的消息。暂不加入全文分页、磁盘缓存或跨会话正文搜索。

### 本次验收结果

新增 26 项测试，全量 121 tests 通过；fmt、clippy 零 warning、release build、终端宽窄导航和恢复验收通过。内存测试覆盖 49 MiB 工具压力、17.5 MiB Prompt、48 MiB 精确预算边界、长中文回复 phase 升级、后台 dirty 淘汰同步。真实主/子会话身份正确，分别检测到 9 / 4 条最终回复，历史文件 hash 不变。详细结果见 docs/qa.md；交付为 codex-nav 1.1.0，本地独立提交与 v1.1.0 标签，不执行 push。
