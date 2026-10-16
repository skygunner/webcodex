# 续接状态与控制者检查点

[English](continuation-status.md)

Goal Plan 和 Runtime Console 将**续接条件**与 Host 派发、新回合证明分开展示。
就绪表示现有 operator 声明与当前精确、已授权的载体满足就绪检查，不保证立即启动新回合：
聊天应用仍可能延迟或要求用户确认。未就绪不禁止普通手动继续。
`accepted` 只是 Host 确认派发；只有精确 Wake consume 才证明本次续接回合运行。
本改动不新增就绪字段、权限或调度器。

控制者必须等齐选定工作者时，使用已有 Goal-scoped `AgentWait(mode=all)`：
显式关联确切 Task，并在启动执行**之前**注册 Wait。部分终态保留独立事实，
不产生逐 Task 控制者 Wake；最后一个匹配才创建一个 Wait Wake。执行 Wake 保持独立。
`delivery_unknown` 应核对原派发，不得盲目再次发送。

## ChatGPT 验收

在显式配置的测试控制者上展示 continuation card，并结束设置回合。下一回合展示确切
Goal Plan，检查就绪文案没有承诺零点击执行。触发一次已授权测试更新，分别检查 Host
派发与新回合证明。若 Host 要求 Send，由用户明确确认，再检查恢复回合消费原 Wake。
对于 ALL 检查点，先完成两个选定工作者之一；两者都结束前不应产生控制者 follow-up。
不要使用生产任务。

本地 DOM/React 测试验证文案与 accepted/consume 分离，Store 测试验证 ALL attention
抑制；这些结果不证明真实 Host 调度或确认弹窗次数。
