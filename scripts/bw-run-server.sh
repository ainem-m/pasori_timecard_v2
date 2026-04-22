#!/bin/bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/bw-env.sh
source "${script_dir}/lib/bw-env.sh"

trap bw_cleanup_session EXIT

bw_setup_session

export LINEWORKS_BOT_ID
LINEWORKS_BOT_ID="$(bw_get_required_password "lineworks-bot-id")"
export LINEWORKS_BOT_SECRET
LINEWORKS_BOT_SECRET="$(bw_get_required_password "lineworks-bot-secret")"
export LINEWORKS_API_TOKEN
LINEWORKS_API_TOKEN="$(bw_get_required_password "lineworks-api-token")"

admin_channel_id="$(bw_get_optional_password "lineworks-admin-channel-id")"
if [[ -n "${admin_channel_id}" ]]; then
  export LINEWORKS_ADMIN_CHANNEL_ID="${admin_channel_id}"
fi

exec cargo run -p server "$@"
