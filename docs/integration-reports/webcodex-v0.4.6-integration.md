# WebCodex Fork 集成审计报告：上游 v0.4.6 升级

- 集成分支：`integration/upstream-v0.4.6`
- Merge Commit：`576a5ef31345b2b0078d8e1af2195b040bbf5680`（最终 HEAD 以报告尾部状态栏为准）
- 报告日期：2026-10-12
- 执行环境：`/home/vincenty/Projects/webcodex-integration-v0.4.6`（独立 worktree；
  任务书路径 `/home/hermes/src/webcodex` 在本机不存在，为异机记录，实际以本仓库为准）
- 执行说明：全部命令经 bash 显式执行（登录 shell 为 fish 的环境约束）

---

## 1. Executive Summary

- **目标完成**：上游 v0.4.6（`5072608f`）全部 74 个提交已通过显式 merge 完整集成
  （`git merge-base --is-ancestor` 证明），fork 与上游的所有差异都经过逐文件审计取舍。
- **fork 自定义功能全部保留**（详见 §4）：Job 观测可靠性遥测、安全加固（远程
  shared-key 边界、供应链 pinning、QUIC 闸门）、部署脚本、文档与测试加固全部存活；
  唯一被上游等价实现取代的是 noexec 测试加固（上游 #775 以更强设计覆盖）与
  aarch64 getpwuid_r 一行修复（上游 #829 等价）。最终 fork 覆盖层 58 文件与设计
  预期闭式对账一致（§8.5）。
- **核心测试全部通过**：monolithic Server 3236/0/3；全 workspace ~7705/0/35（ignored
  均为文档化 real-process 门，其中 runner real-process 69/0 已补跑）；前端 172/172；
  全部 7 个 E2E harness + 上游新 session continuity + MCP conformance 通过。
- **过程中发现并修复 2 个预先存在的 TEST BUG**（与本次合并无关，基线复现证明）：
  reconciliation 场景 C/D 的 compact 回执解析过期（`50fc306d`）、artifact smoke 的
  bash `${2:-{}}` 陷阱 + 稀疏 `git_available` 契约（`580c5181`）。
- **无阻塞**。性能 A/B 显示 tool_manifest 提升 ~20×，其余无系统性退化（§5.7）。
- **建议进入外部独立代码验收**：READY FOR INDEPENDENT REVIEW。
- **不构成生产部署授权**：部署候选资格与时机由用户在独立验收后决定（§7 给出
  部署预案但不执行）。

## 6. Regression / Performance

### 6.2 功能兼容性与安全变化（相对 fork 基线 49ed5263）

- **新增能力**（上游）：交互式 Job 输入（`job_write_input`）、Projects & Resources
  workbench 与资源引用工具、session-first 工作区恢复、Work Result 审查入口、锁定
  依赖策略（`project_dependency_policy_v1`）、durable ArtifactHandoffGrant、
  project_build（Rust/Go）、OAuth admin CLI、Brave 发现、Windows onboarding 恢复、
  有界资源/历史生命周期。
- **工具面变化**：见 §6.1（28→28；4 新增、2 下沉长尾、2 情境化 continuation App）。
- **回执契约变化**（上游，自 nightly 时代已存在）：`run_process`/`cargo_check` 超宽限
  窗交接返回 compact pending 回执（job_id 仅在 continuation）；`show_changes` 稀疏
  工作区状态（`git_available` 仅异常时输出）。既有 MCP 消费者（含 wa1 dogfood 生产
  消费模式"compact 回执从 continuation 拿 job_id"）已兼容。
- **安全边界**：SH-01~14 全部回归通过；无任何认证/授权/沙箱削弱；workflow 供应链
  全部 40-hex SHA pinning；版本元数据与上游一致、fork 以 build identity 区分。

## 7. Deployment Readiness（预案，不执行）

- **构建**：`cargo build --locked --workspace --bins --profile dogfood`（本报告已验证）
  或既有 `pull_compile_install.sh`（release 路径，本报告未真实执行）。
- **预期 Build Identity**：合并后任一 commit 的 `--version` 均含其 short SHA 且
  `dirty=false`（例：本报告测试时为 `580c5181`）；版本号 0.4.6 与官方一致。
- **配置兼容**：无新增必需配置；新能力均有默认关闭或缺省路径
  （`project_dependency_policy_v1` 等 capability 由 Runner 协商）。`host_code_mode`
  与 `X-WebCodex-MCP-Profile`/`X-WebCodex-MCP-Budget-Secs` 为可选部署默认。
- **数据/状态持久化**：SQLite 各域 schema 由上游迁移路径演进；本集成未改持久层
  fork 语义。#809 保留 retention 过期身份。部署前建议按惯例备份 data 目录
  （deploy_dogfood.sh 自带二进制备份；数据目录备份由运维执行）。
