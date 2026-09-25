#!/usr/bin/env bash
set -euo pipefail

git pull --ff-only

head="$(git rev-parse --short=12 HEAD)"
built_at="$(git show -s --format=%ct HEAD)"
expected_version="$(
  cargo metadata --no-deps --format-version 1 |
    python3 -c 'import json,sys; data=json.load(sys.stdin); print(next(p["version"] for p in data["packages"] if p["name"] == "webcodex"))'
)"
expected_target="$(rustc -vV | sed -n 's/^host: //p')"
expected_arch="$(rustc --print cfg | sed -n 's/^target_arch="\(.*\)"$/\1/p')"

if [[ -z "$expected_version" || -z "$expected_target" || -z "$expected_arch" ]]; then
  echo "failed to resolve expected build identity" >&2
  exit 1
fi

export WEBCODEX_BUILT_AT="$built_at"

RUSTFLAGS="-C target-cpu=native" \
  cargo build --locked --release --workspace --bins

assert_build_identity() {
  local path="$1"
  local binary="$2"
  local build_info

  build_info="$("$path" --build-info-json)"
  printf '%s\n' "$build_info"

  EXPECTED_BINARY="$binary" \
  EXPECTED_VERSION="$expected_version" \
  EXPECTED_COMMIT="$head" \
  EXPECTED_BUILT_AT="$built_at" \
  EXPECTED_TARGET="$expected_target" \
  EXPECTED_ARCH="$expected_arch" \
  BUILD_INFO_JSON="$build_info" \
    python3 - <<'PY'
import json
import os
import sys

info = json.loads(os.environ["BUILD_INFO_JSON"])
expected = {
    "binary": os.environ["EXPECTED_BINARY"],
    "version": os.environ["EXPECTED_VERSION"],
    "git_commit": os.environ["EXPECTED_COMMIT"],
    "git_dirty": False,
    "built_at": os.environ["EXPECTED_BUILT_AT"],
    "target": os.environ["EXPECTED_TARGET"],
    "architecture": os.environ["EXPECTED_ARCH"],
}

mismatches = {
    key: {"expected": value, "observed": info.get(key)}
    for key, value in expected.items()
    if info.get(key) != value
}
if mismatches:
    print(
        f"build identity mismatch for {expected['binary']}: "
        + json.dumps(mismatches, sort_keys=True),
        file=sys.stderr,
    )
    raise SystemExit(1)
PY
}

for bin in webcodex webcodex-server webcodex-runner; do
  assert_build_identity "target/release/$bin" "$bin"
done

cp -v target/release/webcodex* ~/.local/bin/

for bin in webcodex webcodex-server webcodex-runner; do
  assert_build_identity "$HOME/.local/bin/$bin" "$bin"
  "$HOME/.local/bin/$bin" --version
done

webcodex doctor

