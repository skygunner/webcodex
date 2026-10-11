# WebCodex v0.4.6 独立验收整改对账报告

- 日期：2026-10-12
- 原始 integration SHA（验收基线）：`02c41df8771995d198fd3134ba6b6e2ec3c5dc3c`
- 最终 SHA：以 `git ls-remote origin refs/heads/integration/upstream-v0.4.6` 为准
  （本报告主体证据对应的最后代码 commit 为 observe_jobs 修复 commit，其后仅文档提交）
- 分支：`integration/upstream-v0.4.6`；worktree：
  `/home/vincenty/Projects/webcodex-integration-v0.4.6`
- Upstream v0.4.6：`5072608fe74b2bfa6e8f208515c04242f475582f`
- 本轮 commits（按序）：
  1. `fix(runtime): preserve observe_jobs wake reason across budget truncation`
  2. `fix(server): deduplicate remote shared-key exposure warnings`
  3. `test(lsp): build executable fixtures on the executable target filesystem`
  4. `test(runner): fail loudly when Node.js is missing for the real-process smoke`
  5. `style(deploy): remove trailing blank line from pull_compile_install.sh`
  6. `docs(integration): restructure v0.4.6 report and add review-fixes evidence`

## B. 整改逐项对账

| # | 问题 | 根因 | 修改文件 | 新测试 | 最终状态 |
|---|---|---|---|---|---|
| P2 | observe_jobs 批量输出预算截断时 `wait.reason` 可能失真 | `apply_output_budget` 收尾 `batch_output` 用截断后的当页子集重新计算 detailed wake reason | `src/tool_runtime/observe_jobs.rs` | `wake_reason_survives_output_budget_truncation`、`wake_reason_survives_oversized_item_replacement` | **FIXED** |
| P3-a | `src/lib.rs` 两段远程 shared-key 警告在相同边界下重复输出 | fork 段（v0.4.3 时代）与上游 #813 时代的新段条件完全等价并存 | `src/lib.rs` | 行为验证（4 配置实测；无日志测试框架，见 D 节） | **FIXED** |
| P3-b | LSP 测试在 /tmp noexec 主机 28 failed | fake LSP server 编译进 `tempfile::tempdir()`（TMPDIR）；另有 hard-link 到 /tmp 再 spawn 的第二依赖点 | `crates/webcodex-lsp/src/test_support.rs`、`tests.rs` | `fake_server_fixture_is_built_under_the_executable_target_root` | **FIXED**（待 aarch64 复验） |
| P3-c | node real-process 测试缺 Node.js 时误报 NotStarted 为功能失败 | 测试直接依赖 PATH 上的 Node.js，无前置探测 | `crates/webcodex-runner/src/webcodex_runner/shell_tests.rs` | 前置探测内建于原测试（双向行为验证） | **FIXED** |
| P4 | `pull_compile_install.sh` EOF 空行（`git diff --check v0.4.6` 唯一违例） | fork 覆盖层遗留 | `pull_compile_install.sh` | — | **FIXED** |
| 文档 | 报告章节重复（6/7/8 各两次）、占位残留、计数口径、ARM64 证据 | 初稿分两次写入导致骨架残留 | `docs/integration-reports/webcodex-v0.4.6-integration.md` | — | **FIXED** |

无 NOT REPRODUCIBLE / BLOCKED / DEFERRED 项。

## C. observe_jobs 证明（P2）

- **修复前失败的回归证据**（commit 前实测，原始输出）：
  ```
  test ...wake_reason_survives_output_budget_truncation ... FAILED
  assertion `left == right` failed: wait.reason must describe the full-observation
  wake event, not the truncated page
    left: String("change")        ← 截断页计算的错误值
    right: "meaningful_change"    ← 完整观察集的真实值
  ```
- **根因**：`WakeReason::Updated` 的细化依赖 `detailed_wake_reason(items)` 扫描
  items 的 `meaningful_changed`/`heartbeat_changed`。`apply_output_budget` 的收尾
  `batch_output(returned, …)` 传入的是预算截断后的当页子集；真实唤醒的 Job 因预算
  不在当页时，`meaningful_change` 被降级为 `change`/`heartbeat`。
  `wait.outcome`/`wake_on`/`requested_secs`/`waited_ms` 一直正确（来自完整集合/
  原始请求），只有 `wait.reason` 受影响；metrics 与 trace 本就使用完整集合
  （`detailed_wake_reason(&completed, …)`），与 canonical truth 一致，无需改动。
- **修复**：`apply_output_budget` 在任何截断前对完整 `completed` 集合计算一次
  detailed reason，作为参数传入全部 4 处 `batch_output`；`batch_output` 不再自行
  计算。`output_truncated`、`next_index`、continuation（`suggested_call`）行为
  零变化——只修正 `wait.reason` 的事实来源。
- **不退化证据**：`observe_jobs` 64 + `tool_runtime::tests::jobs` 43 +
  `job_log_wait` 19 + tool-contracts 全量 272 = **398/0**；现有预算测试
  （`output_budget_replaces_one_oversized_item…`、`output_budget_keeps_whole_prefix…`）
  无一改动、原样通过。