- **部署前检查**：独立验收 → 合入 dev → wa1 dogfood 先行（deploy_dogfood.sh 有
  备份/回滚/身份门禁）→ 观察后再评 prod。跨平台残余风险见 §8.11。
- **部署后 Smoke**：`deploy_dogfood.sh --url` 的 MCP initialize 冒烟 +
  `webcodex doctor`（信息性）+ `smoke_artifact_transfer.sh` 活动模式（本报告已
  修复并可作部署后检查）。
- **MCP 客户端重连/Schema 刷新**：MCP 客户端重连后 `tools/list` 自动刷新；直接调用
  过 `wait_for_job_terminal`/`wait_for_agent_events` 的自动化需改经
  `call_runtime_tool`（长尾仍在）或 continuation 回执。

## 8. Independent Reviewer Checklist

1. **Repository path**：`/home/vincenty/Projects/webcodex`（worktree：
   `/home/vincenty/Projects/webcodex-integration-v0.4.6`）
2. **Integration branch**：`integration/upstream-v0.4.6`
3. **最终 Commit SHA**：（以仓库实际 HEAD 为准；本报告主体撰写于 `580c5181`，
   报告提交后为最终 SHA）
4. **Upstream tag**：`v0.4.6` = `5072608fe74b2bfa6e8f208515c04242f475582f`
5. **复现命令**：
   ```bash
   cd /home/vincenty/Projects/webcodex
   git fetch upstream 'refs/tags/v0.4.6:refs/tags/v0.4.6'
   git worktree list                       # integration worktree
   git -C ../webcodex-integration-v0.4.6 log --oneline -4
   git merge-base --is-ancestor v0.4.6 integration/upstream-v0.4.6 && echo INTEGRATED
   git -C ../webcodex-integration-v0.4.6 diff --stat v0.4.6 HEAD   # 58-file fork overlay
   # 重跑核心门：
   git -C ../webcodex-integration-v0.4.6 cargo test --locked -p webcodex --lib
   bash ../webcodex-integration-v0.4.6/scripts/e2e_job_reconciliation_ws.sh
   ```
6. **最关键审查文件/模块**（15）：
   - `src/tool_runtime/observe_jobs.rs`（fork 遥测 vs 上游零冲突的合并点）
   - `src/tool_runtime/runtime_metrics.rs`（JobObservation 指标）
   - `src/tool_runtime/jobs.rs`（上游 −71/+18 重构 × fork returned_bytes 并存）
   - `src/lib.rs`（远程 shared-key 边界告警块）
   - `src/runner_quic.rs`（fork QUIC 侧 × 上游 6 个新 capability 标志）
   - `crates/webcodex-environment/src/installer_unix.rs`（getpwuid_r 取上游等价修复）
   - `crates/webcodex-runner/src/main_tests.rs` + `main_tests/isolated_env.rs`（#775 取代）
   - `crates/webcodex-tool-contracts/src/registry/output_schemas/jobs.rs`（fork schema 域）
   - `scripts/e2e_job_reconciliation_ws.sh`（修复 `50fc306d`）
   - `scripts/smoke_artifact_transfer.sh`（修复 `580c5181`）
   - `deploy_dogfood.sh` / `pull_compile_install.sh`（零变化确认）
   - `Cargo.toml`/`Cargo.lock`/`frontend/package-lock.json`（版本一致性）
   - `.github/workflows/*`（SHA pinning 保留 + 上游新 lane）
   - `docs/agent/job-observation-reliability-testing.md`（playbook 保留）
   - `docs/agent/security-hardening-regression-acceptance.md`（SH 矩阵保留）
7. **fork 定制功能代码路径**：见 §4 表格各行（observe_jobs/runtime_metrics/jobs/
   runner_quic/lib.rs/两部署脚本/两 playbook 文档）。
8. **需独立复核的合并冲突**：§3 全部 9 项；重点抽验 main_tests.rs（EnvGuard→
   IsolatedEnv 取舍）与 installer_unix.rs（等价修复取舍）。
9. **必须检查的安全边界**：远程 shared-key 默认拒绝 + 显式 opt-in（`src/lib.rs`
   告警 + `runner_quic` 测试）、OAuth scope、Config redaction（`config::tests`）、
   workflow SHA pinning（`test_workflow_security.py`）、Git snapshot 隔离
   （`git_diff_hunks_`/`git_review_summary_` 分片）、QUIC 闸门（`runner_quic` 分片）。
10. **可能残留的技术风险**：
    - aarch64 原生测试未在本任务执行（wa1 为生产机）；上游官方 linux-arm64 产物 +
      installer 字节一致 + fork delta arch-neutral 为替代证据，建议合入后由每日
      审计 cron 在 wa1 补跑。
    - 上游 #775 将部分 executable_tempdir 校准回普通 tempdir（fixture 不真 spawn 的
      判断）；在 /tmp noexec 的主机上若上游判断有误会暴露，wa1 正是 noexec /tmp。
    - `wait_for_job_terminal` 等已从直接面下沉（§6.1），外部自动化若有硬编码直接
      调用需迁移（长尾仍可用，已实测）。
    - Node 26 本地跑前端测试需 `--localstorage-file`（环境项，非产品问题）。
    - console-nav-requests 前端测试存在负载下时序 flaky（上溯至基线即有，单测稳定）。
