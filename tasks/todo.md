# Codex Navigator v1.0 实施计划

## v1.2.0 — B 专注阅读 Web 正式版

用户已选择 B 并授权推进。实施约定见 docs/web-implementation.md；确认既有 CLI/解析器/只读 worker 后并行实施，最后集成验收，不再停在预览。

- [x] 内嵌资源与 --web / --port / --no-open，保留 TUI 及现有启动参数。
- [x] 实现本机令牌认证、来源校验、分页与有界缓存的只读 API。
- [x] B 主会话入口、搜索、正文/活动/最终回复、复制、历史保护与响应式键盘导航。
- [x] HTTP 安全/状态/增量测试、前端状态测试与真实浏览器宽窄验收。
- [x] fmt、clippy、全量 tests、release、终端回归；同步文档与知识记录。
- [x] 验证只读边界，核对独立 v1.2.0 commit/tag 内容，不 push。

### v1.2.0 验收结果

157 项 Rust 测试（含 11 个 Web 单测、13 个 HTTP 黑盒集成）、18 项前端单测全部通过。fmt/clippy/Prettier/ESLint、release build、六组终端回归通过；终端 harness 修正同尺寸误清屏与 resize 后发键竞争后连续通过。浏览器 1280/390px × 实时/手动刷新四组通过，覆盖 105 轮目录、64 条活动窗口、复制降级、请求失败自动恢复、历史 DOM/分页/滚动保护、精确新增提示与安全渲染。

真实本机 Picker 显示 14 个主会话，主会话 API 正常加载；另显式只读历史子会话 12 轮，文件 SHA-256 不变，不记录内容。已打开正式本机网页供使用。详细结果与边界见 docs/qa.md；交付为独立本地 v1.2.0 提交与版本标签，不执行 push。

## Web 设计预选 — 已选择 B，保留设计阶段记录

范围：只做三套可交互 HTML 预览及本地展示，不实现 --web、HTTP 会话 API 或真实会话接入；现有 binary 与版本不变。使用合成数据，保持主会话入口、生命周期/活动警告分离及最终回复定位语义。

- [x] 写明三套视觉 token、布局和交互范围，确认差异不是仅换色。
- [x] 制作 A 清晰工作台、B 专注阅读、C 紧凑控制台，以及统一切换展示页。
- [x] 浏览器验证会话选择、搜索、时间线、最终回复、折叠活动、宽窄布局和键盘；核对无外部资源/真实会话请求。
- [x] 记录预览验收、打开本地展示页并准备独立设计预览提交；不发布新 binary 版本。
- [x] 用户已选择 B，进入正式 Web 实施计划。

### 设计预览验收

四份 HTML 内联 JavaScript 语法与无外部依赖检查通过。三套样稿在约 1280 / 390px 外框宽度下各 18 项浏览器断言通过，覆盖默认会话选择、搜索空态、选择、f/g/G、折叠、帮助、返回与无横向溢出；A 的模拟追加另检查历史保护和无 final 提示。已查看桌面三方案和手机截图，修正 A 的按钮焦点快捷键及选中轮次可见性。

原有 132 tests、fmt、clippy 与 release build 通过；Rust 源码与 binary 版本未修改。预览服务只绑定 127.0.0.1:8877，仅提供 docs/design-preview。后台 QA tab 已关闭，按用户展示需求保留新开的预览窗口；未操作已有页面。正式 Web 开发等待用户选择，不创建发布标签。

## v1.1.2 — 轮次生命周期与活动警告分离

正常完成只描述执行生命周期，不判断答案正确。工具/命令错误保留 activity.errors，Timeline 独立显示 !N；显式轮次错误显示 ✕，中断显示 ⊘，完成显示 ✓。未知/未完成/rollback 语义保留。Web 仅评估当前架构，不新增服务或网络监听。

- [x] 补充失败后重试再完成、未结束活动错误、明确执行错误、中断、晚到工具记录与 rollback 的 parser/state 测试。
- [x] 分离状态判定及错误计数，更新 Timeline、正文摘要和帮助，确保不宣称答案正确。
- [x] 验证宽窄终端、f/g/G、完整测试及 release；同步使用文档、Web 支持边界、经验和版本。
- [x] 记录验收并准备本地 v1.1.2 commit/tag，不推送。

### v1.1.2 验收结果

旧实现先复现“活动失败后正常 completion 仍为 Failed”；修复后新增 6 项 parser/state 和 4 项 UI 回归通过。全量 132 tests、fmt、clippy 零 warning、release build、6 组真实终端验收通过。只读检查本机 12 份 rollout 的事件类型，不记录内容；合成终端测试文件 hash 不变。Web 仅完成架构评估与文档，未实现或启动网络服务。

## v1.1.1 — 默认主会话选择入口

按最新反馈修正入口：不传 --session 时始终进入 Picker，只列明确识别为 MAIN 的会话；子代理与 UNKNOWN 不进入列表。--all 仅扩展日期范围；显式 --session 仍可直接打开任意类型，不重定向父会话。保留 cwd 相关度与更新时间排序。

- [x] 补充主会话过滤、显式打开非主会话、单一主会话不自动打开和返回/刷新 Picker 回归。
- [x] 移除自动打开路径，统一启动与返回列表的主会话过滤，更新空态和 CLI 说明。
- [x] 同步文档、经验、版本；通过 fmt、clippy、全量 tests、release 和实际终端验收。
- [x] 记录验收结果并核对本地 v1.1.1 提交内容与标签命名，不执行 push。

### v1.1.1 验收结果

122 tests 全部通过，fmt、clippy 零 warning、release build 通过。旧 release 已由真实终端测试复现唯一 MAIN 自动打开，新版终端 5 组验收通过，覆盖启动/Enter/s/r/搜索/--all、非主会话空态、显式子会话 ID/path 和原有 f/g/G/终端恢复。所有合成文件 hash 不变，不修改 Codex 数据。交付版本为 1.1.1，以独立本地 commit/tag 保存。

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
