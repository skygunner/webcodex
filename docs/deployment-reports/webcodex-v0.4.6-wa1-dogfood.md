# WebCodex v0.4.6：wa1 ARM64 Dogfood Canary 部署报告

- 日期：2026-10-12
- 执行机：ZCode 本机（git/merge）；目标主机：wa1（ARM64）
- **最终结论：DOGFOOD VALIDATED**（隔离 Canary 验证成功，生产未受影响）

## A. Git Evidence

| 项 | SHA |
|---|---|
| 已验收 integration SHA | `230731637ec8c663ce85ce0507a916501cd5ab52` |
| dev merge commit | `024cc30de936778978fd42343e7abd591feaf519`（`git merge --no-ff`，零冲突） |
| Gitea origin/dev | `024cc30de936778978fd42343e7abd591feaf519`（普通推送，远端回读一致） |
| Upstream v0.4.6 | `5072608fe74b2bfa6e8f208515c04242f475582f` |

- ancestry 证明：`git merge-base --is-ancestor 23073163 HEAD` 通过；
  `git merge-base --is-ancestor v0.4.6 HEAD` 通过。
- 合并树核对：`git diff 23073163 HEAD` = **0 文件**——合并零代码变化，独立审查的
  测试证据直接覆盖 dev HEAD 树；合并后 `cargo fmt --check` + `cargo check
  --workspace --all-targets` 通过。
- 无新冲突（dev 的 merge-base 恰为 integration 基线 49ed5263，纯前进合并）。
- `git diff --check` 干净；dev/prod 既有历史未重写；未动 prod 分支。

## B. wa1 Baseline

| 项 | 值 |
|---|---|
| OS / Arch | Ubuntu 22.04.5 LTS / aarch64，4 核 |
| RAM / 磁盘 | 23G（22G 可用）/ 42G 空闲 |
| `/tmp` | `rw,nosuid,nodev,noexec`（**noexec**） |
| Rust / C 工具链 | cargo/rustc 1.98.1 / gcc 11.4 |
| Node.js | **不存在** |
| 生产 WebCodex | `~/.local/bin/webcodex` 0.4.3 (commit 49ed52633dac, dirty=false) |
| 生产单元 | `webcodex-share-webcodex.service`（ChatGPT MCP 通道：`webcodex share --tunnel none --public-url https://wd.palmit.au --auth oauth`，server 绑 127.0.0.1:36596）+ `webcodex-mcp-proxy.service`（127.0.0.1:39507） |
| 生产数据目录 | `~/.local/state/webcodex/projects/personal/webcodex-dd14e61ef9/data`（webcodex.db 6.8M + WAL） |
| 生产依赖图 | mcp-proxy ← share-webcodex（After）；两者共用 `~/.local/bin` 三件套 binary；`deploy_dogfood.sh` 管理 `webcodex-share-*` 单元 = **会中断生产，本轮未使用** |
| 隔离设计 | Canary 全新目录 `~/webcodex-canary-v046/`（src/data/projects/logs/repo/凭据）、独立端口 127.0.0.1:36601、独立 Runner 身份 `canary-v046-agent`、独立 systemd 用户单元 `webcodex-canary-{server,runner}-v046.service`（与生产单元无任何 After/PartOf/BindsTo 关系）、独立 token（`~/webcodex-canary-v046/canary.env`，600）、构建产物留在 build target 不触碰 `~/.local/bin` |

## C. Backup

- 一致性备份：python3 sqlite3 在线 `backup()` API（只读 URI 打开生产 webcodex.db →
  写副本），产物 `~/webcodex-canary-v046/prod-baseline/db-backup/`；
  `PRAGMA integrity_check` = ok，48 tables；sessions.json 与 private/ 一并拷贝。
- 目录权限 `go-rwx`（700）。无 Secret 进入日志/报告/Git（token 仅存 600 文件）。
- 生产不变性基线：进程清单、端口、单元状态、三件套 SHA256 存档于 prod-baseline/。
- 恢复步骤：Canary 完全独立——`systemctl --user disable --now
  webcodex-canary-{server,runner}-v046` 即彻底撤销，生产零接触；生产自身回滚
  资源 = 现存 `~/backup-webcodex-*` 目录 + 本轮 SHA 记录（本轮未做任何生产变更）。

## D. Build Identity（wa1 原生 ARM64 构建）

构建：`CARGO_BUILD_JOBS=4 cargo build --locked --workspace --bins --profile dogfood`
（systemd 瞬态单元承载，6m45s 墙钟 / 24.5min CPU）。三件套：

