# WebCodex Fork 集成审计报告：上游 v0.4.6 升级

- 分支：`integration/upstream-v0.4.6`
- 报告日期：2026-10-12（v2：并入独立验收证据与本轮整改，结构按 14 节重排）
- 独立整改对账报告：[`webcodex-v0.4.6-review-fixes.md`](webcodex-v0.4.6-review-fixes.md)
- 执行环境：`/home/vincenty/Projects/webcodex-integration-v0.4.6`（独立 worktree）
- 说明：全部命令经 bash 显式执行；原始测试日志在仓库外
  `/home/vincenty/Projects/webcodex-v0.4.6-evidence/`

## 1. Executive Summary

- **目标完成**：上游 v0.4.6（`5072608f`）全部 74 个提交通过显式 merge 完整集成
  （`git merge-base --is-ancestor` 证明），所有差异逐文件审计取舍。
- **fork 自定义功能全部保留**（§4）：Job 观测可靠性遥测、安全加固、部署脚本、
  文档与测试加固全部存活；被上游等价/更强实现取代的仅 noexec 测试加固（#775）
  与 aarch64 getpwuid_r 一行修复（#829）。最终 fork 覆盖层与设计预期闭式对账一致（§13）。
- **核心测试全部通过**：Linux x86_64 全工作区 Rust 普通套件 6708 passed / 0 failed
  （monolithic 3236 + runner/lsp 1049 + 其余 crates 2423，三组按 package 互斥切分，
  该加总即去重总数；35 个 ignored 不在其中）；runner real-process 69/0 额外运行；
  前端 172/172；全部 E2E harness + MCP conformance 通过。
- **独立验收（aarch64）已完成**并发现 7 项整改问题；本轮全部整改完成，
  详见整改对账报告。其中 2 个为预先存在的 TEST BUG（reconciliation 回执解析、
  artifact smoke 花括号陷阱），1 个为 fork 遥测语义缺陷（observe_jobs 截断
  wake reason，本轮确定性复现并修复），其余为测试环境适配与文档问题。
- **无阻塞**。性能 A/B 显示 tool_manifest 提升约 20×，无系统性退化（§9）。
- **状态：READY FOR FINAL INDEPENDENT REVIEW**（不构成生产部署授权，§12）。

## 2. Git Integration Evidence

### 2.1 基线（2026-10-12 实测核实）

| 引用 | 实测 SHA | 说明 |
|---|---|---|
| `origin/dev` = 本地 `dev` | `49ed52633dac63c697c02ca9c9a29d8eec1d07c4` | 与远端同步；集成分支基线 |
| `prod`（本地 = origin） | `d1636f72` | 任务书转述的 59adb1dd 与实际不符（那是 2026-09-28 审计时 dev 状态） |
| `upstream` tag `v0.4.6` | `5072608fe74b2bfa6e8f208515c04242f475582f` | annotated tag，tagger yyjeqhc，fetch 自 github.com/yyjeqhc/webcodex |
| merge-base(dev, v0.4.6) | `7301186b98527cb4ebc0191f6033474e9abf7c20` | #764 = fork nightly cutoff |

### 2.2 差异结构

- `git rev-list --left-right --count origin/dev...v0.4.6` → `117  74`
  （fork 独有 117；上游 v0.4.4/0.4.5/0.4.6 全量 74）
- patch-equivalence：`git log --cherry-mark` → 0 对（无双补丁）
- fork 曾按 SHA merge 的全部 ~90 个上游 commit 逐一验证均在 v0.4.6 祖先内
- 上游 diff：933 文件 +131923/−70164（1 rename：build.rs → webcodex-build-info；
  0 delete；380 create）；fork 侧独有 diff：65 文件 +1737/−266

### 2.3 Upstream Change Inventory（74 commits 按 release 分组）

