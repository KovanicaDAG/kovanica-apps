#!/usr/bin/env bash
# One bounded recovery command for the Linux node supervisor and monitor.
# Exit 0: mocked regressions passed and the live chain-5 run passed.
# Exit 2: live half BLOCKED. That is not a pass.
# Any other non-zero status: a real assertion or build failure.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
ART="${RECOVERY_ARTIFACT_DIR:-$ROOT/recovery-artifacts}"
mkdir -p "$ART"
export RECOVERY_ARTIFACT_DIR="$ART"
rm -f "$ART/live.pids" "$ART/report.json"

cleanup() {
  if [[ -f "$ART/live.pids" ]]; then
    while read -r pid; do
      [[ -n "${pid}" ]] || continue
      kill -TERM -- "-${pid}" 2>/dev/null || kill -TERM -- "${pid}" 2>/dev/null || true
    done < "$ART/live.pids"
  fi
}
trap cleanup EXIT

echo "=== mocked UI recovery regressions ==="
npm test

echo "=== mocked supervisor and monitor regressions ==="
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib --no-default-features -- --test-threads=8

blocked() {
  RECOVERY_REASON="$1" python3 - <<'PY'
import json, os
from pathlib import Path
art = Path(os.environ["RECOVERY_ARTIFACT_DIR"])
reason = os.environ["RECOVERY_REASON"]
report = {
    "suite": "node-desktop-recovery",
    "result": "blocked",
    "blocked": reason,
    "limitations": [
        "Live chain-5 recovery did not run. A missing prerequisite is not a pass.",
        "Chain 1337 was not substituted. KOVANICA_TEST_TARGET_BITS was not enabled.",
    ],
    "checks": [],
}
(art / "report.json").write_text(json.dumps(report, indent=2) + "\n")
print("BLOCKED:", reason)
PY
  exit 2
}

if [[ -n "${KOVANICA_TEST_TARGET_BITS:-}" ]]; then
  blocked "KOVANICA_TEST_TARGET_BITS is set. This suite will not use a runtime difficulty override. Unset it. The value is not shown."
fi

foreign="$(env | awk -F= '/^KOVANICA_/ {print $1}' | tr '\n' ' ' | sed 's/ *$//')"
if [[ -n "${foreign}" ]]; then
  blocked "KOVANICA_* is set (${foreign}). Values are not shown. Unset them before the live half. Chain 1337 is not a substitute."
fi

if [[ -z "${NODE_RECOVERY_BIN:-}" ]]; then
  blocked "NODE_RECOVERY_BIN is unset. Pass the absolute path of a kovanica-node binary. This suite does not select a checkout binary and does not treat chain 1337 as recovery."
fi
if [[ "${NODE_RECOVERY_BIN}" != /* ]]; then
  blocked "NODE_RECOVERY_BIN must be an absolute path. A relative path is not assumed to be a released binary."
fi
if [[ ! -x "${NODE_RECOVERY_BIN}" ]]; then
  blocked "NODE_RECOVERY_BIN is not an executable file: ${NODE_RECOVERY_BIN}"
fi
if [[ -z "${NODE_RECOVERY_FUNDED_ACCOUNTS:-}" ]]; then
  blocked "NODE_RECOVERY_FUNDED_ACCOUNTS is unset. Point it at the chain-5 funded_accounts.toml published for the node image (L1 ops/docker/chain5/funded_accounts.toml). This suite does not embed that file."
fi
if [[ "${NODE_RECOVERY_FUNDED_ACCOUNTS}" != /* || ! -f "${NODE_RECOVERY_FUNDED_ACCOUNTS}" ]]; then
  blocked "NODE_RECOVERY_FUNDED_ACCOUNTS must be an absolute file path. This suite does not invent a funded-account fixture."
fi

echo "=== live chain-5 recovery ==="
if ! command -v timeout >/dev/null 2>&1; then
  blocked "timeout(1) is missing, so the live half has no outer bound. It was not run."
fi
set +e
timeout 12m cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --test recovery_live -- --test-threads=1 --nocapture
code=$?
set -e
if [[ "$code" -eq 124 ]]; then
  echo "live recovery exceeded 12 minutes" >&2
fi
exit "$code"