| Binary | Version | Commit | Dirty | SHA256 |
|---|---|---|---|---|
| webcodex | 0.4.6 | 024cc30de936 | false | `da5a4ea8d1c04f08…` |
| webcodex-server | 0.4.6 | 024cc30de936 | false | `76e365671ab04cad…` |
| webcodex-runner | 0.4.6 | 024cc30de936 | false | `41b769ed53ba171b…` |

（完整 SHA256 见 `~/webcodex-canary-v046/canary-binary-sha256.txt`。构建源 = 推送后
origin/dev 024cc30d 深克隆，`git status` 0 dirty。）

## E. Canary Deployment

- Server：`webcodex-canary-server-v046.service`，pid 隔离单元内，binary 绝对路径
  `~/webcodex-canary-v046/src/target/dogfood/webcodex-server`，监听 **127.0.0.1:36601**，
  data `~/webcodex-canary-v046/data`（全新，无生产数据）。
- Runner：`webcodex-canary-runner-v046.service`，client_id `canary-v046-agent`，
  独立 project_registry_dir + 测试 repo（canaryproj/smoke 两个 disposable 项目）。
- 认证：全新生成 WEBCODEX_TOKEN（bootstrap，loopback only），remote shared-key 未启用，
  无匿名访问；凭据不复用生产。
- 与生产隔离确认：进程/端口/数据目录/凭据/单元名五维全独立；生产 ChatGPT MCP
  通道全程在线。

## F. Functional Test Matrix

自动化验收驱动：`scripts/dogfood/wa1_canary_acceptance.py`（对 Canary loopback 执行，
23/23 通过；驱动运行全程约 4 分钟）。逐项：

| Area | Test | Expected | Actual | Status | Evidence |
|---|---|---|---|---|---|
| F1 Server | /healthz | 200 | 200 | PASS | curl |
| F1 Server | serverInfo | webcodex 0.4.6 | webcodex 0.4.6 | PASS | MCP initialize |
| F1 Server | SQLite 初始化 | 新 data 目录建库 | 正常，无生产污染 | PASS | data/ 独立 |
| F2 Runner | 注册 | canary-v046-agent 上线 | online | PASS | list_projects |
| F2 Runner | Project 注册 | canaryproj 可见 | 可见（含 online 标记） | PASS | 错误回执候选列表 |
| F2 Runner | 无害只读命令 | stdout 返回 | printf/shell 均返回 | PASS | acceptance F3 |
| F3 MCP | tools/list | 28 工具 | 28（含 job_write_input 等新工具） | PASS | acceptance |
| F3 MCP | tool_manifest/runtime_status/list_projects/read_files/search/run_process/run_shell/show_changes/review_changes | 成功 | 全部成功（review_changes 需 `scope` 字段 = v0.4.6 新契约） | PASS | acceptance |
| F3 MCP | 长尾迁移 | wait_for_job_terminal 经 call_runtime_tool 可达 | 参数级错误证明工具存在 | PASS | acceptance |
| F4 Job | 短命令内联完成 | 完成 | quick-done 返回 | PASS | acceptance |
| F4 Job | 长任务交接 | job_id（compact 回执） | continuation 携带 job_id | PASS | acceptance |
| F4 Job | observation baseline/token/delta | token 连续、日志不重复 | L-START/L-END 各一次 | PASS | acceptance |
| F4 Job | 失败路径 | exit_code 3 终态 | status=failed exit=3 | PASS | acceptance |
| F4 Job | 超时路径 | 终态 timeout | status=timeout exit=-1 | PASS | acceptance |
| F4 Job | stop_job 取消 | 终态 stopped | terminal（需 project+job_id+confirm） | PASS | acceptance |
| F4 Job | **observe_jobs wait.reason**（整改修复实测） | 真实唤醒事件 | meaningful_change wake 如实上报（本机单元测试覆盖截断路径；现场覆盖 Updated 路径） | PASS | acceptance |
| F4 重连 | Runner 断连 | `recovering` | recovering + changed=true | PASS | 现场实测 |
| F4 重连 | Runner 实例替换 | 旧 Job fence 终态 lost（不可收养） | lost/terminal | PASS（符合文档契约） | 现场实测 |
| F5 Auth | 无效/缺失 token | 401 | 401/401 | PASS | acceptance |
| F5 Auth | 未知 project | fail-closed | 拒绝 + 候选提示 | PASS | acceptance |
| F5 Auth | 异构 job_id | 拒绝/不存在 | 拒绝 | PASS | acceptance |
| F6 Artifact | 上传/分块/完成/SHA256/读/abort/删除/清理 | 27 项 | **27/27** | PASS | smoke_artifact_transfer |
| F7 LSP | `cargo test -p webcodex-lsp` 于默认 noexec /tmp | 63/0、无 Permission denied | **63/0/0**（无需 TMPDIR） | PASS | canary-lsp-test 单元日志 |
| F8 Node | 前置条件（wa1 无 Node） | 明确 `precondition failed`，不计 PASS | `no Node.js on PATH … install Node.js >= 22.7` | PASS（预期行为） | canary-node-test 单元日志 |
| 持久化 | Canary SQLite | 正常读写 | 各 Job/会话落库，生产目录零写入 | PASS | data/ 独立 |