11. **测试日志与证据目录**：`/home/vincenty/Projects/webcodex-v0.4.6-evidence/`
    （monolithic-test.log、shards-test.log、runner-realprocess.log、
    process-shell-lanes.log、focused-shards.log、e2e-lanes.log、
    e2e-reconciliation-{rerun,fixed}.log、e2e-session-continuity.log、
    mcp-conformance.log、tools-before/after.json、perf-before/after.json、
    PROGRESS.md、aarch64-check.log）。
12. **尚未执行的测试与原因**：
    - aarch64 原生测试：交叉 C 工具链缺失（cc-rs 找不到 aarch64-linux-gnu-gcc，
      aarch64-check.log）；wa1 为生产机禁触碰。
    - Windows/macOS 实测：无对应环境；以上游官方 v0.4.6 全平台产物为编译级证据。
    - `pull_compile_install.sh` 真实安装 / `deploy_dogfood.sh` 真实部署：任务明令
      禁止（dry-run fixture 已验证）。
    - Desktop（Tauri）端到端：上游 release 产物含全平台 Desktop 包；本机未构建
      Desktop（跨平台 native 依赖），CI lane 为准。

---

## 状态栏（最终）

- Integration branch：`integration/upstream-v0.4.6`
- 最终 Commit（含本报告）：本报告提交后的分支 HEAD，即 `git -C /home/vincenty/Projects/webcodex-integration-v0.4.6 log -1 --format=%H`（报告主体测试证据对应 `580c5181`，其后仅追加本报告，无代码变更）
- 上游集成证明：`v0.4.6 (5072608f) ⊆ HEAD`（ancestry）
- 测试总况：~7705 Rust + 380 node + 172 前端 + 7 E2E harness + conformance 全绿；
  唯一 flaky（console-nav-requests）与 2 个已修复 TEST BUG 均如实披露
- READY FOR INDEPENDENT REVIEW（非生产部署授权）

## 2. Git Integration Evidence

### 2.1 基线（2026-10-12 实测核实，非任务书转述）

| 引用 | 实测 SHA | 说明 |
|---|---|---|
| `origin/dev` = 本地 `dev` | `49ed52633dac63c697c02ca9c9a29d8eec1d07c4` | 与远端同步，clean；集成分支基线 |
| `prod`（本地 = origin） | `d1636f72` | 任务书声称 59adb1dd，实测不符（59adb1dd 为 2026-09-28 审计时 dev 状态） |
| `upstream` tag `v0.4.6` | `5072608fe74b2bfa6e8f208515c04242f475582f` | annotated tag，tagger yyjeqhc，fetch 自 github.com/yyjeqhc/webcodex |
| merge-base(dev, v0.4.6) | `7301186b98527cb4ebc0191f6033474e9abf7c20` | #764（2026-09-28）= fork nightly 集成 cutoff |

### 2.2 差异结构

- `git rev-list --left-right --count origin/dev...v0.4.6` → `117  74`
  - fork 侧独有 117 commits（安全加固、Job 观测可靠性、部署脚本、nightly 适配修复）
  - 上游侧新增 74 commits（v0.4.4 + v0.4.5 + v0.4.6 三个 release 的全部变更）
- patch-equivalence：`git log --cherry-mark` → **0 个等价补丁对**（无双补丁重复）
- fork 曾按 SHA merge 的全部 ~90 个上游 commit 逐一验证均在 v0.4.6 祖先内
- 上游 diff：933 文件 +131923/−70164；fork 侧独有 diff：65 文件 +1737/−266
- 上游 diff 结构：1 rename（`webcodex-core/build.rs` → `webcodex-build-info/build.rs`，#769
  build identity 隔离）、0 delete、380 create

### 2.3 Upstream Change Inventory（74 commits，按 release 分组）

