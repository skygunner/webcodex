# 原生 Node JSON 验证与声明式构建

[English](node-json-project-validation.md)

当前用户契约见 [MCP](../MCP.zh-CN.md)。此扩展为原生 Node TAP 测试增加直接
Vitest/Jest script 支持，并为 project build gateway 增加 manifest 声明的 Node 构建。

验证 plan 只包含固定的本地框架 CLI 和报告占位符。Runner 为每个 Job 创建独占的
私有临时目录后，才替换占位符。框架创建报告；Runner 不读取项目中的旧报告。
托管进程结束后，Runner 最多读取 1 MiB，拒绝链接和特殊文件，并核对汇总计数、
逐项 assertion 状态及进程结果。只把 passed/failed 计入已执行测试；
skipped/pending/todo 不计入。缺失、不完整、超限或不一致的报告无法证明计数。
超时、停止和 lost 状态不能提供成功的测试数量证据。

终态投递前删除原始报告。有界聚合计数沿用既有 authoritative test-count evidence，
贯穿投递、snapshot、registry update、同 Runner instance inventory 对账及
ToolRuntime 结果。stdout/stderr 截断不会否定独立完整报告；日志文本不提供框架计数。
新 Runner instance 或不匹配的 receipt 不能恢复脱离身份的验证证据，也不能授权重跑。
这些报告与其他项目测试一样，不证明 source freshness。

构建复用 packageManager/lockfile 检测与固定 script argv；现代 Yarn 不接受
`run --silent`，因此使用 `run build`。Runner 覆盖 Corepack 网络、下载与 auto-pin
配置，避免适配器隐式引导包管理器。不猜测构建输出目录；显式
`finish_coding_task(outputs=...)` 检查与脚本成功、功能正确性证明分别处理。
profile 准备完成后、原生进程交接前再次核验 provenance。这是 stale-plan fence，
不代表原子的源码快照。

## 聚焦验证

单元测试覆盖报告大小限制、缺失/旧/截断报告、计数一致性、链接拒绝与清理。
recipe/Runner/registry 测试覆盖固定 argv、manager 歧义、不支持的 scope、
旧 Runner 能力拒绝与排队期间 drift。真实 fixture 经由 Job 准入、执行、终态计数
及 snapshot 序列化，验证两个框架的通过、失败、仅跳过导致零执行、仅 todo 导致零执行、
通过/跳过/todo 混合、超时与取消，并在同一项目中验证失败 → 修改源码 → 新 Job 通过。
依赖只安装在一次性目录，不修改生产状态：

```bash
fixture_dir=$(mktemp -d)
npm install --prefix "$fixture_dir" --ignore-scripts --no-audit --no-fund vitest@3.2.4 jest@30.2.0
WEBCODEX_NODE_FIXTURE_MODULES="$fixture_dir/node_modules" cargo test -p webcodex-runner -j 2 node_json_real -- --ignored
cargo test -p webcodex-runner -j 2 node_project_build_real -- --ignored
rm -rf "$fixture_dir"
```

真实 fixture 要求已有 Node/npm，目前使用 Unix symlink 放置测试依赖。
普通报告、recipe 和契约测试不安装依赖。Windows/macOS 进程行为仍需对应 CI 验证。
