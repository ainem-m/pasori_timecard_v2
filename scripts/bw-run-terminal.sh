#!/bin/bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/bw-env.sh
source "${script_dir}/lib/bw-env.sh"

trap bw_cleanup_session EXIT

bw_setup_session

terminal_token_item="${BW_TERMINAL_TOKEN_ITEM:-terminal-api-token}"

export SERVER_API_URL="${SERVER_API_URL:-http://localhost:8080/api}"
export TERMINAL_API_TOKEN
TERMINAL_API_TOKEN="$(bw_get_required_password "${terminal_token_item}")"

exec cargo run -p terminal "$@"