- **v0.4.4 线（#765–#829 主体）**：model 执行/校验投影收敛（#765/#767）、project recipe
  真相集中（#766/#798）、runtime bootstrap/Job UX（#768）、build identity 与最终结果快照
  隔离（#769）、typed job 查询与响应投影拆分（#770）、runtime 所有权/观测生命周期
  （#771）、tool surface policy 显式化 + 不活跃 continuation 下沉（#772）、Session ledger
  索引与写合并（#773）、输入规范化可测量性（#774）、**环境 fixture 隔离（#775）**、
  工具契约元数据先选（#777，发现性能）、Console 查询/审计投影拆分（#778）、Tunnel 恢复
  与 macOS 签名（#779）、Session/AgentTask selector 契约（#781）、trace 证据检查
  （#782）、bash login durable job（#785）、scoped native test filters（#786）、timeout
  terminal attention 规范化（#787）、Browser 有界批量表单（#790）、project_build Rust/Go
  （#789）、全局指令热载（#788）、跨平台个人服务生命周期（#793）、Browser/Window 活动
  路径性能（#791）、Runtime 刷新竞态（#795）、OAuth admin CLI + renameat2（#797）、Cargo
  workspace provenance（#799）、**runtime/Runner 模块职责拆分（#801）**、Desktop 工作流
  布局（#796）、客户端契约对齐（#802）、**交互式 Job 输入（#803）**、wake 审批门控
  （#804）、session 上下文恢复与 model API 评审（#800）、CI fixture 可移植（#805/#837）、
  **v0.4.4 发布（#806/#829）**、Go 隔离（#807）、project op scope（#808）、retention 过期
  身份（#809）、**锁定依赖策略（#810）**、**durable ArtifactHandoffGrant（#811/#813）**、
  任务完成反馈（#812）、Window 协作连续性（#814）、general-agent 指引（#815）、tray 本地化
  （#824）、Brave 发现（#825）、Runtime 切换 Runner 身份（#827）
- **v0.4.5 线**：Work Result 审查入口（#828）、session-first 工作区恢复（#831）、
  **Projects & Resources workbench（#832）**、webui 布局（#833）、MCP occupied provider
  连接等待（#834）、统一安装器与恢复（e98ab112）、Windows SCM SID 所有权（33ef276f）、
  macOS 自签身份（4b329f3f）、Intel Desktop 源身份（168f66a3）、v0.4.5 发布（fe5a6d3e）、
  macOS Bash 3.2 签名（a4ece582）
- **v0.4.6 线（backport）**：**runtime 读取路径优化（#862）**、**Windows onboarding 与
  本地文件夹恢复（#863）**、**Desktop 本地工作流恢复（#865）**、**有界资源与历史生命
  周期（#866）**、macOS review snapshot index（abe734d7）、Windows recipe 路径（4fcac8b5）、
  macOS 危险 temp root（2d8fcc0a）、stale allowed roots（5a805f93）、未确认 project 刷新
  状态（5072608f）

### 2.4 集成方式与证明

- 方式：在独立 worktree 中 `git merge --no-ff v0.4.6`（显式保留可审计历史）
- 集成完整性证明：`git merge-base --is-ancestor v0.4.6 576a5ef3` → 通过。
  74 个上游 commit **全部**通过 Git ancestry 纳入，不依赖 patch-equivalence 推断
- 独立复核命令：
  ```bash
  cd /home/vincenty/Projects/webcodex && git fetch upstream 'refs/tags/v0.4.6:refs/tags/v0.4.6' 2>/dev/null
  git rev-parse v0.4.6^{commit}            # 5072608fe74b2bfa6e8f208515c04242f475582f
  git merge-base origin/dev v0.4.6          # 7301186b98527cb4ebc0191f6033474e9abf7c20
  git merge-base --is-ancestor v0.4.6 integration/upstream-v0.4.6 && echo INTEGRATED
  git log --oneline integration/upstream-v0.4.6 -3
  ```

## 3. Conflict Resolution Report

合并冲突共 9 个文件，全部逐文件核实两侧提交来源后解决（无批量 ours/theirs）：

| 文件/模块 | 上游更改 | Fork 更改 | 解决方法 | 验证证据 |
|---|---|---|---|---|
| `crates/webcodex-environment/src/installer_unix.rs` | #829 携带的 getpwuid_r 缓冲 arch-neutral 修复 `vec![0 as libc::c_char; 16384]` | 49ed5263 的等价修复 `vec![0u8; 16384]`（aarch64 c_char=u8） | 取上游（语义等价，上游为规范形式） | 该文件与 v0.4.6 逐字节一致；`cargo check`/全套测试通过；aarch64 交叉检查见测试矩阵 |
| `crates/webcodex-runner/src/main_tests.rs` | #775：拆出 `main_tests/isolated_env.rs`（`IsolatedEnv` 子进程级环境隔离断言）+ 原生 `executable_tempdir` + `TEST_ENV_LOCK` | 7a951b58：内联 `executable_tempdir`（noexec 主机加固）+ 内联 `EnvGuard` | 取上游（含等价 executable_tempdir；EnvGuard 被 IsolatedEnv 取代——更强：环境在子进程启动前注入，libtest 进程零 env 变更） | runner/lsp 分片 1049/0；real-process 组 69/0；冲突文件与 v0.4.6 一致 |
| `main_tests/shell_config.rs` | #775：环境 fixture 隔离；部分 tempdir 有意回退为 `tempfile::tempdir()`（该文件 fixture 只做 PATH 发现/解析，不真正 spawn） | 7a951b58：全部改 `executable_tempdir` | 取上游（上游判断有依据；本机 /tmp 无 noexec 实测通过；wa1 noexec 为披露的跨平台项） | runner 分片含 shell_config 全部测试通过 |
| `main_tests/shell_profiles.rs` | #775（同上） | 7a951b58 | 取上游 | 同上；sshd POSIX 门控（7c7f4947）位于 `ssh.rs` 未受影响 |
| `webcodex_runner/coding_agent.rs` | #801 模块职责拆分 + #775 | 7a951b58 | 取上游 | runner 分片通过 |
| `webcodex_runner/mcp_gateway_tests.rs` | #834 occupied provider 连接等待 + #775 | 7a951b58 | 取上游 | runner 分片 + mcp_gateway 测试通过 |
| `webcodex_runner/plugin_tests.rs` | #775 | 7a951b58 | 取上游 | runner分片通过 |
| `webcodex_runner/shell_tests.rs` | #775 | 7a951b58（另含 2 个 fork 侧 merge commit，其目标 SHA 均已验证在 v0.4.6 祖先内） | 取上游（无内容可回退） | runner 分片通过 |
| `webcodex_runner/validation/execute.rs` | #775 | 7a951b58 | 取上游 | runner 分片通过 |