- **v0.4.4 线**：model 执行/校验投影收敛（#765/#767）、project recipe 真相集中
  （#766/#798）、runtime bootstrap/Job UX（#768）、build identity 快照隔离（#769）、
  typed job 查询拆分（#770）、runtime 所有权/观测生命周期（#771）、tool surface
  policy 显式化 + 不活跃 continuation 下沉（#772）、Session ledger 索引与写合并
  （#773）、输入规范化（#774）、环境 fixture 隔离（#775）、工具契约元数据先选
  （#777）、Console 查询拆分（#778）、Tunnel/macOS 签名（#779）、Session/AgentTask
  selector（#781）、trace 证据（#782）、bash login durable job（#785）、scoped
  native test filters（#786）、timeout attention 规范化（#787）、Browser 批量表单
  （#790）、project_build（#789）、全局指令热载（#788）、跨平台服务生命周期
  （#793）、Browser/Window 性能（#791）、Runtime 刷新竞态（#795）、OAuth admin CLI
  （#797）、workspace provenance（#799）、runtime/Runner 模块拆分（#801）、
  Desktop 布局（#796）、客户端契约对齐（#802）、交互式 Job 输入（#803）、wake
  审批门控（#804）、session 恢复与 model 评审（#800）、CI fixture 可移植
  （#805/#837）、v0.4.4 发布（#806/#829）、Go 隔离（#807）、project op scope
  （#808）、retention 身份（#809）、锁定依赖策略（#810）、durable
  ArtifactHandoffGrant（#811/#813）、任务完成反馈（#812）、Window 协作连续性
  （#814）、general-agent 指引（#815）、tray 本地化（#824）、Brave 发现（#825）、
  Runtime 切换 Runner 身份（#827）
- **v0.4.5 线**：Work Result 审查入口（#828）、session-first 恢复（#831）、
  Projects & Resources workbench（#832）、webui 布局（#833）、MCP occupied provider
  等待（#834）、统一安装器（e98ab112）、Windows SCM SID（33ef276f）、macOS 自签
  （4b329f3f）、Intel Desktop 源身份（#837/168f66a3）、v0.4.5 发布（fe5a6d3e）、
  macOS Bash 3.2 签名（a4ece582）
- **v0.4.6 线（backport）**：runtime 读取路径优化（#862）、Windows onboarding 恢复
  （#863）、Desktop 本地工作流恢复（#865）、有界资源与历史生命周期（#866）、macOS
  review snapshot index（abe734d7）、Windows recipe 路径（4fcac8b5）、macOS 危险
  temp root（2d8fcc0a）、stale allowed roots（5a805f93）、未确认 project 刷新状态
  （5072608f）

### 2.4 集成方式与证明

- 方式：独立 worktree 中 `git merge --no-ff v0.4.6`（保留可审计历史）
- 证明：`git merge-base --is-ancestor v0.4.6 HEAD` → 通过（74 个上游 commit 全部
  ancestry 纳入，不依赖 patch-equivalence 推断）
- 独立复核命令：
  ```bash
  cd /home/vincenty/Projects/webcodex && git fetch upstream 'refs/tags/v0.4.6:refs/tags/v0.4.6'
  git rev-parse v0.4.6^{commit}
  git merge-base --is-ancestor v0.4.6 integration/upstream-v0.4.6 && echo INTEGRATED
  git -C ../webcodex-integration-v0.4.6 log --oneline -10
  ```

## 3. Conflict Resolution

9 个冲突文件，全部逐文件核实两侧提交来源后解决（无批量 ours/theirs）：

| 文件 | 上游侧 | fork 侧 | 解决 | 验证 |
|---|---|---|---|---|
| `crates/webcodex-environment/src/installer_unix.rs` | #829 的 `vec![0 as libc::c_char;16384]` | 49ed5263 等价修复 `vec![0u8;16384]`（aarch64） | 取上游（等价，规范形式） | 与 v0.4.6 逐字节一致；全套测试通过 |
| `crates/webcodex-runner/src/main_tests.rs` | #775 IsolatedEnv + 原生 executable_tempdir + TEST_ENV_LOCK | 7a951b58 内联 executable_tempdir + EnvGuard | 取上游（IsolatedEnv 子进程级隔离更强，取代 EnvGuard） | runner/lsp 分片 + real-process 组通过 |
| `main_tests/shell_config.rs` | #775（部分 tempdir 有意回退：fixture 不真正 spawn） | 7a951b58 | 取上游 | 分片通过；wa1 noexec 为披露风险 |
| `main_tests/shell_profiles.rs` | #775 | 7a951b58 | 取上游 | sshd 门控（7c7f4947）在 ssh.rs 未受影响 |
| `webcodex_runner/coding_agent.rs` | #801 + #775 | 7a951b58 | 取上游 | 分片通过 |
| `webcodex_runner/mcp_gateway_tests.rs` | #834 + #775 | 7a951b58 | 取上游 | 分片通过 |
| `webcodex_runner/plugin_tests.rs` | #775 | 7a951b58 | 取上游 | 分片通过 |
| `webcodex_runner/shell_tests.rs` | #775 | 7a951b58（另 2 merge 目标均在 v0.4.6 内） | 取上游 | 分片通过 |
| `webcodex_runner/validation/execute.rs` | #775 | 7a951b58 | 取上游 | 分片通过 |