MCP 兼容性备注：v0.4.6 工具面与生产 0.4.3 非同名一一对应（4 新增、2 下沉长尾、
2 情境化 continuation App）；`review_changes` 现要求 `scope` 字段；交接回执为
compact pending 形态。旧客户端迁移路径：长尾经 `call_runtime_tool`（已实测）、
App continuation 回执自带 parser-ready 跟进（已实测）。

## G. Performance（Canary 单侧采样，非严格 A/B）

| case（20 采样） | median | p95 | err |
|---|---|---|---|
| tools/list | 145.93ms | 147.40ms | 0/20 |
| tool_manifest | 6.58ms | 7.25ms | 0/20 |
| runtime_status | 3.29ms | 6.18ms | 0/20 |
| read_files 单文件 | 5.59ms | 6.14ms | 0/20 |
| observe_jobs（已完成 Job） | 5.87ms | 6.28ms | 0/20 |

- **与生产实例的对比未执行**：生产通道为 OAuth 认证，本轮授权禁止接触生产凭据，
  故不做跨实例测量（环境与授权差异也会使绝对值不可比）。跨版本 A/B 已在 ZCode
  x86_64 完成（tool_manifest −95%，见集成报告 §9），wa1 数据与其同一量级方向
  （tool_manifest 6.6ms ≪ tools/list 146ms）。
- 测试条件：wa1 本机 loopback、canary server、python urllib 串行。

## H. Risks

- **Blocking**：无。
- **Non-blocking**：Node.js 相关 real-process 测试在 wa1 未执行（缺依赖，前置条件
  如实失败；如需覆盖可在隔离工具链安装 Node ≥ 22.7 后重跑，本轮未装以避免动生产环境）。
- **Environment-specific**：wa1 `/tmp noexec`——LSP/测试 fixture 已由整改轮修复，
  本轮在真实环境验证通过（63/0，无 TMPDIR）。
- **Pre-existing flaky**：cloudflared 测试族高并发 `ExecutableFileBusy`（Canary 不含
  Tunnel 组件，不受影响；未跑该族以免噪声）。
- **Not tested**：Windows/macOS（无环境）；生产切换后的 OAuth 全链路（属下一阶段）。

## I. Rollback

- Canary 撤销：`systemctl --user disable --now webcodex-canary-server-v046
  webcodex-canary-runner-v046`；可选清理 `~/webcodex-canary-v046/`。生产不受影响
  （无共享 binary/端口/数据/单元依赖）。
- dev 合并撤销：以新 revert commit 前进（不 rewrite）；仅 Canary 失败无需回滚 dev。
- 数据：Canary 独立新库，不存在写入生产路径；生产数据未被新版 schema 触碰，
  正式切换时的迁移/回滚方案留待切换设计（见下）。

## J. Final Decision

**DOGFOOD VALIDATED**

隔离 Canary（v0.4.6 @ 024cc30d，ARM64 原生构建）在 wa1 上通过全部核心验收：
Server/Runner/MCP 工具面/Job 生命周期（含本轮 observe_jobs 修复的现场验证）/认证
负路径/artifact 27 项/LSP noexec 63/0/重连语义均符合预期；生产 ChatGPT MCP 通道
全程在线且二进制/端口/单元/数据零变化（SHA/快照逐项 IDENTICAL）。

### 下一阶段（正式生产切换）所需的明确授权事项

1. **维护窗口**：`deploy_dogfood.sh` 将停止/重启 `webcodex-share-webcodex.service`
   （ChatGPT MCP 通道会中断数分钟）——需用户明确批准窗口与中断。
2. **生产数据迁移授权**：0.4.3 → 0.4.6 首次启动将执行 SQLite 迁移（单向）；
   需批准迁移前的一致性备份策略（本轮已建备份流程可复用）与失败回滚决策。
3. **生产凭据边界**：share 单元每次启动轮换凭据属既有产品语义；切换后需按现有
   runbook 重新分发 MCP 客户端连接信息。
4. **prod 分支更新**：本轮未动 prod；正式切换时是否推进 prod 分支由用户单独授权。