- **Schema 一致性**：jobs.rs schema 的 `wait.reason` 是 batch 级 enum
  （"The single shared-wait fact for this batch"），枚举值不变；修复使实现符合
  该既有描述，schema 零修改。
- **性能影响**：每个 observe_jobs 调用新增一次 O(n≤8) 的 JSON 布尔字段扫描；
  §9 A/B 显示 observe_jobs 中位 2.10ms（基线 2.04ms），无可测量开销。

## D. Shared-key 警告去重（P3-a）

- **删除**：fork 段 `if crate::auth::shared_key_enabled() { … }` 整块
  （`tracing::warn!` ×2）。该段与上游段外层条件（`enabled && requires_remote_opt_in`）
  及内部分支（`remote_enabled` 二分）**完全语义等价**，无一方覆盖额外边界。
- **保留**：上游 v0.4.6 单套警告（结构、条件、认证谓词零修改），并合并 fork 段的
  独有价值安全信息：exposed 分支补 "any unknown non-wc_* Bearer becomes a
  shared-key principal. Prefer managed PAT/OAuth/Runner credentials…"；disabled
  分支补 "or use managed PAT/OAuth/Runner credentials"。
- **行为验证**（一次性 server，remote boundary 由 `WEBCODEX_PUBLIC_URL` 触发）：
  | 配置 | boundary 警告条数 | 旧 fork 文本条数 |
  |---|---|---|
  | shared-key + remote + REMOTE_ENABLED 未设 | 1 | 0 |
  | shared-key + remote + REMOTE_ENABLED=true | 1 | 0 |
  | shared-key + local-only（127.0.0.1） | 0 | 0 |
- **未触碰**：`shared_key_enabled()`、`shared_key_remote_enabled()`、
  `shared_key_requires_remote_opt_in()`、认证逻辑、HTTP Host/Origin、QUIC gate、
  OAuth 策略、managed credential 检查——diff 仅删一个警告块 + 改两段文本。
- **回归**：runner_quic 13 + config 22 + oauth 297 + mcp 316 + share_service 18
  = 666/0。未加日志断言测试的说明：`run_server_with_shutdown` 无单测日志框架，
  现有测试均不捕获启动日志；引入仅为此断言的全局日志捕获框架超出整改最小面，
  以 4 配置行为实测 + diff 为证据（重复可能性已随 fork 块删除而物理消失）。

## E. LSP noexec /tmp（P3-b）

- **原始失败原因**：`test_support.rs::fake_server_path()` 用 `tempfile::tempdir()`
  （= TMPDIR，默认 /tmp）编译并执行 fake LSP server 二进制 → noexec 主机 spawn
  全部 `Permission denied (os error 13)`（审查环境 28 failed）。第二依赖点：
  `tests.rs` 的 spaced-program fixture 将 fake server 硬链接进 /tmp tempdir 再
  spawn（同文件系统时成功、跨设备时 EXDEV）。
- **最终方案**（修改 LSP 测试源码，仿上游 #775 的 runner 模式）：
  1. `test_support.rs` 新增 `executable_tempdir()`：在
     `CARGO_MANIFEST_DIR/../../target/test-executables` 下创建唯一 tempdir；
  2. `fake_server_path()` 改用它（OnceLock 单例/自动清理/并发唯一语义不变）；
  3. spaced fixture 改用它（同时消除 EXDEV 与 noexec spawn 两个问题）；
  4. 其余数据型 tempdir（workspace/project/marker/PATH 解析）**有意不动**。
- **是否需要 TMPDIR**：不需要。fake server 与全部编译测试二进制同文件系统
  （Cargo target）；`cargo test` 本身要求该文件系统可执行，因此 fixture 的可执行性
  随构建系统天然成立。
- **可移植性**：路径由 `CARGO_MANIFEST_DIR` 推导（无硬编码绝对路径）；
  `EXE_SUFFIX` 处理后缀；Windows/macOS 的 target 目录同样可执行。
- **验证**：新契约测试 `fake_server_fixture_is_built_under_the_executable_target_root`
  断言（a）binary 存在且可执行位、（b）路径在 target 下、（c）路径**不在**
  `std::env::temp_dir()` 下。修复前用最小替换复现该测试失败
  （"must be built under …/target, got /tmp/.tmpL91HKD/…"），修复后 LSP **63/0**。
  本机无 sudo 不能挂 noexec tmpfs 实测；aarch64 noexec 主机复验（可不再设 TMPDIR）
  留给审查环境，属披露的复验项而非阻塞。
- **上游 #775 涉及的 Runner fixture 复查**：runner 侧已在 #775 使用
  `executable_tempdir`（150 处引用）；本轮新增 grep 复查未发现 runner 测试存在
  fake server 同类 exec-tempdir 遗漏。

## F. Node real-process 前置条件（P3-c）

- **最低版本**：**22.7**。理由：TypeScript 腿显式传
  `--experimental-transform-types`（Node 22.7.0 引入）；产品自身的
  `typescript_node_prefix_args` 门槛为 22.6（strip-types），测试 flag 更新故取 22.7。
