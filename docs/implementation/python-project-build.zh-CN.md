# Python 项目打包

[English](python-project-build.md)

`project_build(adapter=python)` 为已有 `pyproject.toml` 的 Project 打包。自动选择沿用最近且无歧义的 recipe root；`cwd` 选择一个打包根目录。Python 不接受 `scope`，包括 all-packages。调用者不能指定 executable、argv、配置、输出目录、安装或环境字段。

Runner 拥有固定逻辑命令 `python -m build --no-isolation`。解释器沿用既有脚本运行时选择器，按 configured/profile/PATH 解析；已配置解释器优先，探测与实际启动固定使用同一个解释器。受管理且有界的探测使用相同环境，检查 Python 3 与 build frontend 可用性。保留 frontend 的 backend dependency checks，不创建环境、不安装包、不跳过依赖检查。默认先构建 sdist，再从 sdist 构建 wheel。`dependency_policy.mode=locked` 使用相同的 no-install 命令，不承诺任意 backend 代码无法写文件或访问网络。

显式 Python planning 与每次 Python Job admission 都要求 additive `project_build_python_v1`，旧 Runner 缺省为 false；通用 build capability 不能隐含 Python 支持。既有 recipe digest 对 manifest、`setup.cfg`、`setup.py` 建立有界来源校验。准入、排队后以及环境准备与解释器探测完成后的启动前检查，会核对相同 registered Project identity、root、cwd、输入与调用。过期计划不会启动原生构建。

执行沿用 Runner shell policy、profile preparation、进程树所有权、空 stdin、取消与总 Job 超时。Pending 保留同一 durable Job identity，不授权重放。

成功的原生打包可附带 `build_artifacts`：通过已有 artifact metadata reader 观察默认 `dist/` 中现存普通文件，只记录 Project-relative path、bytes、SHA-256 与 MIME（无法识别时为 null），最多 16 个文件、总计 256 MiB。原生退出码为 0 但没有观察到 sdist 与 wheel 时，报告 `build_artifacts_missing` 并保留原生退出码，不制造空的成功产物回执。

观察可能包含旧文件；存在与摘要都不证明本 Job 创建文件、包功能正确或源码新鲜。缺失历史 metadata 保持 unknown。符号链接输出目录、符号链接文件、逃逸及敏感路径均拒绝。已知 metadata 沿同一 Job 的 reconciliation 与 terminal receipt 恢复；文件内容仍通过现有 artifact tools 读取或导出，不新增产物数据库。

定向检查：

```sh
cargo test --locked -p webcodex-core --lib project_build
cargo test --locked -p webcodex-runner-registry --lib project_build
cargo test --locked -p webcodex-runner --bin webcodex-runner project_build
cargo test --locked -p webcodex-tool-contracts --all-features --lib project_build
```

真实进程 fixture 要求单独准备已有 Python 环境，其中安装 `build`、`setuptools` 与 `wheel`；fixture 自身不安装依赖。它核对包内容、默认 sdist→wheel 顺序、旧产物观察、依赖缺失、backend 失败、取消、超时、缺失产物与探测期间清单变化，以及同一次执行的 Job identity：

```sh
WEBCODEX_TEST_BUILD_PYTHON=/absolute/venv/bin/python \
  cargo test --locked -p webcodex-runner --bin webcodex-runner \
  --features runner-real-process-tests runner_real_process_python_project_build_fixtures \
  -- --ignored --test-threads=1
```
