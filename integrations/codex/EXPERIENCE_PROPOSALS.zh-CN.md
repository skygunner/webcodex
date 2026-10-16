# 显式经验候选

[English](EXPERIENCE_PROPOSALS.md)

可选的 Python 3.10+ [适配器](experience_proposals.py) 接收操作者或当前 Host
编写的有界候选，使用现有 Workflow Session `proposal` 保存，绝不写成 `guidance`。
它不提取会话记录、不调用模型、不扫描历史、不创建 Session、不修改配置，也不新增
数据库。审阅和批准都是显式命令。Session ACK、候选正文及批准备注均不授予运行时权限。

按[集成说明](README.md#optional-adapter-macos--linux--windows-python-310)准备私有配置
及项目外预先创建的专用私有状态目录。配置固定服务器、规范 Project、现有 Workflow
Session 和本地会话。命令在配置的项目内运行；操作者或当前 Host 必须明确选择此关联，
目录匹配本身不构成写入授权。复用现有 HTTPS/回环地址、禁止重定向和环境代理、私有文件
ACL 以及状态锁。每次入口及写入派发前还通过现有只读 Project inventory 核对精确根目录映射。

凭据必须已具备原生 Session 协作及 Project 权限。Memory 读取要求 `project:read`
和 `memory:read`；写入要求 `project:write`、`memory:manage` 及正常运行时权限检查。
Memory 使用 `/mcp` 上受支持的 Stateless MCP 2026；通用 REST 明确拒绝 Memory。
Session 和文件调用使用认证 Runtime API，两种传输均不增加权限。Skill 晋升仅通过原生 `write_project_file` 写项目内容
`.agents/skills/<name>/SKILL.md`，遵守 Project 写入、路径、Runner 和权限检查。
不安装、激活或修改受信任配置目录或托管 Skill。拒绝时不会从 Memory 回退到 Skill，
也不会直接写文件绕过运行时。参见[Memory 合同](../../docs/agent/session-model.md)
和[Runtime API 合同](../../docs/agent/runtime-api-guidelines.md)。

获取精确来源引用，不导出正文：

```sh
python3 /absolute/path/experience_proposals.py --config /private/operator/observation.json source --file src/example.py
python3 /absolute/path/experience_proposals.py --config /private/operator/observation.json source --message wc_msg_EXACT_SAVED_RECEIPT
```

返回一个 `{kind, ref, sha256}` 对象。文件 SHA-256 覆盖 Runner 上完整文件。
Session 消息哈希覆盖精确已保存消息的语义字段规范 JSON，包含生命周期、解决说明和替代引用，
不含 ACK/投影计数。已保存回执只能证明其实际记录的事实；哈希不会把备注变成原生执行证据。
来源仅限文件及配置 Session 中的精确消息。

在项目外创建私有、单硬链接的候选 JSON 文件：

```json
{
  "schema": "webcodex.experience-proposal.v1",
  "summary": "检查有界读取结果",
  "content": "确认 continuation 字段后才能判断读取是否完整。",
  "conditions": ["使用有界项目检查工具时"],
  "sources": [
    {"kind": "file", "ref": "src/example.py", "sha256": "REPLACE_WITH_EXACT_CURRENT_HASH"}
  ],
  "destination": {
    "kind": "memory",
    "memory_key": "bounded-inspection",
    "expected_revision": null
  }
}
```

`null` 表示只打算创建；已存在的 Memory 必须提供精确当前 `wc_memrev_*` 版本。
原生无版本 Memory 写入对并发出现的相同内容具有期望状态幂等性，但回执不证明由谁创建；
本适配器不会盲目重试。Memory 使用 normal 优先级、`bootstrap=false` 和 `experience`
标签。审阅显示精确渲染正文：候选内容后追加 `Applies when:` 和适用条件。不要保存凭据、
秘密或私有会话记录。

项目 Skill 目标使用：

```json
{"kind":"skill","path":".agents/skills/bounded-inspection/SKILL.md","expected_sha256":null}
```

已有 Skill 将 `null` 替换为其当前完整文件 SHA-256。正文必须以 YAML frontmatter
开头，依次含 `name` 和 `description`；同样追加适用条件。范围仅为单个项目 Skill
定义，不含脚本或包安装。

```sh
python3 /absolute/path/experience_proposals.py --config /private/operator/observation.json propose --candidate /private/operator/candidate.json
python3 /absolute/path/experience_proposals.py --config /private/operator/observation.json review --message wc_msg_RETURNED_ID --version RETURNED_VERSION_HASH
python3 /absolute/path/experience_proposals.py --config /private/operator/observation.json approve --message wc_msg_RETURNED_ID --version RETURNED_VERSION_HASH --destination-hash RETURNED_DESTINATION_HASH
```

先审阅精确候选、渲染正文和目标，再执行 `approve`。它保存精确批准 `note`，重新读取
已保存回执及当前打开的候选，复核来源哈希和目标，再以批准的 CAS 版本调用 `set_memory`，
或以新获取的原生 `read_revision` 保护批准的 Skill 快照并调用 `write_project_file`。
新 Skill 使用 `overwrite=false`。返回精确候选、批准、晋升备注，以及 Memory id/版本
或文件哈希回执引用。候选最多 8 个来源、8 个条件、4 KiB 内容和 7,500 字节编码。
状态目录达到 1,024 文件时拒绝继续；这些边界不会放宽运行时限制。

派发前 fsync 私有尝试标记；重复调用返回已保存回执，不重复效果。拒绝、损坏或丢失的
效果回执留下 `promotion_outcome_unresolved_do_not_retry`；请通过原生读取工具检查
精确目标和引用，保留尝试记录，不可删除它来制造重试许可。本适配器不自动恢复或重放
不确定效果。若效果回执已保存但 Session 晋升备注丢失，下次返回
`receipt_saved_promotion_note_missing` 和精确私有回执，不重复写入。消息发布复用原生
保留期内重放键；消息逐出后，键不是历史档案，也不能证明此前效果。

新鲜度检查是顺序观察。现有 API 能原子保护目标写入，但无法将来源文件/消息、批准备注
和目标组成一个事务；最终读取之后来源或 Project 映射仍可能变化，适配器不声称全来源原子新鲜度。
测试不证明真实部署或 Host UI 验收。运行实际回环 HTTP 传输夹具测试：

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s integrations/codex -p test_experience_proposals.py -q
```

使用现有 dogfood 二进制运行不调用模型的原生 Server/WebSocket Runner 验收：

```sh
python3 scripts/e2e_experience_proposals_ws.py --bin-dir /absolute/path/target/dogfood
```

测试创建临时回环服务、私有项目和配置，验证原生 Memory 创建/CAS、Skill 创建/受保护更新、
来源/版本拒绝、缺失 Memory 管理权限、重复处理及精确已保存回执引用，最后只停止自己拥有的
Server/Runner。JSON 报告列出实际二进制版本（包含 dirty 标记）、回执引用及清理结果，
临时凭据和服务状态随之删除。该测试还确认 Memory 使用受支持的传输；回执丢失和写入竞争
由 HTTP 夹具另行覆盖，不构成真实 Host UI 验收。
