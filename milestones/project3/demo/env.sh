#!/usr/bin/env bash
#
# Shared configuration for the demo scripts.
#
# Every endpoint and credential lives here so the three scripts cannot drift
# apart. Sourced, never run directly.

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEMO_DIR="$REPO_ROOT/demo"

# Where the two engines are. Both are expected to be running by the time
# 01-setup.sh runs; it detects them rather than starting them. MySQL comes from
# the repository's docker-compose.yml, Doris from 00-doris-up.sh.
MYSQL_PORT=13306
MYSQL_USER=app
MYSQL_PASS=apppw

DORIS_PORT=9030
DORIS_USER=root
DORIS_PASS=""            # Doris ships with root and no password

# The Doris container 00-doris-up.sh manages. Nothing else reads these, but
# they live here so there is one place to point the demo at a different Doris.
DORIS_CONTAINER=doris-demo
DORIS_IMAGE=apache/doris:doris-all-in-one-2.1.0
DORIS_HTTP_PORT=8030     # FE web UI; not used by the demo, useful when it sulks
DORIS_READY_TIMEOUT=${DORIS_READY_TIMEOUT:-900}

# The proxy's two front doors, one per backend.
PROXY_MYSQL_PORT=13307
PROXY_DORIS_PORT=13308

DB=shop
TABLE=orders
FILTER="region = 'EU'"

PROXY_BIN="$REPO_ROOT/target/release/mysql-proxy"
PROXY_CONFIG="$DEMO_DIR/proxy.toml"
PROXY_PIDFILE="$DEMO_DIR/logs/proxy.pid"
MYSQL_LOG="$DEMO_DIR/logs/mysql.jsonl"
DORIS_LOG="$DEMO_DIR/logs/doris.jsonl"

# The MySQL command-line client runs in a container, since the host has none.
# `host.docker.internal` is how it reaches engines and the proxy on the host.
CLIENT_IMAGE=mysql:8.0
HOST_ALIAS=host.docker.internal

# ---------------------------------------------------------------- formatting
if [ -t 1 ]; then
  BOLD=$'\033[1m'; DIM=$'\033[2m'; GREEN=$'\033[32m'; RED=$'\033[31m'
  CYAN=$'\033[36m'; RESET=$'\033[0m'
else
  BOLD=""; DIM=""; GREEN=""; RED=""; CYAN=""; RESET=""
fi

heading() { printf '\n%s%s%s\n' "$BOLD" "$1" "$RESET"; }
ok()      { printf '  %sOK%s   %s\n' "$GREEN" "$RESET" "$1"; }
bad()     { printf '  %sFAIL%s %s\n' "$RED" "$RESET" "$1"; }
info()    { printf '  %s%s%s\n' "$DIM" "$1" "$RESET"; }

# ---------------------------------------------------------------- sql helpers
#
# The password goes through MYSQL_PWD rather than -p. That keeps the client's
# "password on the command line is insecure" warning off stdout, which means no
# filtering pipeline — and therefore no chance of grep's exit status (1 when it
# matches nothing) masquerading as a failed query under `set -o pipefail`.
#
# Nothing here passes `docker run -i` unless it is actually feeding stdin: an
# interactive container whose stdin never reaches EOF will not exit.
MYSQL_ARGS=(--ssl-mode=DISABLED --get-server-public-key --connect-timeout=5)

# run_sql <port> <user> <password> [mysql args...]   — no stdin
run_sql() {
  local port="$1" user="$2" pass="$3"; shift 3
  docker run --rm -e MYSQL_PWD="$pass" "$CLIENT_IMAGE" \
    mysql -h "$HOST_ALIAS" -P "$port" -u "$user" "${MYSQL_ARGS[@]}" "$@" < /dev/null
}

# run_sql_file <port> <user> <password> <file> [mysql args...]
run_sql_file() {
  local port="$1" user="$2" pass="$3" file="$4"; shift 4
  docker run --rm -i -e MYSQL_PWD="$pass" "$CLIENT_IMAGE" \
    mysql -h "$HOST_ALIAS" -P "$port" -u "$user" "${MYSQL_ARGS[@]}" "$@" < "$file"
}

mysql_sql()       { run_sql "$MYSQL_PORT"       "$MYSQL_USER" "$MYSQL_PASS" "$@"; }
doris_sql()       { run_sql "$DORIS_PORT"       "$DORIS_USER" "$DORIS_PASS" "$@"; }
proxy_mysql_sql() { run_sql "$PROXY_MYSQL_PORT" "$MYSQL_USER" "$MYSQL_PASS" "$@"; }
proxy_doris_sql() { run_sql "$PROXY_DORIS_PORT" "$DORIS_USER" "$DORIS_PASS" "$@"; }

mysql_seed() { run_sql_file "$MYSQL_PORT" "$MYSQL_USER" "$MYSQL_PASS" "$DEMO_DIR/seed-mysql.sql"; }
doris_seed() { run_sql_file "$DORIS_PORT" "$DORIS_USER" "$DORIS_PASS" "$DEMO_DIR/seed-doris.sql"; }

# Runs the shared query file against one endpoint, in table form.
run_query_file() {
  local fn="$1"
  local port user pass
  case "$fn" in
    mysql_sql)       port=$MYSQL_PORT;       user=$MYSQL_USER; pass=$MYSQL_PASS ;;
    doris_sql)       port=$DORIS_PORT;       user=$DORIS_USER; pass=$DORIS_PASS ;;
    proxy_mysql_sql) port=$PROXY_MYSQL_PORT; user=$MYSQL_USER; pass=$MYSQL_PASS ;;
    proxy_doris_sql) port=$PROXY_DORIS_PORT; user=$DORIS_USER; pass=$DORIS_PASS ;;
    *) return 1 ;;
  esac
  run_sql_file "$port" "$user" "$pass" "$DEMO_DIR/query.sql" -D "$DB" -t
}

# Counts rows the shared query returns, without the table decoration.
#
# Note what this actually does: it issues its own `SELECT COUNT(*)`, a second
# statement, rather than counting the lines the displayed query printed. That is
# deliberate and it is honest in both directions — through a proxied listener
# the COUNT(*) is itself a single-table read of a ruled table, so it is rewritten
# by the same rule and reports the same three rows the client saw. If it ever
# disagrees with the visible output, the filter is applying inconsistently
# between two statements, and that is worth knowing rather than hiding.
count_rows() {
  local fn="$1"
  "$fn" -D "$DB" -N -B -e "SELECT COUNT(*) FROM $TABLE" | tr -d '[:space:]'
}

# True when the endpoint answers at all.
endpoint_alive() {
  local port="$1" user="$2" pass="$3"
  run_sql "$port" "$user" "$pass" -N -B -e "SELECT 1" >/dev/null 2>&1
}

proxy_running() {
  [ -f "$PROXY_PIDFILE" ] && kill -0 "$(cat "$PROXY_PIDFILE")" 2>/dev/null
}
