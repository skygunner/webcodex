# ChatGPT 任务执行与证据

[English](chat-task-loop.md)

`work_on_project` 保持严格的 selector 校验。参数组合错误时说明授权发现路径，
不回显被拒的 selector，也不据此判断 Runner 离线：已知项目使用规范 Project
或已签发的 `project_ref`，未知项目使用 `list_projects`；路径必须配合确切的
`client_id`。复杂任务先调查规则和代码、明确可观察的完成条件、展示简短计划，
然后继续已授权执行。默认仍为 `on_demand`，展示计划不会创建持久 Goal。
读取续接沿用原参数；执行结果未知或仍在运行时保留原 Job，先做独立工作，
再汇合确切的阻塞 Job。工具成功、Job 结束、步骤勾选不能证明用户目标完成。

## 隔离验收与严格评测

现有 [评测运行器](../../scripts/agent_loop_benchmark.py) 支持清单中的六个
`generic_agent_disposable_fixture` 案例。每个样本使用普通临时目录；源码 revision
仍固定清单与运行时证据规则。[共享 fixture](../../scripts/e2e_generic_agent_ws.py)
提供有界文件 oracle。Host driver 必须把提供的固定 locale、时区和 Python 环境
应用到经 WebCodex 执行的命令。

通用样本新增 `WEBCODEX_BENCH_TARGET`、`WEBCODEX_BENCH_WORKSPACE`、
`WEBCODEX_BENCH_FIXTURE_ENVIRONMENT`（JSON 子进程环境）、
`WEBCODEX_BENCH_FIXTURE_COMMAND`（指定的失败日志压力脚本）以及
`WEBCODEX_BENCH_FIXTURE_COORDINATION`（项目外协调目录）。过期读取案例中，
driver 保留确切读取回执后发出 `first-read`，等待 `external-written`，执行旧
revision 的编辑，确认拒绝后发出 `stale-rejected`，等待 `rejection-checked`
再重新读取和完成。外部写入只使用一个已验证的普通文件描述符，不跟随被替换
的符号链接。协调信号本身不是工具选择、权限或执行的运行时证据。

严格验收使用 `--strict`、明确的 `--case-id`，以及强制 lane 的 `--pairs`
或 `--experiment tool_selection --samples`。零退出码要求所选案例和次数完整，
任务及必需验证通过、有运行时 lane 证据、所有可用文件 oracle 通过。
缺失案例、`partial`、`unsupported`、缺失必需证据均不能通过；driver 自报不能
证明执行。

目前四个时序案例（失败修复、后台汇合、过期 revision、缺失输出）在 Host
评测中保持 `partial`：现有隐私受限审计不能证明全部有序结果语义。文件与 CSV
案例可在运行时及 oracle 门禁通过后合格。六案例严格运行不能宣称全部通过。
真实 Server/Runner 确定性 smoke 另行验证这些运行机制，不能证明 ChatGPT
选择了正确动作。可执行命令见英文对应文档。

## ChatGPT Host 验收

仅使用隔离临时项目和授权 Runner，直接发送：

> 调查这个 fixture，展示简短计划，修复 `add(a,b)` 返回 a+b，运行提供的失败
> 测试，诊断并修复，再次验证、审阅实际 diff 并交付。无需常规确认继续执行，
> 不得声称未观察到的验证。

应观察到计划、调查、带 revision 的编辑、真实失败与修复、通过的重跑、审阅和
有证据的交付。再加入截断读取、一个长 Job 与独立文件工作：读取使用确切续接，
长 Job 仅启动一次，独立工作后再汇合。跨聊天恢复原 Session，核对事实与当前
源码。错误 selector 应被拒绝并沿授权发现恢复，不应捏造离线判断。缺失产物或
验证证据时应如实报告未完成或 unproven。这些验收依赖真实 Host 观察，独立于
本地传输测试和模拟 driver 测试。