- **检测策略**：测试开头 `node --version` 探测 + 复用产品解析器
  `parse_node_version`（`use super::*` 已可见）+ 显式版本比较断言。零产品代码修改。
- **缺 Node 时行为**：显式 **FAILED**，消息
  `precondition failed: no Node.js on PATH (…); install Node.js >= 22.7 …`。
  实测（空 PATH 运行测试二进制）：
  ```
  test ... FAILED
  precondition failed: no Node.js on PATH (No such file or directory (os error 2)); …
  ```
  不再把 NotStarted 误报为 Shell 功能失败，也不悄悄跳过计入 PASS。`#[ignore]`
  说明同步更新为具体版本与行为约定。
- **有 Node 时**（本机 Node v26.11.1 ≥ 22.7）：测试真实执行，
  `runner_real_process_node_script_runtime_preserves_argv_stdin_and_cwd ... ok`
  （JS + TS 两腿的 argv/stdin/cwd 断言全部真实运行，0.13s）。
- **未执行的验证**：低于 22.7 的中间版本（如 22.6.x）未实测——版本比较断言按
  flag 引入史推导，属披露项。

## G. 测试矩阵（本轮）

| Test | Environment | Commit | Passed | Failed | Ignored | Exit | Log |
|---|---|---|---|---|---|---|---|
| observe_jobs 单元（含 2 新回归） | x86_64 rustc 1.99 | observe-jobs fix | 11 | 0 | 0 | 0 | 会话记录 |
| observe_jobs 全过滤 + jobs + job_log_wait + tool-contracts | 同上 | fix | 398 | 0 | 0 | 0 | p2-observe-jobs-verify.log |
| runner_quic / config / oauth / mcp / share_service | 同上 | shared-key fix | 666 | 0 | 1i | 0 | p3-sharedkey-verify.log |
| webcodex-lsp 全套 | 同上 | lsp fix | 63 | 0 | 0 | 0 | 会话记录 |
| LSP 契约测试（修复前验证） | 同上 | pre-fix swap | 0 | 1 | 0 | 101 | 会话记录（§E） |
| node real-process（有 Node） | Node 26.11.1 | node fix | 1 | 0 | 0 | 0 | 会话记录 |
| node real-process（空 PATH） | PATH=∅ | node fix | 0 | 1 | 0 | 101 | 会话记录（§F） |
| shared-key 行为 4 配置 | disposable servers | shared-key fix | — | 0 | — | 0 | 会话记录（§D） |
| `git diff --check`（worktree + v0.4.6…HEAD 提交后） | — | style fix | — | — | — | 0 | 会话记录 |
| `bash -n` 两部署脚本 | — | style fix | — | — | — | 0 | 会话记录 |

（修复前 observe_jobs 复现失败 1 例见 §C；monolithic 等全量门见 §H 后的最终复跑。）

## H. Git Evidence（推送后为准）

```bash
git rev-parse HEAD                                   # = 远端 SHA（见最终回复）
git status --short                                   # clean
git log -8 --oneline
git diff --check v0.4.6 HEAD                         # PASS（EOF 空行已清）
git merge-base --is-ancestor v0.4.6 HEAD && echo OK  # ancestry 保持
git ls-remote origin refs/heads/integration/upstream-v0.4.6
```
远程 SHA 与本地 HEAD 一致性在最终回复中报告。

## I. Reviewer Handoff

- **Base SHA**：`02c41df8`（验收基线） → **Final SHA**：见最终回复
- **改动文件**（6）：`src/tool_runtime/observe_jobs.rs`、`src/lib.rs`、
  `crates/webcodex-lsp/src/test_support.rs`、`crates/webcodex-lsp/src/tests.rs`、
  `crates/webcodex-runner/src/webcodex_runner/shell_tests.rs`、
  `pull_compile_install.sh`（+2 报告文档）
- **重点审查**：observe_jobs 修复的 `wake_detail` 传递链（apply_output_budget →
  4 处 batch_output；中间试探调用不影响输出）；src/lib.rs 删除块与保留块的
  条件等价性；LSP executable_tempdir 的清理/并发语义
- **新增测试名**：`wake_reason_survives_output_budget_truncation`、
  `wake_reason_survives_oversized_item_replacement`、
  `fake_server_fixture_is_built_under_the_executable_target_root`
- **复跑命令**：
  ```bash
  cargo test --locked -p webcodex --lib wake_reason_survives
  cargo test --locked -p webcodex --lib observe_jobs
  cargo test --locked -p webcodex-lsp
  cargo test --locked -p webcodex-runner --features runner-real-process-tests \
    runner_real_process_node_script_runtime_preserves -- --ignored --test-threads=1
  cargo test --locked -p webcodex --lib   # monolithic 门
  git diff --check v0.4.6 HEAD
  ```
- **仍未解决的风险**：aarch64 noexec 主机上 LSP 修复与 Node 前置条件的现场复验
  （本机已双向行为验证）；Node 22.6.x–22.6 尾部版本未实测。
- **是否建议进入 dogfood 部署**：建议独立审查通过后按原报告 §12 流程
  （合入 dev → wa1 dogfood 先行）评估；本报告不构成部署授权。
