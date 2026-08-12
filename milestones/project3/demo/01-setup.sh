#!/usr/bin/env bash
#
# Prepares the demo environment. Run this WELL BEFORE the presentation — Doris
# takes several minutes to become ready, and this script deliberately does not
# wait for it.
#
# It detects the two engines rather than starting them, seeds identical data
# into each, starts the proxy, and then smoke-tests all four demo cases. That
# last step is the point: the proxy is not modified for this demo, so a failure
# discovered while presenting cannot be fixed. Finding out here means finding
# out while there is still time to adjust.
#
#   ./demo/01-setup.sh          prepare and verify
#   ./demo/01-setup.sh --stop   stop the proxy and clean up
set -uo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/env.sh"

stop_proxy() {
  if proxy_running; then
    kill "$(cat "$PROXY_PIDFILE")" 2>/dev/null
  fi
  # Also clear any proxy left behind by an interrupted run, which would still
  # hold the listener ports. Matched on this demo's config path so nothing
  # else on the machine is touched.
  pkill -f "mysql-proxy .*demo/proxy.toml" 2>/dev/null
  sleep 0.5
  rm -f "$PROXY_PIDFILE"
}

if [ "${1:-}" = "--stop" ]; then
  heading "Stopping the demo proxy"
  stop_proxy
  ok "proxy stopped; the database engines were left running"
  exit 0
fi

failures=0
note_failure() { failures=$((failures + 1)); }

# ------------------------------------------------------------------ engines
heading "1. Database engines"

if endpoint_alive "$MYSQL_PORT" "$MYSQL_USER" "$MYSQL_PASS"; then
  ok "MySQL reachable on $MYSQL_PORT"
else
  bad "MySQL is not reachable on $MYSQL_PORT"
  info "start it with:  docker compose up -d"
  note_failure
fi

if endpoint_alive "$DORIS_PORT" "$DORIS_USER" "$DORIS_PASS"; then
  ok "Doris reachable on $DORIS_PORT"
else
  bad "Doris is not reachable on $DORIS_PORT"
  info "start your Doris container and wait — it needs several minutes before"
  info "it accepts connections. This script does not start or wait for it."
  note_failure
fi

if [ "$failures" -gt 0 ]; then
  heading "Cannot continue"
  info "Bring the engines up, then run this script again."
  exit 1
fi

# ------------------------------------------------------------------ identity
heading "2. What these engines actually are"
python3 "$DEMO_DIR/inspect-server.py" 127.0.0.1 "$MYSQL_PORT" "MySQL"
python3 "$DEMO_DIR/inspect-server.py" 127.0.0.1 "$DORIS_PORT" "Doris"
info "Different versions, different auth plugins, different capability sets."
info "Doris does not negotiate CLIENT_DEPRECATE_EOF, so the proxy reads its"
info "result sets through a different code path than MySQL's."

# ------------------------------------------------------------------ fixtures
heading "3. Seeding identical data"

if seed_output="$(mysql_seed 2>&1)"; then
  ok "MySQL seeded from seed-mysql.sql"
else
  bad "seeding MySQL failed"
  echo "$seed_output" | tail -5 | sed 's/^/      /'
  note_failure
fi

if seed_output="$(doris_seed 2>&1)"; then
  ok "Doris seeded from seed-doris.sql"
else
  bad "seeding Doris failed"
  echo "$seed_output" | tail -5 | sed 's/^/      /'
  note_failure
fi

# ------------------------------------------------------------------- proxy
heading "4. Starting the proxy"

if [ ! -x "$PROXY_BIN" ]; then
  info "building release binary"
  ( cd "$REPO_ROOT" && cargo build --release --quiet ) || { bad "build failed"; exit 1; }
fi

stop_proxy
mkdir -p "$DEMO_DIR/logs"
rm -f "$MYSQL_LOG" "$DORIS_LOG"

# Fully detached: all three standard descriptors are redirected so the proxy
# cannot hold this script's stdout open. Without that, piping this script
# anywhere (`| tee demo.log`) hangs after it finishes, because the reader never
# sees end-of-file.
# The whole launch group is redirected, not just the proxy. The background
# job runs in an intermediate subshell that would otherwise keep this script's
# stdout open, so piping the script anywhere (`| tee demo.log`) would hang
# after it finished — the reader never sees end-of-file.
(
  cd "$REPO_ROOT" || exit 1
  nohup "$PROXY_BIN" "$PROXY_CONFIG" < /dev/null > "$DEMO_DIR/logs/proxy.out" 2>&1 &
  echo $! > "$PROXY_PIDFILE"
) > /dev/null 2>&1 < /dev/null
sleep 1

if proxy_running; then
  ok "proxy listening on $PROXY_MYSQL_PORT (MySQL) and $PROXY_DORIS_PORT (Doris)"
else
  bad "proxy failed to start"
  sed 's/^/      /' "$DEMO_DIR/logs/proxy.out" | tail -5
  info "if a port is already in use, something else is bound to"
  info "$PROXY_MYSQL_PORT or $PROXY_DORIS_PORT — find it with: lsof -nP -iTCP:$PROXY_MYSQL_PORT -sTCP:LISTEN"
  exit 1
fi

# ------------------------------------------------------------- readiness
heading "5. Readiness — all four demo cases"

check_case() {
  local label="$1" fn="$2" expected="$3"
  local got
  got="$(count_rows "$fn")"
  if [ "$got" = "$expected" ]; then
    printf '  %sOK%s   %-40s %s rows\n' "$GREEN" "$RESET" "$label" "$got"
  else
    printf '  %sFAIL%s %-40s expected %s, got %s\n' "$RED" "$RESET" "$label" "$expected" "${got:-<nothing>}"
    note_failure
  fi
}

check_case "case 1-1  MySQL direct"          mysql_sql       8
check_case "case 1-2  proxy -> MySQL"        proxy_mysql_sql 3
check_case "case 2-1  Doris direct"          doris_sql       8
check_case "case 2-2  proxy -> Doris"        proxy_doris_sql 3

heading "Result"
if [ "$failures" -eq 0 ]; then
  printf '  %sREADY%s — run ./demo/02-direct.sh then ./demo/03-proxied.sh\n' "$GREEN" "$RESET"
  exit 0
fi

printf '  %s%s case(s) failed.%s The demo is NOT ready.\n' "$RED" "$failures" "$RESET"
info "The proxy is not modified for this demo, so a failure here cannot be"
info "fixed during the presentation. Decide now whether to drop a case."
exit 1
