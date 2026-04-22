#!/bin/bash
set -euo pipefail

if ! command -v bw >/dev/null 2>&1; then
  echo "bw command not found" >&2
  exit 1
fi

bw_setup_session() {
  if [[ -n "${BW_SESSION:-}" ]]; then
    return 0
  fi

  if [[ -n "${BW_MASTER_PASSWORD:-}" ]]; then
    export BW_SESSION
    BW_SESSION="$(bw unlock --passwordenv BW_MASTER_PASSWORD --raw)"
  else
    export BW_SESSION
    BW_SESSION="$(bw unlock --raw)"
  fi

  export BW_RUN_LOCK_ON_EXIT=1
}

bw_cleanup_session() {
  if [[ "${BW_RUN_LOCK_ON_EXIT:-0}" == "1" ]]; then
    bw lock >/dev/null 2>&1 || true
  fi
}

bw_get_required_password() {
  local item_name="$1"
  local value

  value="$(bw get password "${item_name}")"
  if [[ -z "${value}" ]]; then
    echo "Bitwarden item '${item_name}' returned an empty password" >&2
    exit 1
  fi

  printf '%s\n' "${value}"
}

bw_get_optional_password() {
  local item_name="$1"
  bw get password "${item_name}" 2>/dev/null || true
}