冲突解决共性依据：fork 侧在这些文件的改动**仅**来自 7a951b58（noexec 临时目录加固），
上游 #775 以更强设计（IsolatedEnv + 校准后的 executable_tempdir 使用面）实现同一目标；
且 fork 曾 merge 的所有上游 SHA 都在 v0.4.6 内，取上游侧不可能回退任何 fork 侧上游内容。
无"等价实现而不采用上游代码"需单独记录的条目（除 installer_unix 一行，两实现等价取上游）。

## 4. Fork Feature Preservation

fork 独有 diff 共 65 文件（+1737/−266），逐域核对：

| Fork 功能 | 来源 commits | 集成状态 | 依据/测试证据 |
|---|---|---|---|
| **Job 观测可靠性遥测**：`observe_jobs` wake reason 细化（`wait.reason/wake_on/requested_secs`）、`trace_observe_jobs_diagnostics`（revision/cursor 前后追踪）、`stdout/stderr_returned_bytes`、`job_observation_*` 指标 | 5165761f, a53b5599, d3578048, 7170a070 | **Retained**（上游 base→v0.4.6 完全未触碰 observe_jobs.rs/runtime_metrics.rs；jobs.rs 上游重构 −71/+18 与 fork +2 行自动并存） | H1 分片 87 tests + tool-contracts 全量 272 + monolithic 3236 全过；`observe_jobs` 描述 331ch（<1024 预算） |
| **安全加固**：远程 shared-key 默认拒绝 + 显式 opt-in、QUIC 闸门、Config redaction、workflow 供应链 SHA pinning、 hardened temp hosts | bcecfdee, 7dc7bb03, 921d415c, f990759b | **Retained** | SH 分片：mcp 316+1i / runner_quic 13 / share_service 18 / config 22 / search_ 64 / cloudflared 11 / show_changes 73 / git_diff_hunks 38 / git_review_summary 13 全过；workflows 全部 40-hex SHA-pinned；`test_workflow_security.py` 2/2、`test_release_readiness` 23/23 |
| **部署能力**：`deploy_dogfood.sh`（备份→停单元→构建→身份门禁→起单元→冒烟→自动回滚）、`pull_compile_install.sh`（ff-only + release 构建 + build identity 断言 + 安装 + doctor） | 706db7ac, 590ff4f8, bda8a619, a90df0d2, cd3f2e13, cde4f77e, d962a0d6 | **Retained**（两脚本与 origin/dev 逐字节一致，零合并影响） | `bash -n` 通过；`--dry-run` fixture 实测（正确读 576a5ef3、无单元安全跳过、零副作用、git 状态不变）；真实安装被任务禁止未执行（脚本在 49ed5263 已有 wa1 七轮实战史） |
| **测试环境加固**：sshd POSIX login shell 门控、fake runner stub 原子发布、noexec temp hosts | 7c7f4947, 447e047b, 7a951b58 | **Replaced with equivalent upstream**（#775 以 IsolatedEnv + 原生 executable_tempdir 覆盖 noexec 项） | `ssh.rs` 门控保留（grep 7 处）；runner 分片 + real-process 69/0 |
| **Nightly 集成适配**（auth 测试收敛上游单策略、aarch64 getpwuid_r） | e03a4033, 7d346b11, 30d7725c, b87c3776, ba15c8f6, 49ed5263 | **Superseded/Retained**（49ed5263 被 #829 等价实现取代；其余自动并入） | auth 相关测试在 monolithic 3236 中通过；installer_unix 取上游等价修复 |
| **文档**：job-observation playbook、security-hardening playbook、prod-deploy-report、DEPLOYMENT 等 | f990759b, 59adb1dd, d1636f72 等 | **Retained**（playbook 引用在 TESTING.md 3 处保留；上游新测试通道并入） | TESTING.md 合并 diff 复核 |
| **CLI/探针微调**：connect/probe.rs、init.rs 测试 | 49ed5263 等 | **Retained**（自动并入） | cli 测试在 remainder 分片通过 |

