#!/usr/bin/env bash
#
# Cases 1-1 and 2-1: the query straight to each engine, no proxy involved.
#
# This establishes the baseline. Both engines hold the same eight rows, and
# both return all of them, because nothing is filtering anything yet.
set -uo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/env.sh"

run_case() {
  local label="$1" endpoint="$2" fn="$3"

  heading "$label — $endpoint"
  printf '  %sthe query:%s\n' "$DIM" "$RESET"
  grep -v '^--' "$DEMO_DIR/query.sql" | grep -v '^\s*$' | sed 's/^/    /'
  echo

  local output
  if ! output="$(run_query_file "$fn" 2>&1)" || [ -z "$output" ]; then
    bad "could not reach $endpoint"
    info "run ./demo/01-setup.sh first, and check the engine is up"
    return 1
  fi
  echo "$output" | sed 's/^/    /'

  local total
  total="$(count_rows "$fn")"
  printf '\n  %s%s rows returned%s — the whole table\n' "$BOLD" "$total" "$RESET"
}

heading "DIRECT CONNECTIONS — no proxy"
info "Same query, two different database engines, nothing in between."

status=0
run_case "case 1-1" "MySQL   127.0.0.1:$MYSQL_PORT" mysql_sql || status=1
run_case "case 2-1" "Doris   127.0.0.1:$DORIS_PORT" doris_sql || status=1

heading "Baseline established"
info "Both engines hold all eight orders across EU, US and APAC."
info "Next: ./demo/03-proxied.sh — the same query, through the proxy."
exit "$status"
