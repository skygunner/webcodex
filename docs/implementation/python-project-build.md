# Python project packaging

[简体中文](python-project-build.zh-CN.md)

`project_build(adapter=python)` packages a Project with an existing
`pyproject.toml`. Auto selection uses the same nearest, unambiguous recipe root
as other project operations. Python rejects `scope`, including all-packages;
`cwd` chooses one packaging root. There are no caller executable, argv,
configuration, output-directory, install or environment fields.

Runner owns the exact logical argv `python -m build --no-isolation`. It resolves
one existing configured/profile/PATH Python interpreter through the existing
script runtime selector, with a configured interpreter taking precedence and
being pinned for the probe and actual spawn. A bounded managed probe checks
Python 3 and frontend availability using the same environment. The frontend
retains its backend dependency checks. No environment creation, package
installation or dependency-check suppression occurs. Default packaging builds
an sdist, then builds a wheel from that sdist. `dependency_policy.mode=locked`
uses the same no-install command; it does not promise that arbitrary backend
code cannot write files or use the network.

Explicit Python planning and all Python Job admissions require additive
`project_build_python_v1`, false on older Runners. Generic build support cannot
imply Python packaging support. Manifest, `setup.cfg` and `setup.py` inputs are
bounded by the existing recipe digest machinery. Admission and queue promotion
replan the same registered Project identity, root, cwd, inputs and invocation.
The process uses normal Runner shell policy, profile preparation and managed
process-tree ownership, with null stdin, cancellation and the total Job timeout.
Pending execution retains one durable Job identity and never permits replay.

Successful native packaging may attach `build_artifacts`, a bounded observation
of existing regular files in the default `dist/` directory, using the existing
artifact metadata reader. It records only Project-relative path, bytes, SHA-256
and MIME (null when unrecognized), with at most 16 files and 256 MiB total. Exit zero without observed sdist and wheel outputs reports `build_artifacts_missing` while retaining the native exit status; it does not invent an empty successful artifact receipt. Missing historical metadata is unknown,
not an empty inferred success. Pre-existing files can be observed too: neither
presence nor a hash proves creation by this Job, package correctness or source
freshness. Tests and package acceptance remain separate. Read/export uses the
existing artifact tools, without another artifact store. Symlinked output
directories, symlink files, escaping and sensitive paths fail closed. Known
observations survive canonical Job reconciliation and terminal receipts;
historical missing observations remain absent.

Focused tests:

```sh
cargo test --locked -p webcodex-core --lib project_build
cargo test --locked -p webcodex-runner-registry --lib project_build
cargo test --locked -p webcodex-runner --bin webcodex-runner project_build
cargo test --locked -p webcodex-tool-contracts --all-features --lib project_build
```

The opt-in real-process fixture requires a separately provisioned existing Python
environment with `build`, `setuptools` and `wheel`; the fixture never installs.
It verifies package contents, default sdist-to-wheel order, existing output
observations, missing dependencies, backend failure, cancellation, timeout and
same-execution Job identity:

```sh
WEBCODEX_TEST_BUILD_PYTHON=/absolute/venv/bin/python \
  cargo test --locked -p webcodex-runner --bin webcodex-runner \
  --features runner-real-process-tests runner_real_process_python_project_build_fixtures \
  -- --ignored --test-threads=1
```