有意弃用/上游取代项（非丢失）：
- `EnvGuard`（runner 内联版）→ 上游 `IsolatedEnv`（更强：进程级隔离）
- fork 内联 `executable_tempdir` → 上游原生同义实现
- 49ed5263 aarch64 缓冲修复 → 上游 #829 等价修复

## 5. Testing Report

环境：Linux x86_64（CachyOS，kernel 6.18.55-2-cachyos-lts），20 核，rustc 1.99.0，
Node v26.11.1，python 3.14。所有命令在 integration worktree 执行（除标注 baseline
对照）。原始日志：`/home/vincenty/Projects/webcodex-v0.4.6-evidence/`（不进 Git）。

### 5.1 静态与构建

| 测试 | 命令 | 状态 | 结果 | 日志 |
|---|---|---|---|---|
| Rust 格式 | `cargo fmt --all -- --check` | PASS | 0 diff | — |
| 全 workspace 编译（含测试目标） | `cargo check --locked --workspace --all-targets` | PASS | 57.21s，2 个上游固有 warning（见 5.5） | 会话记录 |
| Clippy | `cargo clippy --locked --workspace --all-targets` | PASS | RC=0（风格 lint 为上游固有） | — |
| dogfood 二进制 | `cargo build --locked --workspace --bins --profile dogfood` | PASS | server/runner/cli 全产出 | — |
| 前端安装 | `npm ci`（frontend/） | PASS | | |
| 前端类型 | `npm run typecheck` | PASS | | |
| 前端测试 | `NODE_OPTIONS=--localstorage-file=… npm test` | PASS（见 5.5 flaky 注记） | 26 文件 / 172 测试 | 会话记录 |
| 前端构建 | `npm run build` | PASS | dist 1251.65KB(gzip 305KB) | |
| 生成物校验 | `npm run check:dist` | PASS | 重建零 drift（git status 0 变更） | |
| 空白字符 | `git diff --check` | PASS | | |
| 供应链 | `python3 scripts/tests/test_workflow_security.py` | PASS | 2/2 | |
| 发布就绪契约 | `python3 -m scripts.tests.test_release_readiness` | PASS | 23/23 | |

### 5.2 Rust 测试

| 测试 | 命令 | 状态 | 通过/失败/跳过 | 日志 |
|---|---|---|---|---|
| **monolithic Server** | `cargo test --locked -p webcodex --lib` | PASS | **3236 / 0 / 3**（55.89s） | monolithic-test.log |
| Runner+LSP 分片 | `cargo test --locked -p webcodex-runner -p webcodex-lsp` | PASS | 1049 / 0 / 4 | shards-test.log |
| remainder 分片 | `cargo test --locked --workspace --exclude webcodex --exclude webcodex-runner --exclude webcodex-lsp` | PASS | 2423 / 0 / 28 | shards-test.log |
| Runner real-process | `cargo test --locked -p webcodex-runner -p webcodex-lsp --features runner-real-process-tests runner_real_process -- --ignored --test-threads=1` | PASS | 69 / 0 / 0（45.18s 串行） | runner-realprocess.log |
| ManagedChild 边界 | `cargo test --locked -p webcodex-process --test managed_child -- --ignored --test-threads=1` | PASS | 15 / 0 / 0 | process-shell-lanes.log |
| 持久 shell 边界 | `cargo test --locked -p webcodex-persistent-shell -- --ignored --test-threads=1` | PASS | 6 / 0 / 0 | process-shell-lanes.log |

ignored 项均为文档化的 real-process/慢速门（TESTING.md 契约），非跳过验证。

### 5.3 fork 专项（playbook 分片）

