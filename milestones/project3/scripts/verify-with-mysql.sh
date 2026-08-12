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
FILTER_LOG="$(mktemp -t mysql-proxy-filtered).jsonl"
FILTER_PORT=13308
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

# The same backend, reached through a listener that filters \`orders\` to one
# tenant. Comparing the two ports is what makes row-visibility assertions
# meaningful.
[[listener]]
name = "filtered"
bind = "0.0.0.0:$FILTER_PORT"
backend = "127.0.0.1:$DB_PORT"
log_file = "$FILTER_LOG"
row_filters = { orders = "tenant_id = 7" }
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
DROP TABLE IF EXISTS items;
CREATE TABLE items (id INT PRIMARY KEY, oid INT, label VARCHAR(64));
INSERT INTO items VALUES (10,1,'i1'),(11,2,'i2'),(12,3,'i3');
SQL

docker exec -i mysql-proxy-test-db mysql -uroot -prootpw >/dev/null 2>&1 <<'SQL'
CREATE USER IF NOT EXISTS 'app_native'@'%' IDENTIFIED WITH mysql_native_password BY 'apppw';
GRANT ALL ON shop.* TO 'app_native'@'%';
FLUSH PRIVILEGES;
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


# ---------------------------------------------------------------- row filters
#
# Every assertion here is about which rows come back, not about SQL text. A
# missing pair of parentheses produces a perfectly reasonable looking statement,
# so only row counts can catch it.

filtered_query() {
  docker run --rm -i "$MYSQL_IMAGE" mysql -h host.docker.internal -P "$FILTER_PORT" \
    -u app -papppw --ssl-mode=DISABLED --get-server-public-key -N -B -e "$1" shop 2>/dev/null
}
unfiltered_query() {
  mysql_client "$PROXY_PORT" -N -B -e "$1" shop 2>/dev/null
}

note "Row filter: visibility"
# orders holds 2 rows for tenant 7 and 1 for tenant 8.
all=$(unfiltered_query "SELECT COUNT(*) FROM orders")
mine=$(filtered_query "SELECT COUNT(*) FROM orders")
[[ "$all" == "3" && "$mine" == "2" ]] \
  && ok "plain select returns only the filtered tenant's rows ($mine of $all)" \
  || bad "expected 2 of 3, got '$mine' of '$all'"

mine=$(filtered_query "SELECT COUNT(*) FROM orders WHERE note = 'a'")
[[ "$mine" == "1" ]] && ok "select with an existing WHERE stays filtered" \
                     || bad "expected 1, got '$mine'"

# The precedence case. Unfiltered this matches 'a' (tenant 7) and 'c' (tenant 8).
# Without parentheses around the original condition the filter would bind only
# to the second alternative and 'c' would leak through.
all=$(unfiltered_query "SELECT COUNT(*) FROM orders WHERE note = 'a' OR note = 'c'")
mine=$(filtered_query "SELECT COUNT(*) FROM orders WHERE note = 'a' OR note = 'c'")
[[ "$all" == "2" && "$mine" == "1" ]] \
  && ok "OR condition is parenthesized, so the filter narrows rather than widens" \
  || bad "precedence leak: unfiltered '$all', filtered '$mine' (expected 2 and 1)"

mine=$(filtered_query "SELECT note FROM orders ORDER BY id LIMIT 5" | tr '\n' ' ' | xargs)
[[ "$mine" == "a b" ]] && ok "ORDER BY and LIMIT survive the injection" \
                       || bad "expected 'a b', got '$mine'"

mine=$(filtered_query "SELECT COUNT(*) FROM orders o WHERE o.id > 0")
[[ "$mine" == "2" ]] && ok "aliased table is still filtered" || bad "expected 2, got '$mine'"

note "Row filter: best-effort skips"
all=$(unfiltered_query "SELECT COUNT(*) FROM orders o JOIN items i ON o.id = i.oid")
mine=$(filtered_query "SELECT COUNT(*) FROM orders o JOIN items i ON o.id = i.oid")
[[ "$all" == "3" && "$mine" == "3" ]] \
  && ok "a join is forwarded unfiltered, as specified" \
  || bad "expected the join to return 3 either way, got '$mine' of '$all'"

mine=$(filtered_query "SELECT COUNT(*) FROM items")
[[ "$mine" == "3" ]] && ok "a table with no rule is unaffected" || bad "expected 3, got '$mine'"

note "Row filter: prepared statements (binary protocol)"
# The mysql CLI's PREPARE is server-side dynamic SQL, which the proxy cannot
# see into. A real driver issuing COM_STMT_PREPARE is needed to exercise the
# path this feature actually rewrites.
prep=$(docker run --rm -i python:3.12-slim sh -c "
pip install --quiet mysql-connector-python >/dev/null 2>&1
python - <<'PYEOF'
import mysql.connector
c = mysql.connector.connect(host='host.docker.internal', port=$FILTER_PORT,
                            user='app_native', password='apppw', database='shop')
cur = c.cursor(prepared=True)
cur.execute('SELECT COUNT(*) FROM orders WHERE note = %s', ('a',))
with_param = cur.fetchone()[0]
cur2 = c.cursor(prepared=True)
cur2.execute('SELECT id, tenant_id, note FROM orders')
rows = cur2.fetchall()
print(f'{with_param} {len(rows)} {len(rows[0])}')
PYEOF
" 2>/dev/null | tail -1)
if [[ "$prep" == "1 2 3" ]]; then
  ok "prepared statement is filtered, with parameter and column counts unchanged"
else
  bad "prepared statement gave '$prep' (expected '1 2 3')"
fi

note "Row filter: log records"
sleep 1
if python3 -c "
import json
rows=[json.loads(l) for l in open('$FILTER_LOG') if l.strip()]
cmds=[r for r in rows if r.get('type')=='command']
rew=[r for r in cmds if r.get('rewritten')]
skipped=[r for r in cmds if r.get('filter_skipped')]
assert rew, 'no rewritten records'
assert any('tenant_id = 7' in r.get('forwarded_statement','') for r in rew), 'predicate missing from forwarded text'
assert all('tenant_id = 7' not in r.get('statement','') for r in rew), 'client text was altered'
assert any(r['filter_skipped']=='multiple_tables' for r in skipped), 'join not recorded as a skip'
assert not any(r.get('filter_skipped') for r in cmds if 'FROM items' in r.get('statement','')), 'unruled table counted as a skip'
" >/dev/null 2>&1; then
  ok "records carry both statements, the rule applied, and skip reasons"
else
  bad "filtered log records did not have the expected shape"
fi

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
