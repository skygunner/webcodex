#!/usr/bin/env bash
# deploy_dogfood.sh — dogfood/生产机一键部署：备份 → pull_compile_install → 重启 share 单元 → 冒烟 → 失败回滚。
#
# 需要 bash（使用数组与 [[ ]]）。在目标机的 webcodex 仓库内执行：
#   ./deploy_dogfood.sh                     # 自动发现 webcodex-share-* 单元
#   ./deploy_dogfood.sh --url https://h/mcp --url https://h/pi/mcp
#   ./deploy_dogfood.sh --ref d1636f7202fca25d420383bcff59f952e2e22fa7
#   ./deploy_dogfood.sh --dry-run
#
# 流程：
#   0. 前置检查（clean tree；--ref 时 HEAD 必须等于该 commit）
#   1. 备份 ~/.local/bin/{webcodex,webcodex-server,webcodex-runner}
#      → ~/backup-webcodex-<日期>-<short>/（无旧安装则跳过并记下：本次不可回滚）
#   2. 停止 webcodex-share-* 单元（只停 active 的；避免 binary Text file busy）
#   3. ./pull_compile_install.sh（ff-only pull + --release 构建 + identity 断言 + 安装 + doctor）
#   4. 安装后身份复验：--version 必须含当前 HEAD short sha 且 dirty=false
#   5. 重启第 2 步停掉的单元；15s 宽限后复查仍 active（防 crash-loop）
#   6. 若给了 --url，逐个 initialize 冒烟（期望 200 且含 serverInfo）
#   任一步失败且存在备份 → 自动恢复旧三件并重启单元后退出 1。
#   注意：回滚只还原 binary；仓库 ref 停留在新 commit（git 状态不回退）。

set -euo pipefail

URLS=()
EXTRA_UNITS=()
PIN_REF=""
DRY_RUN=0

usage() { sed -n '2,20p' "$0"; exit 0; }
while [[ $# -gt 0 ]]; do
  case "$1" in
    --url) URLS+=("$2"); shift 2 ;;
    --unit) EXTRA_UNITS+=("$2"); shift 2 ;;
    --ref) PIN_REF="$2"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) usage ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

command -v git >/dev/null && command -v systemctl >/dev/null || { echo "need git+systemctl" >&2; exit 2; }
[[ -f pull_compile_install.sh ]] || { echo "run from the webcodex repo root (pull_compile_install.sh not found)" >&2; exit 2; }
[[ -n "$(git status --porcelain --untracked-files=no)" ]] && { echo "tracked worktree dirty; commit/stash first (build identity must be dirty=false)" >&2; exit 2; }

git pull --ff-only origin "$(git branch --show-current)" >/dev/null 2>&1 || true
HEAD_SHA="$(git rev-parse HEAD)"
HEAD_SHORT="$(git rev-parse --short=12 HEAD)"
if [[ -n "$PIN_REF" && "$HEAD_SHA" != "$PIN_REF" ]]; then
  echo "HEAD $HEAD_SHA != pinned $PIN_REF; abort" >&2; exit 2
fi
echo "== source: $HEAD_SHA ($(git branch --show-current)) =="

# 发现需要管理的 share 单元（enabled 或 active 的 webcodex-share-*；可再 --unit 追加）
# 纳入管理 = active 或 enabled 的 webcodex-share-* 单元（enabled 说明运维意图是常驻，
# 失败运行中被 stop 过的也要拉回）。bash 5.1 空数组 ${arr[@]:-} 会展开出一个空串元素，
# 一律用 ${arr[@]+"${arr[@]}"} 惯用法。
mapfile -t UNITS < <(systemctl --user list-unit-files 'webcodex-share-*.service' --no-legend 2>/dev/null | awk '{print $1}')
for u in ${EXTRA_UNITS[@]+"${EXTRA_UNITS[@]}"}; do UNITS+=("$u"); done
ACTIVE_UNITS=()
for u in ${UNITS[@]+"${UNITS[@]}"}; do
  [[ "$(systemctl --user is-active "$u" 2>/dev/null)" == "active" ]] && ACTIVE_UNITS+=("$u") && continue
  [[ "$(systemctl --user is-enabled "$u" 2>/dev/null)" == "enabled" ]] && ACTIVE_UNITS+=("$u")
done
echo "== share units: ${ACTIVE_UNITS[*]:-none} =="

BIN_DIR="$HOME/.local/bin"
BINS=(webcodex webcodex-server webcodex-runner)
STAMP="$(date -u +%Y%m%d-%H%M%S)"
BACKUP_DIR="$HOME/backup-webcodex-$STAMP-$HEAD_SHORT"
HAVE_BACKUP=0

dry() { [[ $DRY_RUN == 1 ]] && { echo "[dry-run] $*"; return 0; }; return 1; }