| 测试 | 状态 | 计数 | 备注 |
|---|---|---|---|
| H1 `observe_jobs_wake_policy`（tool-contracts） | PASS | 2 | |
| H1 `job_log_wait`（runner-registry） | PASS | 19 | |
| H1 `heartbeat`（runner） | PASS | 4 | |
| H1 `observe_jobs`（webcodex） | PASS | 62 | |
| H1 heartbeat 边缘 4 项 | PASS | 1+1+1+2 | 3 个 playbook 过滤名过期：`heartbeat_batch_advances_all_runner_active_jobs_and_skips_terminal_jobs`→上游更名 `heartbeat_batch_advances_only_runner_owned_active_lifecycles`（origin/dev 同名，先于本次集成）；terminal 策略由 `observe_jobs_terminal_policy_coalesces_*`/`observe_jobs_all_terminal_*` 覆盖 |
| H1 tool-contracts 全量 | PASS | 272 | |
| H3 `mcp::tests::` | PASS | 316 / 1 ignored | SH-01 |
| H3 `runner_quic` | PASS | 13 | SH-02 |
| H3 `project_entry::share_service::tests::` | PASS | 18 | SH-03 |
| H3 `config::tests::` | PASS | 22 | SH-04 |
| H3 `tool_runtime::tests::files::search_` | PASS | 64 | SH-05 |
| H3 `project_entry::cloudflared_service::tests::` | PASS | 11 | SH-06 |
| H3 `show_changes_` | PASS | 73 | SH-07 |
| H3 `git_diff_hunks_` | PASS | 38 | SH-08~12 |
| H3 `git_review_summary_` | PASS | 13 | SH-09~13 |
| SH-14 跨测试污染 | PASS | — | 由 monolithic 3236/0 证明 |
| H5 deploy_dogfood.sh | PASS | dry-run fixture | 见 §4；真实安装未执行（禁止） |
| H2 工具清单捕获 | PASS | 28 工具×2 侧 | tools-before/after.json |

### 5.4 E2E / 集成

| 测试 | 状态 | 备注 | 日志 |
|---|---|---|---|
| e2e_reconnect_ws.sh | PASS | 重启重连连续性 | e2e-lanes.log |
| e2e_job_reconciliation_ws.sh | **PASS（修复后）** | 场景 A-D 全绿；原始失败为 harness 过期断言（见 5.5） | e2e-reconciliation-fixed.log |
| e2e_job_recovery_failures_ws.sh | PASS | C/D/E/F 场景 | e2e-lanes.log |
| e2e_hosted_connect.sh | PASS | 跨 key 隔离、PID 复用 | e2e-lanes.log |
| e2e_shared_key_ws.sh | PASS | | e2e-lanes.log |
| smoke_artifact_transfer.sh（活动模式） | **PASS（修复后）** | 27/27（见 5.5） | 会话记录 |
| e2e_zero_config_ws.sh | PASS | 全表面（GPT Actions + MCP + Agent） | e2e-lanes.log |
| e2e_session_continuity_ws.py（上游新增） | PASS | 4/4：发现/恢复/权限、双协议 SSE roundtrip、封闭历史 | e2e-session-continuity.log |
| MCP conformance（2025-11-25 profile） | PASS | 33 场景全按 pinned 基线分类（24 个预期 FAILURE 为老 spec 有意拒绝；missing=0 unexpected=0 problems=[]）；server_sha=580c5181 | mcp-conformance.log |
| MCP App DOM（node --test） | PASS | 380/380 | |

### 5.5 原始失败、定性、修复（如实记录）

1. **e2e_job_reconciliation_ws 场景 C/D 失败 ×2 次**（同签名复现）：
   - 第一因果：脚本读顶层 `output.job_id`，但 `run_process`/`cargo_check` 交接回执自
     nightly 时代起即为 compact pending 形态（job_id 仅在
     `continuation.arguments.items[0]`）。**基线 49ed5263 二进制同 payload 复现相同
     回执 → 定性 TEST BUG（预先存在），非合并回归。**
   - 修复：`50fc306d`（顶层优先 + continuation 兜底，兼容两代构建）。修复后全场景 PASS。
2. **smoke_artifact_transfer 活动模式 4 项失败**：
   - 第一因果①：三处 helper 使用 `${2:-{}}`，bash 花括号歧义解析给所有已传值追加
     字面量 `}`（`{}`→`{}}`），全部 JSON body 被服务端严格解析拒绝。e2e_zero_config
     的 api_post 早已注释记载同一陷阱，smoke 脚本漏修。**TEST BUG（预先存在）**。
   - 第一因果②：`git_available` 断言要求恒真字段，但现行稀疏契约仅在 false 时输出
     （coding_tasks.rs:1236 schema 明文"omission means no exceptional Git-unavailable
     fact"）。**TEST BUG（契约过期）**。
   - 修复：`580c5181`。修复后活动模式 27/27 PASS。
3. **前端 npm test 首轮 172 全挂**：Node v26.11.1 的 localStorage 需
   `--localstorage-file`（fork 2026-09-26 已知环境项；上游 CI 用 Node 22）。
   **ENVIRONMENT**，加 flag 后 171/172；余 1 例
   （console-nav-requests projects 视图 5vs8 轮询计数）定性 **FLAKY**（单测复跑 3/3
   过 + 全量复跑 26 文件 172/172 全过）。
4. **cargo check 2 个 warning**：`job_receipts.rs` fetch_update 弃用 +
   `trace_diagnostics.rs` unused import；两文件与 v0.4.6 逐字节一致，为 rustc 1.99
   对上游既有代码的新告警。**NOT A REGRESSION**，未改动以避免与上游无谓分叉。