共性依据：fork 侧在这些文件的改动仅来自 7a951b58，上游 #775 以更强设计实现同一
目标；fork 曾 merge 的所有上游 SHA 都在 v0.4.6 内，取上游侧不可能回退任何内容。

## 4. Fork Customization Preservation

| Fork 功能 | 来源 | 状态 | 证据 |
|---|---|---|---|
| Job 观测可靠性遥测（wake reason 细化、trace 诊断、returned_bytes、job_observation_* 指标） | 5165761f, a53b5599, d3578048, 7170a070 | **Retained + 修复**（截断 wake reason 语义缺陷由本轮确定性回归修复，见整改报告 C 节） | observe_jobs 64 + tool-contracts 272 + job_log_wait 19 + jobs 43 全绿 |
| 安全加固（远程 shared-key 边界、QUIC 闸门、Config redaction、供应链 SHA pinning、hardened temp hosts） | bcecfdee, 7dc7bb03, 921d415c, f990759b | **Retained**（重复警告由本轮去重为单一上游实现 + fork 关键信息合并，整改报告 D 节） | SH 分片全绿；workflows 全部 40-hex pin；安全分片 666 通过 |
| 部署能力（deploy_dogfood.sh / pull_compile_install.sh） | 706db7ac…d962a0d6 | **Retained**（本轮仅清理 EOF 空行，行为零变化） | bash -n 通过；dry-run fixture 通过；真实安装被禁止未执行 |
| 测试环境加固（sshd 门控、原子 stub、noexec temp） | 7c7f4947, 447e047b, 7a951b58 | **Replaced with equivalent upstream**（#775）；LSP 侧 noexec 兼容由本轮补齐（整改报告 E 节） | LSP 63/0 含新契约测试 |
| Nightly 适配（auth 收敛、aarch64 getpwuid_r） | e03a4033…49ed5263 | **Superseded/Retained** | monolithic 全绿 |
| 文档（job-observation playbook、security-hardening playbook、prod-deploy-report） | f990759b, 59adb1dd | **Retained** | TESTING.md 引用 3 处保留 |

## 5. Upstream Change Inventory

见 §2.3（74 个上游提交按 v0.4.4/v0.4.5/v0.4.6 分组的完整清单）。

## 6. Test Matrix

### 6.0 测试计数口径（重要）

- **Rust 普通套件 6708 = 3236（root `webcodex` --lib）+ 1049（runner/lsp：
  runner bin 985 + lsp lib 62 + config_preflight 2）+ 2423（其余 workspace crates）**。
  三组按 package 用 `--exclude` 互斥切分，一个测试只属于一组，**6708 即真实去重总数**。
- **ignored 35（3+4+28）不在 6708 内**：普通执行不运行 ignored 测试。
- **real-process 69/0、ManagedChild 15/0、persistent-shell 6/0 是显式 `--ignored`
  额外运行**，与普通套件零重叠（普通运行不执行 ignored 测试；带
  `runner-real-process-tests` feature 编译时 runner 的测试总数会因 ignored 体
  参与编译而上升，但普通执行仍未运行它们）。
- MCP App DOM 380（node --test）与前端 172（vitest）为独立测试面，不与 Rust 计数混合。
- E2E harness / conformance 是进程级验收，不折算 testcase 数。

### 6.1 Linux x86_64（本机 20 核，rustc 1.99.0）

