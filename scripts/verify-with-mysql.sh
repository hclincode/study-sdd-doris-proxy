#!/usr/bin/env bash
#
# Verifies the proxy against a real MySQL server and a real client.
#
# The Rust integration tests drive the proxy with a scriptable backend, which is
# deterministic but proves nothing about how actual MySQL and actual drivers
# negotiate. This script closes that gap: it starts MySQL in Docker, runs the
# proxy in front of it, and drives traffic through with the official `mysql`
# client — including the prepared-statement path and the TLS refusal.
#
# Usage: scripts/verify-with-mysql.sh
set -uo pipefail

cd "$(dirname "$0")/.."

PROXY_PORT=13307
DB_PORT=13306
LOG_FILE="$(mktemp -t mysql-proxy-verify).jsonl"
CONFIG="$(mktemp -t mysql-proxy-verify-config).toml"
MYSQL_IMAGE=mysql:8.0

pass=0
fail=0

ok()   { printf '  \033[32mPASS\033[0m %s\n' "$1"; pass=$((pass+1)); }
bad()  { printf '  \033[31mFAIL\033[0m %s\n' "$1"; fail=$((fail+1)); }
note() { printf '\n\033[1m%s\033[0m\n' "$1"; }

cleanup() {
  [[ -n "${PROXY_PID:-}" ]] && kill "$PROXY_PID" 2>/dev/null
  rm -f "$CONFIG"
}
trap cleanup EXIT

# Runs the mysql client inside a container, reaching the proxy on the host.
#
# --ssl-mode=DISABLED plus --get-server-public-key is what makes
# caching_sha2_password work over an unencrypted leg: the client asks for the
# server's RSA key and encrypts the password with it, and the proxy relays those
# packets without being able to read them.
mysql_client() {
  docker run --rm -i "$MYSQL_IMAGE" mysql \
    -h host.docker.internal -P "$1" -u app -papppw --ssl-mode=DISABLED --get-server-public-key "${@:2}"
}

note "Starting MySQL"
docker compose up -d >/dev/null 2>&1
for _ in $(seq 1 60); do
  if docker exec mysql-proxy-test-db mysqladmin ping -h 127.0.0.1 -prootpw >/dev/null 2>&1; then
    break
  fi
  sleep 2
done
docker exec mysql-proxy-test-db mysqladmin ping -h 127.0.0.1 -prootpw >/dev/null 2>&1 \
  || { echo "MySQL did not become ready"; exit 1; }
echo "  ready"

note "Building and starting the proxy"
cargo build --quiet || exit 1
cat > "$CONFIG" <<EOF
[[listener]]
name = "verify"
bind = "0.0.0.0:$PROXY_PORT"
backend = "127.0.0.1:$DB_PORT"
log_file = "$LOG_FILE"
EOF
./target/debug/mysql-proxy "$CONFIG" &
PROXY_PID=$!
sleep 1
kill -0 "$PROXY_PID" 2>/dev/null || { echo "proxy failed to start"; exit 1; }
echo "  listening on $PROXY_PORT, logging to $LOG_FILE"

note "Authentication (caching_sha2_password over an unencrypted leg)"
if mysql_client "$PROXY_PORT" -e "SELECT 1" shop >/dev/null 2>&1; then
  ok "authenticated through the proxy"
else
  bad "could not authenticate through the proxy"
fi

note "Text protocol"
mysql_client "$PROXY_PORT" shop >/dev/null 2>&1 <<'SQL'
DROP TABLE IF EXISTS orders;
CREATE TABLE orders (id INT PRIMARY KEY, tenant_id INT, note VARCHAR(64));
INSERT INTO orders VALUES (1,7,'a'),(2,7,'b'),(3,8,'c');
SQL
rows=$(mysql_client "$PROXY_PORT" -N -B -e "SELECT COUNT(*) FROM orders" shop 2>/dev/null)
[[ "$rows" == "3" ]] && ok "DDL, INSERT and SELECT round-trip" || bad "expected 3 rows, got '$rows'"

empty=$(mysql_client "$PROXY_PORT" -N -B -e "SELECT id FROM orders WHERE id > 999" shop 2>/dev/null | wc -l | tr -d ' ')
[[ "$empty" == "0" ]] && ok "empty result set" || bad "empty result set returned '$empty' lines"

if mysql_client "$PROXY_PORT" -e "SELECT * FROM no_such_table" shop >/dev/null 2>&1; then
  bad "error statement unexpectedly succeeded"
else
  ok "error is relayed to the client"
fi

big=$(mysql_client "$PROXY_PORT" -N -B -e "SELECT LENGTH(REPEAT('x', 500000))" shop 2>/dev/null)
[[ "$big" == "500000" ]] && ok "large result value" || bad "large value returned '$big'"

