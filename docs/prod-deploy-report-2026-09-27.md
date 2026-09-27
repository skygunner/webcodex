# WebCodex 生产部署报告（2026-09-27）

**TL;DR** — 两个生产目标（ocsydamp 生产、wa1 dogfood）已完成 0.4.3（commit `d1636f72`）部署并通过全量核验；部署栈从「tmux/裸进程、重启即丢」升级为 systemd 单元化（重启自起）；一键部署脚本 `deploy_dogfood.sh` 已在 wa1 实战验证并入库。两项事项待 PM 决策（见 §5.1/5.2）。

---

## 1. 任务背景与验收标准

PM（Track A）要求：prod = **0.4.3 @ d1636f72**，MCP 工具面 **30 个**，公网健康。

| 目标 | 地址 | 接入方式 |
|---|---|---|
| ocsydamp（生产） | https://wc-ocsydamp.palmit.au | cloudflared token 隧道 |
| wa1（dogfood） | https://wd.palmit.au | `--direct` + nginx 反代 |

用户手动完成部署，本次工作的起点是**核验部署是否正确**，随后按用户指示推进单元化与自动化工具入库。

## 2. 工作阶段

### 阶段 1：部署核验（对手动部署的独立验证）

双主机均核验通过：

- **构建身份**：`0.4.3 / d1636f7202fc / dirty=false`，`built_at` 与 commit 时间一致，双机均为 aarch64，与 PM 要求完全对齐；
- **公网 MCP 冒烟**：initialize + tools/list，两侧工具面均 **30**；
- **结论**：手动部署正确，符合 PM 要求。同时发现 4 项问题（§5 风险登记）。

### 阶段 2：消除重启脆弱性（单元化）

原部署用 tmux/裸进程跑 share 和隧道，**主机重启后全部丢失**。现全部改为 systemd 用户单元（linger 已启用，重启自起、崩溃自拉起）：

- **wa1**：`webcodex-share-webcodex.service`（direct）
- **ocsydamp**：`webcodex-cloudflared.service` + `webcodex-share-webcodex.service` + `webcodex-share-teleagent-coworker.service`（双项目）

全部 enabled + active。迁移中发现并处置一个真实故障：杀旧 tmux 会话后孤儿 `webcodex-runner` 残留，导致新 share 陷入 `agent_offline` 重启循环；清除孤儿后稳定，该故障与修复方法已写入 runbook。

### 阶段 3：单元模板与 runbook 入库

**webcodex-tools 仓库 dev 分支 `c9b1afb`**（已 push）：

- `deploy/systemd/webcodex-share.service.example` — 参数化 share 单元模板（含安装步骤说明）
- `deploy/systemd/webcodex-cloudflared.service.example` — 隧道单元模板
- `deploy/systemd/README.md` — 单元清单、安装步骤、依赖排序图、故障排查（含孤儿 runner 修复）、两台主机实况

### 阶段 4：一键部署脚本 deploy_dogfood.sh

**webcodex 仓库 dev 分支 `d962a0d6`**（6 个提交，已 push）。把「备份 → 停栈 → pull_compile_install → 身份门禁 → 起栈 → 冒烟 → 失败自动回滚」串成一条命令：

```bash
# 在部署主机 ~/src/webcodex 下
./deploy_dogfood.sh --url https://wd.palmit.au/webcodex/mcp --url https://wd.palmit.au/mcp
# 可选：--ref <commit>（钉版不 pull）、--unit <extra.service>（追加停/起单元）、--dry-run
```

关键设计：

- 权威门禁 = **安装身份核验**（三件 binary 的 --version 必须含目标 commit 短 SHA 且 dirty=false），不依赖退出码不可靠的 `webcodex doctor`；
- 任一环节失败**自动回滚**：恢复备份 binary + 重启单元，栈回到部署前状态；
- 自动发现 `webcodex-share-*` 单元，检测到孤儿进程时 fail-closed 拒绝继续。

**验证方式不是模拟而是实战**：在 wa1 上完整执行 7 轮，暴露并修复 8 个真实 bug（每个修复单独提交），其中回滚路径 **两次真实触发**（构建依赖缺失一轮、停栈后 doctor 失败一轮），均干净回滚、栈恢复。终轮全绿：

```
webcodex / webcodex-server / webcodex-runner: 0.4.3 (commit d1636f7202fc, dirty=false)
webcodex-share-webcodex.service active (stable)
smoke ok: https://wd.palmit.au/webcodex/mcp  (0.4.3)
smoke ok: https://wd.palmit.au/mcp           (0.4.3)
== deploy ok: d1636f7202fc ==
rollback path: ~/backup-webcodex-20260927-025526-d1636f7202fc
```

## 3. 提交与发布状态

| 仓库/分支 | ref | 内容 |
|---|---|---|
| webcodex **dev** | `d962a0d6` | 部署脚本 6 提交链（已 push） |
| webcodex **prod** | `d1636f72` | **未动**（符合 Track A 约定） |
| webcodex-tools **dev** | `c9b1afb` | 单元模板 + runbook（已 push） |

无 tag/release/npm publish/对外发布动作。

## 4. 部署后运维模型（SOP）

- **日常升级**：目标主机上 `./deploy_dogfood.sh --url ...`，自动拉 dev、备份、部署、冒烟、失败回滚，全程一条命令；
- **ocsydamp 注意**：隧道单元不在自动发现范围，需追加 `--unit webcodex-cloudflared.service`；
- **重启**：无需人工干预，单元自起；
- **定期审计**：每日 07:40 定时任务（git pull + 双架构测试 + wa1 真机 MCP 冒烟）持续运行。

## 5. 风险登记与待决策事项

| # | 事项 | 状态 |
|---|---|---|
| 5.1 | **MCP 代理 noauth 注入范围**：机器级 mcp-proxy 对**所有路由**的无凭据请求自动补 bearer，等效于未认证写访问，当前仅靠公网暴露面收敛兜底 | **待 PM 决策**：是否收窄为只读/特定路由 |
| 5.2 | **share 凭据轮换语义**：`webcodex-share` 每次启动轮换临时凭据，栈重启后本地 CLI 凭据失效，`doctor` 必报 Authentication 失败 | **产品级语义问题**，建议另开讨论；部署脚本已按信息性处理规避 |
| 5.3 | `webcodex doctor` 存在 `[fail]` 行仍 exit 0，不能作 CI/部署门禁 | 已知行为，脚本已改用身份核验规避；可考虑产品侧修复 |
| 5.4 | ocsydamp 无 0.4.2 binary 备份（升级前无快照） | 部分闭环：`deploy_dogfood.sh` 今后每次部署前自动备份；更早版本回滚需重新编译 |
| 5.5 | 重启后服务丢失（tmux/裸进程） | **已闭环**：5 个单元全部 enabled |
| 5.6 | 旧会话孤儿 runner 引发 share 重启循环 | **已闭环**：清除 + 写入 runbook + 脚本 fail-closed 检测 |

## 6. 安全备注

- 核验过程中 share 打印的临时 bearer token 出现在聊天记录里。由于 share 每次重启即轮换凭据，且迁移单元化后已多次重启，**这些 token 现已失效**，无需追加处理。
- 全程未打印任何 token/secret/Authorization header 落盘；备份、回滚、身份核验均不依赖凭据值。