5. **perf harness 迭代中 4 次 400**（read_files items≠reads、search queries≠pattern、
   run_process 无 wait_secs/拒 shell 模式）：harness 参数对齐现行 schema 后解决；
   侧面证实了 strict schema 契约在合并后正确执行。

### 5.6 跨平台

| 目标 | 状态 | 证据 |
|---|---|---|
| Linux x86_64（本机） | PASS | 全部上述测试 |
| Linux aarch64（本机交叉 check） | BLOCKED（环境） | `cargo check --target aarch64-unknown-linux-gnu` 于 cc-rs 处失败：无 `aarch64-linux-gnu-gcc` 交叉 C 工具链（aarch64-check.log）；非代码错误 |
| Linux aarch64（上游证据） | PASS（上游 CI） | 官方 v0.4.6 release 含 `webcodex-v0.4.6-linux-arm64.tar.gz`（manylinux2014_aarch64 原生构建）；`installer_unix.rs` 与上游逐字节一致；fork 其余 delta 为 arch-neutral Rust |
| Windows x64 / ARM64 | NOT RUN（本机） | 无 Windows 环境；官方产物 `webcodex-v0.4.6-win32-{x64,arm64}` 已恢复发布（v0.4.4/0.4.5 曾撤回，v0.4.6 修复后回归，#863/#837） |
| macOS ARM64 / x64 | NOT RUN（本机） | 无 macOS 环境；官方产物 darwin-{arm64,x64} 存在（含 #837 Intel 源身份回填） |
| wa1（aarch64 生产机）实测 | NOT RUN | 生产主机，任务禁止部署/触碰；建议合入 dev 后由每日 07:40 审计 cron 在 wa1 补跑（该流程既有） |

**Linux 测试通过不等价于所有平台通过**：Windows/macOS/wa1 实测如上标注，残余风险见 §8.10。

### 5.7 性能 A/B（Phase K）

同机、同 dogfood profile、同 fixture（300 文件 × ~2KB，40 处 needle）、disposable
loopback server+runner，逐调用计时：

| case | 基线 49ed5263 median | 集成后 median | Δ | 响应字节 |
|---|---|---|---|---|
| tools/list | 52.25ms | 53.63ms | +2.6% | 53159→51404（更小） |
| tool_manifest | 52.74ms | **2.67ms** | **−94.9%** | 14992→14076 |
| runtime_status | 0.93ms | 1.14ms | +0.21ms | 766→766 |
| read_files 单文件 | 1.79ms | 2.11ms | +0.32ms | 3046→3045 |
| read_files 8 文件 | 43.91ms | 46.25ms | +2.3ms | 22877→22869 |
| search_project_texts | 58.76ms | 59.21ms | +0.8% | 4303→4305 |
| observe_jobs（已完成 Job） | 2.04ms | 2.10ms | +2.9% | 790→793 |

结论：**tool_manifest 数量级改善**（上游 #777/#862 读取路径优化的直接效果）；其余
绝对差均 <2.5ms，处于 loopback 单请求抖动范围，未见系统性退化；错误率 0。p95 与
median 同序（tools/list p95 66ms 属偶发调度尾延迟，中位稳定）。


## 6. Regression / Performance

（工具面 diff 已测得，性能数据待 A/B 完成）

### 6.1 MCP Tool Manifest Diff（disposable 实测）

对比：49ed5263（升级前）vs 集成后（同机、同 dogfood profile、disposable loopback）：

- 直接工具数：28 → 28（不变）
- **新增 4**：`job_write_input`（#803 交互式 Job 输入）、`open_webcodex_workbench`、
  `read_webcodex_resource`、`search_webcodex_resources`（#832 Projects & Resources workbench）
- **不再静态列出 4**：
  - `wait_for_job_terminal`、`wait_for_agent_events` → **仍在 runtime 长尾可经
    `call_runtime_tool` 调用**（实测返回参数级错误 = 工具存在），迁移路径完好
  - `present_agent_continuation`、`present_job_terminal_continuation` → 上游 b7f56bad
    设计为情境化 continuation App（内部引用保留于 presentation/discovery/model_surface，
    仅在 continuation 活跃时呈现），#828/#832 的新模型（present_work_result + workbench）
    承接等价能力
- 9 个工具描述/schema 有上游调优（如 run_shell 242→398ch、observe_jobs schema 微缩
  63B，源自 #777 元数据先选优化）
- 兼容性风险：低。MCP 客户端静态面无破坏（数量不变）；被下沉工具的调用方需从直接名
  迁移到 `call_runtime_tool` 或 continuation 回执（fork 生产消费者 compact 回执已从
  continuation 拿 job_id，模式一致）

## 7. Deployment Readiness

（收尾时填充；本报告不构成部署授权）

## 8. Independent Reviewer Checklist

（收尾时填充）