| 测试 | 命令 | 结果 | 日志 |
|---|---|---|---|
| monolithic Server | `cargo test --locked -p webcodex --lib` | **3236/0/3**（55.9s） | monolithic-test.log |
| Runner+LSP | `cargo test --locked -p webcodex-runner -p webcodex-lsp` | **1049/0/4** | shards-test.log |
| 其余 crates | `cargo test --locked --workspace --exclude webcodex --exclude webcodex-runner --exclude webcodex-lsp` | **2423/0/28** | shards-test.log |
| runner real-process | `cargo test --locked -p webcodex-runner -p webcodex-lsp --features runner-real-process-tests runner_real_process -- --ignored --test-threads=1` | **69/0**（45.2s 串行） | runner-realprocess.log |
| ManagedChild | `cargo test --locked -p webcodex-process --test managed_child -- --ignored --test-threads=1` | 15/0 | process-shell-lanes.log |
| persistent-shell | `cargo test --locked -p webcodex-persistent-shell -- --ignored --test-threads=1` | 6/0 | process-shell-lanes.log |
| fmt / check / clippy / dogfood build | 见整改报告 G 节 | 全 PASS | |
| 前端 | `npm ci && typecheck/test/build/check:dist` | 172/172 + 全 PASS | |
| 供应链/发布契约/链接 | workflow_security 2、release_readiness 23、markdown 761/0 | 全 PASS | |
| fork 专项 H1/H3 | playbook 全部过滤器 | 全 PASS（整改报告 G 节矩阵） | focused-shards.log |
| observe_jobs 专项（本轮） | observe_jobs 64 + tool-contracts 272 + job_log_wait 19 + jobs 43 | **398/0** | p2-observe-jobs-verify.log |
| 安全分片（本轮去重后） | runner_quic 13 + config 22 + oauth 297 + mcp 316 + share_service 18 | **666/0** | p3-sharedkey-verify.log |

### 6.2 E2E / 集成（disposable loopback）

| 场景 | 结果 |
|---|---|
| e2e_reconnect / recovery_failures / hosted_connect / shared_key / zero_config | PASS |
| e2e_job_reconciliation（A-D） | PASS（修复 `50fc306d` 后；原始失败为预先存在 TEST BUG，基线复现证明） |
| smoke_artifact_transfer 活动模式 | PASS 27/27（修复 `580c5181` 后） |
| e2e_session_continuity_ws.py（上游新增） | PASS 4/4 |
| MCP conformance（2025-11-25 profile） | PASS 门禁级：33 场景全按 pinned 基线分类（24 个预期 FAILURE 为老 spec 有意拒绝；missing=0 unexpected=0 problems=[]） |
| MCP App DOM | PASS 380/380 |
| deploy_dogfood.sh dry-run fixture | PASS（零副作用） |

### 6.3 原始失败、定性、修复（如实记录）

1. **reconciliation 场景 C/D ×2 同签名失败** → compact 回执解析过期
   （**预先存在 TEST BUG**：基线 49ed5263 同 payload 复现相同回执）→ `50fc306d` 修复。
2. **artifact smoke 4 项失败** → bash `${2:-{}}` 花括号陷阱 + `git_available` 稀疏
   契约过期（**预先存在 TEST BUG**）→ `580c5181` 修复。
3. **前端 npm test 首轮 172 全挂** → Node 26 需 `--localstorage-file`
   （**ENVIRONMENT**；上游 CI 用 Node 22）→ 加 flag 后 171/172，余 1 例
   console-nav-requests 5vs8 定性 **FLAKY**（单测 3/3 + 全量复跑 172/172）。
4. **cargo check 2 个 warning**（fetch_update 弃用 + unused import）→ 文件与
   v0.4.6 逐字节一致，rustc 1.99 对上游既有代码的新告警，非合并引入，未改动。
5. **ARM64 独立验收 LSP 28 failed** → noexec /tmp（**上游既有环境适配问题**，
   crate 与 v0.4.6 逐字节一致）→ TMPDIR 修正后 1049/0/4；本轮已修复 fixture
   （整改报告 E 节）。
6. **ARM64 独立验收 real-process 68/1** → 唯一失败为 node 测试缺 Node.js
   （**环境依赖未声明**）→ 本轮加前置条件探测（整改报告 F 节）。
7. **observe_jobs wait.reason 截断失真** → **fork 遥测语义缺陷（本轮唯一
   PRODUCT REGRESSION 级发现）** → 确定性回归 + 修复（整改报告 C 节）。