backup() {
  local missing=0
  for b in "${BINS[@]}"; do [[ -x "$BIN_DIR/$b" ]] || missing=1; done
  if [[ $missing == 1 ]]; then
    echo "== no previous install found; rollback unavailable this run =="
    return 0
  fi
  dry "backup $BIN_DIR/{${BINS[*]}} -> $BACKUP_DIR" && return 0
  mkdir -p "$BACKUP_DIR"
  cp -a "${BINS[@]/#/$BIN_DIR/}" "$BACKUP_DIR/"
  for b in "${BINS[@]}"; do "$BACKUP_DIR/$b" --version >/dev/null; done
  HAVE_BACKUP=1
  echo "== backup ok: $BACKUP_DIR =="
}

stop_units() {
  for u in ${ACTIVE_UNITS[@]+"${ACTIVE_UNITS[@]}"}; do
    dry "stop $u" || systemctl --user stop "$u"
  done
  sleep 1
  # 残留进程仍握着 binary 会让 cp 报 Text file busy——宁可中止也不静默覆盖失败
  if pgrep -f 'bin/webcodex-serve[r]|bin/webcodex-runne[r]' >/dev/null; then
    pgrep -af 'bin/webcodex-serve[r]|bin/webcodex-runne[r]' | head -5 || true
    echo "refusing to install: stray webcodex-server/runner still running (tmux/其他单元?); stop them first" >&2
    return 1
  fi
}

verify_identity() {
  local bin="$1"
  local v; v="$("$bin" --version)"
  [[ "$v" == *"$HEAD_SHORT"* && "$v" == *"dirty=false"* ]] || {
    echo "identity mismatch after install: $v (want $HEAD_SHORT, dirty=false)" >&2; return 1; }
  echo "  $v"
}

start_units() {
  for u in ${ACTIVE_UNITS[@]+"${ACTIVE_UNITS[@]}"}; do
    dry "start $u" || systemctl --user start "$u"
  done
  [[ ${#ACTIVE_UNITS[@]} -eq 0 ]] && return 0
  sleep 15
  for u in ${ACTIVE_UNITS[@]+"${ACTIVE_UNITS[@]}"}; do
    local before after
    before="$(systemctl --user show "$u" -p ExecMainStartTimestamp --value)"
    [[ "$(systemctl --user is-active "$u")" == "active" ]] || { echo "$u not active" >&2; return 1; }
    sleep 10
    after="$(systemctl --user show "$u" -p ExecMainStartTimestamp --value)"
    [[ "$before" == "$after" ]] || { echo "$u restart-looping (main pid restarted during grace)" >&2; return 1; }
    echo "  $u active (stable)"
  done
}

smoke() {
  [[ ${#URLS[@]} -eq 0 ]] && { echo "== no --url given; skip public smoke =="; return 0; }
  local u
  for u in ${URLS[@]+"${URLS[@]}"}; do
    local body
    body="$(curl -s -m 25 -X POST "$u" -H 'Content-Type: application/json' \
      -H 'Accept: application/json, text/event-stream' \
      -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"deploy-smoke","version":"0"}}}')"
    grep -q '"serverInfo"' <<<"$body" || { echo "smoke FAIL $u: $(head -c 120 <<<"$body")" >&2; return 1; }
    echo "  smoke ok: $u ($(grep -o '"name":"[^"]*","version":"[^"]*"' <<<"$body" | head -1))"
  done
}

rollback() {
  echo "!! FAILURE — rollback triggered" >&2
  if [[ $HAVE_BACKUP == 1 ]]; then
    for u in ${ACTIVE_UNITS[@]+"${ACTIVE_UNITS[@]}"}; do systemctl --user stop "$u" 2>/dev/null || true; done
    cp -a "$BACKUP_DIR/webcodex" "$BACKUP_DIR/webcodex-server" "$BACKUP_DIR/webcodex-runner" "$BIN_DIR/"
    for b in "${BINS[@]}"; do "$BIN_DIR/$b" --version; done
    for u in ${ACTIVE_UNITS[@]+"${ACTIVE_UNITS[@]}"}; do systemctl --user start "$u" 2>/dev/null || true; done
    echo "rolled back to $( "$BIN_DIR/webcodex" --version ); units restarted. Repo ref stays at $HEAD_SHORT." >&2
  else
    echo "no backup existed; nothing to roll back — inspect manually" >&2
  fi
}

main() {
  backup
  dry "./pull_compile_install.sh" && exit 0
  stop_units
  if ! ./pull_compile_install.sh; then
    rollback; exit 1
  fi
  echo "== installed identity =="
  local id_ok=0
  for b in "${BINS[@]}"; do verify_identity "$BIN_DIR/$b" || id_ok=1; done
  [[ $id_ok == 0 ]] || { rollback; exit 1; }
  start_units || { rollback; exit 1; }
  smoke || { rollback; exit 1; }
  echo "== deploy ok: $HEAD_SHORT =="
  [[ $HAVE_BACKUP == 1 ]] && echo "rollback path: $BACKUP_DIR"
}
main