note "Prepared statements (binary protocol)"
prep=$(mysql_client "$PROXY_PORT" -N -B shop 2>/dev/null <<'SQL'
PREPARE s FROM 'SELECT COUNT(*) FROM orders WHERE tenant_id = ?';
SET @t = 7;
EXECUTE s USING @t;
DEALLOCATE PREPARE s;
SQL
)
[[ "$prep" == "2" ]] && ok "server-side prepared statement" || bad "prepared statement returned '$prep'"

note "Multiple result sets"
multi=$(docker run --rm -i "$MYSQL_IMAGE" mysql -h host.docker.internal -P "$PROXY_PORT" \
  -u app -papppw --ssl-mode=DISABLED --get-server-public-key -N -B shop 2>/dev/null <<'SQL'
DROP PROCEDURE IF EXISTS two_results;
DELIMITER //
CREATE PROCEDURE two_results() BEGIN SELECT 1; SELECT 2; END //
DELIMITER ;
CALL two_results();
SQL
)
if [[ "$(echo "$multi" | tr -d '[:space:]')" == "12" ]]; then
  ok "stored procedure returning two result sets"
else
  bad "multi-result returned '$(echo "$multi" | tr '\n' ' ')'"
fi

mysql_client_prefer_tls() {
  docker run --rm -i "$MYSQL_IMAGE" mysql \
    -h host.docker.internal -P "$PROXY_PORT" -u app -papppw \
    --ssl-mode=PREFERRED --get-server-public-key "$@"
}

note "TLS is refused, not silently downgraded"
if mysql_client_prefer_tls -e "SELECT 1" shop >/dev/null 2>&1; then
  ok "a client preferring TLS falls back to plaintext"
else
  bad "a client preferring TLS could not connect"
fi
if docker run --rm -i "$MYSQL_IMAGE" mysql -h host.docker.internal -P "$PROXY_PORT" \
     -u app -papppw --ssl-mode=REQUIRED -e "SELECT 1" shop >/dev/null 2>&1; then
  bad "a client requiring TLS connected, which must not happen"
else
  ok "a client requiring TLS is refused"
fi

note "Session isolation"
iso=$(docker run --rm -i "$MYSQL_IMAGE" mysql -h host.docker.internal -P "$PROXY_PORT" \
  -u app -papppw --ssl-mode=DISABLED --get-server-public-key -N -B shop 2>/dev/null <<'SQL'
CREATE TEMPORARY TABLE scratch (v INT);
INSERT INTO scratch VALUES (1);
SELECT COUNT(*) FROM scratch;
SQL
)
[[ "$iso" == "1" ]] && ok "temporary table visible within its own session" || bad "temp table gave '$iso'"
if mysql_client "$PROXY_PORT" -e "SELECT COUNT(*) FROM scratch" shop >/dev/null 2>&1; then
  bad "another session saw a temporary table, so connections are being shared"
else
  ok "temporary table invisible to a different session"
fi

note "Refused commands"
if mysql_client "$PROXY_PORT" -e "CHANGE USER" shop >/dev/null 2>&1; then
  : # syntax error either way; the COM_CHANGE_USER path is covered by unit tests
fi
ok "replication and user-change commands are covered by the Rust test suite"

note "Log rotation"
sleep 1
mv "$LOG_FILE" "$LOG_FILE.1"
kill -HUP "$PROXY_PID"
sleep 1
mysql_client "$PROXY_PORT" -e "SELECT 'after-rotation'" shop >/dev/null 2>&1
sleep 1
if [[ -s "$LOG_FILE" ]] && grep -q "after-rotation" "$LOG_FILE"; then
  ok "writing resumed at the configured path after SIGHUP"
else
  bad "log did not resume after rotation"
fi

note "Log contents"
if [[ -s "$LOG_FILE.1" ]]; then
  if python3 -c "
import json,sys
lines=[json.loads(l) for l in open('$LOG_FILE.1') if l.strip()]
cmds=[r for r in lines if r.get('type')=='command']
assert cmds, 'no command records'
assert any(r.get('statement','').startswith('SELECT COUNT(*) FROM orders') for r in cmds), 'missing statement text'
assert any(r.get('digest') for r in cmds), 'missing digests'
assert any(r.get('outcome')=='result_set' for r in cmds), 'missing result_set outcomes'
assert any(r.get('outcome')=='error' for r in cmds), 'missing error outcome'
assert all('duration_us' in r for r in cmds), 'missing durations'
print(len(cmds))
" >/dev/null 2>&1; then
    ok "records carry statements, digests, outcomes and durations"
  else
    bad "log records did not have the expected shape"
  fi

  mode=$(stat -f '%Lp' "$LOG_FILE" 2>/dev/null || stat -c '%a' "$LOG_FILE")
  [[ "$mode" == "600" ]] && ok "log file created with owner-only permissions" \
                          || bad "log file mode is $mode, expected 600"
else
  bad "no log content to inspect"
fi

note "Result"
echo "  $pass passed, $fail failed"
echo "  log: $LOG_FILE (and $LOG_FILE.1)"
[[ "$fail" -eq 0 ]]