## 7. MCP Compatibility

直接工具面 28 → 28（disposable 实测，tools-before/after.json）：

- **新增 4**：`job_write_input`（#803）、`open_webcodex_workbench`、
  `read_webcodex_resource`、`search_webcodex_resources`（#832）
- **不再静态列出 4**：`wait_for_job_terminal`、`wait_for_agent_events` → 仍在
  runtime 长尾可经 `call_runtime_tool` 调用（实测）；`present_agent_continuation`、
  `present_job_terminal_continuation` → 上游 #772 情境化 continuation App（内部
  引用保留，App 测试全绿）
- 9 个工具描述/schema 上游调优；observe_jobs 描述 331ch（<1024 预算）
- 回执契约（上游，nightly 时代已存在）：`run_process`/`cargo_check` 超宽限窗
  交接返回 compact pending 回执；`show_changes` 稀疏 `git_available`
- 兼容性风险：低；直接面无破坏，长尾迁移路径实测完好

## 8. Security Review

- SH-01~14 playbook 全部通过（SH-14 由 monolithic 证明）；本轮去重后安全分片
  666/0 复跑通过
- 远程 shared-key：local-only 无警告、remote disabled/enabled 各 1 条
  （行为实测）；认证谓词（`shared_key_enabled`/`shared_key_remote_enabled`/
  `shared_key_requires_remote_opt_in`）、HTTP Host/Origin、QUIC gate、OAuth、
  managed credential 检查**零修改**
- workflows 全部 40-hex SHA pinning；`test_workflow_security.py` 2/2、
  `test_release_readiness` 23/23
- 版本元数据与上游一致（0.4.6），fork 以 build identity 区分

## 9. Performance Comparison

同机、同 dogfood profile、同 fixture（300 文件×~2KB）、disposable loopback：

| case | 基线 49ed5263 | 集成后 | Δ |
|---|---|---|---|
| tools/list | 52.25ms | 53.63ms | +2.6%（载荷 −1.7KB） |
| tool_manifest | 52.74ms | **2.67ms** | **−94.9%** |
| runtime_status | 0.93ms | 1.14ms | +0.21ms |
| read_files 单/8 文件 | 1.79 / 43.91ms | 2.11 / 46.25ms | +0.32 / +2.3ms |
| search_project_texts | 58.76ms | 59.21ms | +0.8% |
| observe_jobs（已完成 Job） | 2.04ms | 2.10ms | +2.9% |

结论：tool_manifest 数量级改善（#777/#862）；其余绝对差 <2.5ms 属 loopback 抖动；
错误率 0。本轮 observe_jobs 修复新增一次性 `detailed_wake_reason(&completed)` 调用
（O(n) JSON 扫描，n≤8），无可测量开销。

## 10. Cross-platform Validation

| 目标 | 状态 | 证据 |
|---|---|---|
| Linux x86_64（本机） | PASS | §6 全部 |
| Linux aarch64（独立审查环境：aarch64、rustc 1.98.1、/tmp noexec） | **PASS**（含环境修正） | A. `cargo check --locked --workspace --all-targets` PASS；B. monolithic **3236/0/3** PASS；C. runner/lsp 首跑 noexec `/tmp` 34/28 fail → 可执行 TMPDIR 后 **1049/0/4** PASS（noexec 根因本轮已修复，待复验可去掉 TMPDIR）；D. real-process 68/1 → 唯一失败为 node 测试缺 Node.js（本轮已加前置条件，行为已双向验证） |
| Windows x64 / ARM64 | NOT RUN（本机） | 官方 v0.4.6 产物 `webcodex-v0.4.6-win32-{x64,arm64}` 已恢复发布（v0.4.4/0.4.5 曾撤回，#863 修复）；**不等于 fork 已在该平台运行测试** |
| macOS ARM64 / x64 | NOT RUN（本机） | 官方 darwin 产物存在；同上不等于 fork 平台测试 |
| wa1（aarch64 生产机）实测 | NOT RUN | 生产主机禁触碰；建议合入 dev 后由每日审计 cron 补跑 |

**注意**：官方 Release artifacts 存在仅证明上游 CI 对上游代码可编译发布，不构成
fork 在对应平台的运行测试证据。

## 11. Outstanding Risks

