# 有界任务上下文与重复失败恢复

[English](task-context-recovery.md)

编码启动结果在 `continuation.task` 提供恢复指针；详细续接反馈在
`attempt.task_context` 提供同一投影。`read_session_handoff` 和
`finish_coding_task` 将这些字段放在 `handoff_brief.task`。它们只读取已有证据，
不会新增任务数据库、执行循环、进度结论或权限。

| 字段 | 含义与来源 |
| --- | --- |
| `scope` | 精确 Session、Project 和保留的任务指令事件；无法确认任务边界或安全范围时，`complete=false`。 |
| `relevant_files` | 当前任务中成功的规范事件所记录的最多六个去重项目相对路径，附带 `event_id` 和修改/观察来源。 |
| `failure_locations` | 最多三个安全的测试或诊断名称，可附文件/行号，并引用规范续接验证证据。 |
| `decisions`、`recent_progress` | 明确记录的 Session 笔记及消息 ID。笔记可供同一 Session 的后续任务参考，不证明实际执行或完成。 |
| `next_steps` | 固定的建议动作，附有界 `source_refs`，指向事件、Session 或验证证据。 |
| `continuation_entry` | 此 Session 的显式 `read_session_handoff` 恢复入口，不会隐式选择其他 Session。 |
| `stall` | 独立于进度、验证和收尾结论的有界重复失败诊断，仅供参考。 |

恢复启动仍然描述新指令被接受前捕获的上一个任务，其显式边界指明该任务。
当前交接只复用 Session、Project 和任务边界均一致的上下文。快照在读取期间
发生变化时，诊断不可用，并建议先重新读取该精确 Session 的交接结果。

每个恢复列表都有 `total`、`returned` 和 `truncated`。任务上下文本身的序列化
JSON 上限为 4 KiB，计入现有启动预算；交接包含新增字段后仍遵守原有 8 KiB
硬上限。安全过滤或字节预算导致的省略会明确标记，省略后的路径或笔记不代表
完整记录。不安全或过长的范围标识会被省略，不会截短后冒充另一标识。
`event:` 和 `session:` 引用须在精确 Session 的账本中解析；验证引用指向显式
详细交接恢复（`diagnostic=true`）中的 `continuation_feedback`。
不会暴露文件正文、从正文推断的符号、argv、stdout/stderr 或新增指纹。

诊断最多检查当前任务、精确 Session/Project 内最近 20 个相关的规范完成事件。
轮询、心跳、分页、规范定义中的支持/传输活动，以及收尾投影不计入；恢复读取不会制造新证据。
只有三个不同、非预期、仍需处理的失败同时具有相同的
规范执行/目标哈希和完整结构化失败标识，且其间没有新的相关观察或成功，才会
报告 `suspected`。先合并 recorder/business 重复记录；重放的事件、调用或
执行关联 ID 不能凑足阈值。预期失败、已解决或过期失败不计入。目标或结构化
失败不同会中断连续失败。解析器标识保留测试名称/失败位置，排除耗时和日志噪声。

普通命令预览及脱敏、截断的输入摘要不能证明 argv 相同。缺少规范标识、
诊断不支持或证据不完整、任务边界缺失时返回 `unavailable`。哈希必须来自当前
任务内唯一的先前启动事件，并以精确调用 ID、Session/Project/工具和逻辑关联
元数据配对；不使用历史时间戳回退。配对缺失、歧义或跨任务边界也返回不可用。缺失或看起来
未变化的有损日志不能代替结构化失败证明。有些适配器只提供计数、不提供失败
标识，此时保守返回不可用。`not_observed` 仅表示这个有界窗口未达到阈值，
不证明任务正在取得进展。

窗口内任何仍需处理的不确定执行结果都会报告 `reconcile_required`，即使其后
读取过文件。两个简报都会用对精确 Job/执行结果的核对建议替换执行建议，
核对后才能考虑再次执行。规范的当前证据核对可以排除已解决的先前失败。
诊断不会重试、派发、调度、改变权限、消费续接或改变至多一次执行语义。
即使缺少执行状态证据，结构化的 `outcome_unknown` 失败类型仍要求核对结果。
具有精确 Job ID 的成功异步接收记录由现有 Job 生命周期投影处理；其历史记录中
命令尚未完成的标记不会永久触发结果未知警告。明确的结果未知证据仍优先于 Job 接收状态。

参见 [Session 契约](../agent/session-model.md#explicit-handoff-recovery-and-internal-snapshot-fencing)
及[启动投影归属](../architecture/startup-projection-boundaries.md)。
