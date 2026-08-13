#!/usr/bin/env bash
#
# Cases 1-2 and 2-2: the same query, through the proxy, to the same engines.
#
# The client sends exactly what it sent in 02-direct.sh. The proxy rewrites it
# in flight and the engine never sees the original. The forwarded statement is
# read back from the proxy's own log so the injected predicate is visible
# rather than asserted.
set -uo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/env.sh"

# Prints the most recent rewritten statement recorded by a listener.
#
# The evidence comes from the proxy's own log, not from this script: it reads
# the JSONL the proxy wrote and prints the `forwarded_statement` field of the
# last record flagged `rewritten`. Nothing here reconstructs what the rewrite
# ought to have been, which is the whole point — a script that printed its own
# idea of the expected SQL would keep printing it after the rewriter broke.
#
# An empty result is therefore meaningful and is not an error: it means no
# record in that log claims a rewrite happened. The caller prints the rows
# either way, so a filter that silently stopped applying shows up as a missing
# statement line next to eight rows instead of three.
forwarded_statement() {
  python3 - "$1" <<'PY'
import json, sys
try:
    rows = [json.loads(l) for l in open(sys.argv[1]) if l.strip()]
except OSError:
    sys.exit(0)
rewrites = [r for r in rows if r.get("type") == "command" and r.get("rewritten")]
if rewrites:
    print(rewrites[-1]["forwarded_statement"])
PY
}

run_case() {
  local label="$1" endpoint="$2" fn="$3" logfile="$4"

  heading "$label — via proxy to $endpoint"
  printf '  %syou typed:%s\n' "$DIM" "$RESET"
  grep -v '^--' "$DEMO_DIR/query.sql" | grep -v '^\s*$' | sed 's/^/    /'

  local output
  if ! output="$(run_query_file "$fn" 2>&1)" || [ -z "$output" ]; then
    bad "could not reach the proxy on this listener"
    info "run ./demo/01-setup.sh first"
    return 1
  fi

  local sent
  sent="$(forwarded_statement "$logfile")"
  if [ -n "$sent" ]; then
    printf '\n  %sthe engine actually received:%s\n' "$DIM" "$RESET"
    printf '    %s%s%s\n' "$CYAN" "$sent" "$RESET"
    printf '    %s%s— injected by the proxy, never typed by the client%s\n' "$DIM" "  " "$RESET"
  fi

  echo
  echo "$output" | sed 's/^/    /'

  local shown
  shown="$(count_rows "$fn")"
  printf '\n  %s%s rows returned%s — filtered to %s\n' "$BOLD" "$shown" "$RESET" "$FILTER"
}

heading "THROUGH THE PROXY — identical SQL"
info "The client sends the same query as before. It is not aware of any filter."

status=0
run_case "case 1-2" "MySQL" proxy_mysql_sql "$MYSQL_LOG" || status=1
run_case "case 2-2" "Doris" proxy_doris_sql "$DORIS_LOG" || status=1

heading "What just happened"
info "One proxy binary, one rule, two different database engines."
info "Neither engine was configured for this, and the client was not changed."
info "MySQL and Doris do not even negotiate the same protocol capabilities —"
info "run ./demo/01-setup.sh to see the two handshakes side by side."
exit "$status"