1. **aarch64 复验项**：LSP noexec 修复（本轮）与 Node 前置条件需在审查环境复验
   （修复已在本机双向行为验证 + 路径契约测试）。
2. 上游 #775 将部分 executable_tempdir 校准回普通 tempdir——/tmp noexec 主机上若
   上游判断有误会暴露（上溯基线即存在）。
3. `wait_for_job_terminal`/`wait_for_agent_events` 长尾迁移（外部自动化若有硬编码
   直接调用需迁移；已实测可达）。
4. console-nav-requests 前端测试负载下时序 flaky（上溯基线即有，单测稳定）。
5. Windows/macOS 无 fork 运行测试证据（见 §10 注意）。
6. Node 26 本地跑前端测试需 `--localstorage-file`（环境项）。

## 12. Deployment Readiness（预案，不执行）

- **构建**：`cargo build --locked --workspace --bins --profile dogfood`（已验证）或
  `pull_compile_install.sh`（release 路径；本轮已清理 EOF 空行，行为零变化，未真实执行）。
- **预期 Build Identity**：任一 commit 的 `--version` 含其 short SHA 且 `dirty=false`；
  版本号 0.4.6。
- **配置兼容**：无新增必需配置；新能力默认关闭或由 Runner capability 协商。
- **数据/状态**：SQLite 各域由上游迁移路径演进；fork 未改持久层语义。
- **部署前**：最终独立验收 → 合入 dev → wa1 dogfood 先行（deploy_dogfood.sh 自带
  备份/回滚/身份门禁）→ 观察后评 prod。
- **部署后 Smoke**：`deploy_dogfood.sh --url` MCP initialize 冒烟 + `webcodex doctor`
  （信息性）+ `smoke_artifact_transfer.sh` 活动模式（已修复可用）。
- **MCP 客户端**：重连后 `tools/list` 自动刷新；直接调用过下沉工具的自动化需迁移。

## 13. Independent Reviewer Checklist

1. **Repository**：`/home/vincenty/Projects/webcodex`（worktree
   `/home/vincenty/Projects/webcodex-integration-v0.4.6`；远程审查 Runner 上的
   `/home/hermes/src/webcodex` 为另一份 checkout，勿混淆）
2. **Branch**：`integration/upstream-v0.4.6`（`origin` = gitea-ts:vincent/webcodex.git）
3. **最终 SHA**：以 `git ls-remote origin refs/heads/integration/upstream-v0.4.6` 为准
4. **Upstream tag**：`v0.4.6` = `5072608fe74b2bfa6e8f208515c04242f475582f`
5. **复现**：§2.4 命令 + `git diff --stat v0.4.6 HEAD`（fork 覆盖层）+
   `cargo test --locked -p webcodex --lib`
6. **关键审查文件**：`src/tool_runtime/observe_jobs.rs`（遥测+截断修复）、
   `src/lib.rs`（shared-key 单套警告）、`crates/webcodex-lsp/src/test_support.rs`
   （executable_tempdir）、`crates/webcodex-runner/src/webcodex_runner/shell_tests.rs`
   （Node 前置条件）、`scripts/{e2e_job_reconciliation_ws,smoke_artifact_transfer}.sh`
   （TEST BUG 修复）、`deploy_dogfood.sh`/`pull_compile_install.sh`、
   `src/runner_quic.rs`、`crates/webcodex-tool-contracts/src/registry/output_schemas/jobs.rs`
7. **fork 代码路径**：§4 表各行
8. **需复核的冲突**：§3 全部 9 项（重点 main_tests.rs、installer_unix.rs）
9. **安全边界**：§8 所列
10. **残留风险**：§11
11. **日志**：`/home/vincenty/Projects/webcodex-v0.4.6-evidence/`
12. **整改对账**：[`webcodex-v0.4.6-review-fixes.md`](webcodex-v0.4.6-review-fixes.md)

## 14. Final Status

- 分支 HEAD：见 `git log -1`（报告提交后为准）；`v0.4.6 ⊆ HEAD` ancestry 持续成立
- 测试总况：§6.0 口径；本轮整改后全部受影响面复跑通过
- **READY FOR FINAL INDEPENDENT REVIEW**（非 APPROVED FOR PRODUCTION；
  部署许可由用户在独立验收后决定）
