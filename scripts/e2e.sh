#!/usr/bin/env bash
# e2e tests for the envsubst container image.
#
# Usage:
#   scripts/e2e.sh                    # builds envsubst:e2e, runs tests, removes it
#   scripts/e2e.sh ghcr.io/…/envsubst:sha-abc  # tests an existing image
#
# Exit code: 0 if all tests pass, 1 on first failure.
set -euo pipefail

# ── Configuration ────────────────────────────────────────────────────────────

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="${1:-envsubst:e2e}"
BUILT_IMAGE=false

# ── Helpers ──────────────────────────────────────────────────────────────────

pass() { echo "  PASS  $1"; }
fail() { echo "  FAIL  $1"; echo "        expected: $2"; echo "        got:      $3"; exit 1; }

assert_contains() {
  local label="$1" needle="$2" haystack="$3"
  if [[ "$haystack" == *"$needle"* ]]; then
    pass "$label"
  else
    fail "$label" "…${needle}…" "$haystack"
  fi
}

assert_not_contains() {
  local label="$1" needle="$2" haystack="$3"
  if [[ "$haystack" != *"$needle"* ]]; then
    pass "$label"
  else
    fail "$label" "(no match for '${needle}')" "$haystack"
  fi
}

assert_exit() {
  local label="$1" want="$2" got="$3"
  if [[ "$want" == "$got" ]]; then
    pass "$label (exit $want)"
  else
    fail "$label" "exit $want" "exit $got"
  fi
}

# Run the container.
# $1..N: arguments forwarded to envsubst
# Mounts:
#   /in  → testdata/integration  (read-only)
#   /out → a fresh temp directory (writable)
# Env vars are forwarded via --env flags when EV2E_* vars are set in the caller.
run_container() {
  local -a env_flags=()
  while IFS='=' read -r key _; do
    [[ "$key" == EV2E_* ]] && env_flags+=(--env "${key}=${!key}")
  done < <(env | grep '^EV2E_' || true)

  docker run --rm \
    "${env_flags[@]}" \
    --volume "${REPO_ROOT}/testdata/integration:/in:ro" \
    --volume "${OUT_DIR}:/out" \
    "$IMAGE" \
    "$@"
}

# ── Build (if no image was supplied) ─────────────────────────────────────────

if [[ "$#" -eq 0 ]]; then
  echo "Building ${IMAGE}…"
  docker buildx build \
    --tag "$IMAGE" \
    --load \
    "$REPO_ROOT"
  BUILT_IMAGE=true
  echo ""
fi

echo "Testing image: ${IMAGE}"
echo ""

# ── Tests ────────────────────────────────────────────────────────────────────

# 1. Basic substitution from real env, single file → stdout
echo "=== 1. Real env → stdout ==="
OUT_DIR="$(mktemp -d)"
EV2E_EV_GREETING=hello EV2E_EV_SERVICE=my-svc \
  out="$(run_container "/in/template.yaml")"
assert_contains "greeting substituted"     "greeting: hello"          "$out"
assert_contains "service substituted"      "service: my-svc"          "$out"
assert_contains "unset var left as-is"     '${EV_UNDEFINED_12345}'    "$out"
rm -rf "$OUT_DIR"

# 2. Output directory, nested path preserved
echo ""
echo "=== 2. Output dir, nested structure ==="
OUT_DIR="$(mktemp -d)"
EV2E_EV_HOST=db.local EV2E_EV_PORT=5432 EV2E_EV_DEBUG=false \
  run_container "/in/nested/service.conf" --output /out > /dev/null
out="$(cat "${OUT_DIR}/service.conf")"
assert_contains "host substituted"   "host = db.local"  "$out"
assert_contains "port substituted"   "port = 5432"      "$out"
assert_contains "debug substituted"  "debug = false"    "$out"
rm -rf "$OUT_DIR"

