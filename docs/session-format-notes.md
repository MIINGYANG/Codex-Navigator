# 本机会话格式与兼容性

## 2026-09-06 本机只读检查

完整阅读产品规格后，只读抽样了最近三个 rollout，随后针对记录结构做有限补充检查。检查时本机有 176 个 rollout，存在 session_index.jsonl；样本 CLI 版本为 0.153.2。此文仅记录字段结构，未复制真实 Prompt、工具输出、认证信息或 reasoning 内容。

- session_meta：id、session_id、timestamp、cwd、cli_version、source。子代理继承历史可能再次出现父 session_meta；取文件中的第一份身份，避免覆盖。
- event_msg / task_started：turn_id、started_at；task_complete：turn_id、last_agent_message、started_at、completed_at、duration_ms。中断使用 turn_aborted。
- response_item / message：role、content、phase；user 的 internal_chat_message_metadata_passthrough 包含 turn_id 和 content_item_kinds。
- content_item_kinds 的 user.text / user.image 对应真实输入；agents_md.instructions / environments.environment_context 是上下文。优先按此结构分类，不能把所有 role=user 都显示为 Prompt。
- event_msg / item_completed：turn_id、item、started_at_ms、completed_at_ms。item 使用 PascalCase 类型。
- UserMessage.content：text 类型片段；AgentMessage.content：Text 类型片段，phase 为 commentary 或 final_answer。
- 图片输入可能在 input_image 前后带 `<image ...>` / `</image>` 包装，而 UserMessage 只保留 local_image 和真实文本。仅在相邻 image 结构明确时去掉包装，保证两种记录不会重复合并同一个问题。
- CommandExecution：command 字符串数组、parsed_cmd、status、aggregated_output、stdout、stderr、exit_code、formatted_output。FileChange.changes 为路径到修改对象的映射。
- function_call / custom_tool_call 和相应 output 仍然存在；custom_tool_call.input 可以是脚本字符串。某些脚本内部命令另有 CommandExecution item，因此它们作为父工具与命令活动分别显示。
- reasoning、Reasoning、agent_reasoning、world_state、token_usage_record、compacted.replacement_history 不进入 Viewer；不展示隐藏推理、加密内容或压缩上下文。
- 索引字段实际为 id、thread_name、updated_at。索引只是辅助，不能替代 rollout。

## 兼容策略

同时支持旧 event_msg.user_message / agent_message、response_item.message，以及 task_started / turn_started 等显式边界。明确边界内合并用户片段；无边界时按用户输入建立 Turn。去重结合输入来源、邻近距离、Turn 身份和真实 agent 活动，避免相同文本的不同提交被全局去重。

未知结构跳过。完成状态只来自明确 completion，失败使用结构化 error、非零 exit code 或旧 shell 固定格式的退出码行；仅出现单词 error 不等于失败。rollback 有明确 turn_ids 或 num_turns 时标记，无法映射时保留 unknown。

所有 session 使用 File::open，只读解析。JSONL 的换行符作为提交标记，末尾无换行记录保留到后续补齐；首记录允许 BOM。默认单记录上限 4 MiB，超限流式丢弃到换行后继续。后台每批最多读取 4 MiB，使主界面在大文件加载期间保持可交互。

这些格式是观察结果，不是稳定的官方 API 合同。tests/fixtures 内全部为人工构造的最小样本。
