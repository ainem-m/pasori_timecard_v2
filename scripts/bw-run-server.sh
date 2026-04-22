#!/bin/bash
set -euo pipefail

if ! command -v bw >/dev/null 2>&1; then
  echo "bw command not found" >&2
  exit 1
fi

cleanup() {
  if [[ "${BW_RUN_SERVER_LOCK_ON_EXIT:-0}" == "1" ]]; then
    bw lock >/dev/null 2>&1 || true
  fi
}

trap cleanup EXIT

if [[ -z "${BW_SESSION:-}" ]]; then
  if [[ -n "${BW_MASTER_PASSWORD:-}" ]]; then
    export BW_SESSION="$(bw unlock --passwordenv BW_MASTER_PASSWORD --raw)"
  else
    export BW_SESSION="$(bw unlock --raw)"
  fi
  export BW_RUN_SERVER_LOCK_ON_EXIT=1
fi

export LINEWORKS_BOT_ID="$(bw get password lineworks-bot-id)"
export LINEWORKS_BOT_SECRET="$(bw get password lineworks-bot-secret)"
export LINEWORKS_API_TOKEN="$(bw get password lineworks-api-token)"

admin_channel_id="$(bw get password lineworks-admin-channel-id 2>/dev/null || true)"
if [[ -n "${admin_channel_id}" ]]; then
  export LINEWORKS_ADMIN_CHANNEL_ID="${admin_channel_id}"
fi

exec cargo run -p server "$@"