# 3. Glob pattern, multiple files, output dir mirroring
echo ""
echo "=== 3. Glob pattern, mirrored output ==="
OUT_DIR="$(mktemp -d)"
EV2E_EV_GREETING=hi EV2E_EV_SERVICE=svc \
  EV2E_EV_HOST=h EV2E_EV_PORT=80 EV2E_EV_DEBUG=true \
  run_container "/in/**/*" --output /out > /dev/null
# Both files must appear under the mirrored structure
[[ -f "${OUT_DIR}/template.yaml" ]]        && pass "template.yaml written" \
                                           || fail "template.yaml written" "file exists" "missing"
[[ -f "${OUT_DIR}/nested/service.conf" ]]  && pass "nested/service.conf written" \
                                           || fail "nested/service.conf written" "file exists" "missing"
rm -rf "$OUT_DIR"

# 4. --env-file mode, real env is ignored
echo ""
echo "=== 4. --env-file mode ==="
OUT_DIR="$(mktemp -d)"
# Write a .env file into the mounted volume so the container can read it
ENV_FILE="${REPO_ROOT}/testdata/integration/.e2e.env"
printf 'EV_HOST=from-file\nEV_PORT=9999\nEV_DEBUG=yes\n' > "$ENV_FILE"
# Real env supplies different values — they must be ignored
EV2E_EV_HOST=real-host EV2E_EV_PORT=0 EV2E_EV_DEBUG=real \
  run_container "/in/nested/service.conf" --env-file /in/.e2e.env --output /out > /dev/null
out="$(cat "${OUT_DIR}/service.conf")"
assert_contains     "file value used"        "host = from-file"  "$out"
assert_not_contains "real env not used"      "real-host"         "$out"
rm -f "$ENV_FILE"
rm -rf "$OUT_DIR"

# 5. Multiple --env-file globs, later file wins on conflict
echo ""
echo "=== 5. Multiple --env-file, last wins ==="
OUT_DIR="$(mktemp -d)"
BASE_ENV="${REPO_ROOT}/testdata/integration/.e2e-base.env"
OVER_ENV="${REPO_ROOT}/testdata/integration/.e2e-over.env"
printf 'EV_HOST=base-host\nEV_PORT=1111\nEV_DEBUG=base\n' > "$BASE_ENV"
printf 'EV_HOST=override-host\n'                           > "$OVER_ENV"
run_container "/in/nested/service.conf" \
  --env-file /in/.e2e-base.env \
  --env-file /in/.e2e-over.env \
  --output /out > /dev/null
out="$(cat "${OUT_DIR}/service.conf")"
assert_contains "override wins"       "host = override-host"  "$out"
assert_contains "base still applies"  "port = 1111"           "$out"
rm -f "$BASE_ENV" "$OVER_ENV"
rm -rf "$OUT_DIR"

# 6. --fail-on-missing exits 1 when variables are absent
echo ""
echo "=== 6. --fail-on-missing exits 1 ==="
OUT_DIR="$(mktemp -d)"
exit_code=0
EV2E_EV_GREETING=hi EV2E_EV_SERVICE=svc \
  run_container "/in/template.yaml" --fail-on-missing --output /out > /dev/null 2>&1 \
  || exit_code=$?
assert_exit "--fail-on-missing with missing var" "1" "$exit_code"
rm -rf "$OUT_DIR"

# 7. --fail-on-missing exits 0 when all variables are resolved
echo ""
echo "=== 7. --fail-on-missing exits 0 when all resolved ==="
OUT_DIR="$(mktemp -d)"
exit_code=0
EV2E_EV_HOST=h EV2E_EV_PORT=80 EV2E_EV_DEBUG=false \
  run_container "/in/nested/service.conf" --fail-on-missing --output /out > /dev/null \
  || exit_code=$?
assert_exit "--fail-on-missing all resolved" "0" "$exit_code"
rm -rf "$OUT_DIR"

# ── Cleanup ───────────────────────────────────────────────────────────────────

echo ""
if $BUILT_IMAGE; then
  docker rmi "$IMAGE" > /dev/null 2>&1 || true
fi

echo "All e2e tests passed."
