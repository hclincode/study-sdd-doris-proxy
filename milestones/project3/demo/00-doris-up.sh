#!/usr/bin/env bash
#
# Brings up the Apache Doris container the demo needs, and waits until it is
# genuinely usable — not merely accepting connections.
#
# This is the one slow step in the whole demo. The image is ~11 GB and Doris
# needs several minutes after launch before it will accept a CREATE TABLE, so
# run this well before anything else. `01-setup.sh` deliberately does not start
# or wait for an engine; this script is where that waiting lives.
#
#   ./demo/00-doris-up.sh             start (or reuse) and wait until ready
#   ./demo/00-doris-up.sh --status    report, change nothing
#   ./demo/00-doris-up.sh --no-wait   start and return immediately
#   ./demo/00-doris-up.sh --stop      stop the container, keep its data
#   ./demo/00-doris-up.sh --rm        stop and delete it, data included
#
# Then:  docker compose up -d  (MySQL)  and  ./demo/01-setup.sh
set -uo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/env.sh"

usage() { sed -n '3,17p' "${BASH_SOURCE[0]}" | sed 's/^#\{1,2\} \{0,1\}//'; }

# --------------------------------------------------------------- inspection
#
# Three questions, deliberately separate: does a container by that name exist,
# is it running, and does anything at all answer on the port. They can disagree
# — a container can be up while Doris inside it is still starting, and the port
# can be answered by a Doris this script did not start.

container_state() {   # prints: running | exited | absent
  # `docker inspect` on a name it does not know writes an empty line to stdout
  # before failing, so its output is captured and tested rather than passed
  # through — an empty answer is the absent case, and a bare `|| echo absent`
  # would emit that blank line ahead of the word.
  local state
  state="$(docker inspect --format '{{.State.Status}}' "$DORIS_CONTAINER" 2>/dev/null)"
  printf '%s\n' "${state:-absent}"
}

# Whichever container publishes DORIS_PORT, if any. Empty when nothing does.
port_holder() {
  docker ps --filter "publish=$DORIS_PORT" --format '{{.Names}}' 2>/dev/null | head -1
}

# Doris answers the MySQL protocol long before it can serve a CREATE TABLE: the
# frontend accepts connections while no backend has registered yet, and a table
# with replication_num=1 fails with "Failed to find enough backend" until one
# has. Seeding is the first thing that needs it, so waiting for a live backend
# here is what stops 01-setup.sh from failing on a Doris that looked ready.
#
# The Alive column is located by name from the header row rather than by index,
# so a future Doris release reordering SHOW BACKENDS does not silently turn this
# check into a coin flip.
backend_alive() {
  doris_sql -B -e 'SHOW BACKENDS' 2>/dev/null | awk -F'\t' '
    NR == 1 { for (i = 1; i <= NF; i++) if ($i == "Alive") col = i; next }
    col && $col == "true" { alive++ }
    END { exit !(alive > 0) }
  '
}

doris_ready() {
  endpoint_alive "$DORIS_PORT" "$DORIS_USER" "$DORIS_PASS" && backend_alive
}

report_status() {
  local state holder
  state="$(container_state)"
  holder="$(port_holder)"

  printf '  %-22s %s\n' "container" "$DORIS_CONTAINER ($state)"
  printf '  %-22s %s\n' "image" "$DORIS_IMAGE"
  printf '  %-22s %s\n' "port $DORIS_PORT" "${holder:-nothing published}"

  if endpoint_alive "$DORIS_PORT" "$DORIS_USER" "$DORIS_PASS"; then
    if backend_alive; then
      ok "Doris is ready on $DORIS_PORT — run ./demo/01-setup.sh"
      return 0
    fi
    bad "frontend answers on $DORIS_PORT but no backend is alive yet"
    info "this is the normal state for the first few minutes; keep waiting"
    return 1
  fi

  bad "nothing answers on $DORIS_PORT"
  return 1
}

# ------------------------------------------------------------------ actions
case "${1:-}" in
  -h|--help)
    usage
    exit 0
    ;;
  --status)
    heading "Doris status"
    report_status
    exit $?
    ;;
  --stop)
    heading "Stopping Doris"
    if [ "$(container_state)" = "running" ]; then
      docker stop "$DORIS_CONTAINER" > /dev/null && ok "$DORIS_CONTAINER stopped; its data is kept"
      info "start it again with: ./demo/00-doris-up.sh"
    else
      ok "$DORIS_CONTAINER is not running"
    fi
    exit 0
    ;;
  --rm)
    heading "Removing Doris"
    if [ "$(container_state)" = absent ]; then
      ok "$DORIS_CONTAINER does not exist"
    else
      docker rm -f "$DORIS_CONTAINER" > /dev/null && ok "$DORIS_CONTAINER removed — its data is gone"
      info "the ${DORIS_IMAGE##*:} image is kept; delete it with: docker rmi $DORIS_IMAGE"
    fi
    exit 0
    ;;
  --no-wait) WAIT=0 ;;
  "")        WAIT=1 ;;
  *)
    bad "unknown option: $1"
    usage
    exit 2
    ;;
esac

heading "Bringing up Doris"

if ! docker info > /dev/null 2>&1; then
  bad "Docker is not running"
  exit 1
fi

# Something already serving the port is a success, not a conflict — the demo
# needs a Doris on DORIS_PORT and does not care who started it. Say whose it is,
# because that determines whether --stop and --rm here will affect it.
holder="$(port_holder)"
if [ -n "$holder" ] && [ "$holder" != "$DORIS_CONTAINER" ]; then
  ok "port $DORIS_PORT is already published by container '$holder'"
  info "using it as-is; this script's --stop and --rm will not touch it"
elif endpoint_alive "$DORIS_PORT" "$DORIS_USER" "$DORIS_PASS" && [ -z "$holder" ]; then
  ok "something outside Docker is already serving $DORIS_PORT"
  info "using it as-is"
else
  case "$(container_state)" in
    running)
      ok "$DORIS_CONTAINER is already running"
      ;;
    exited)
      info "starting existing container $DORIS_CONTAINER"
      if ! docker start "$DORIS_CONTAINER" > /dev/null; then
        bad "could not start $DORIS_CONTAINER"
        info "inspect it with: docker logs $DORIS_CONTAINER"
        exit 1
      fi
      ok "$DORIS_CONTAINER started — its existing data is intact"
      ;;
    absent)
      if ! docker image inspect "$DORIS_IMAGE" > /dev/null 2>&1; then
        info "pulling $DORIS_IMAGE — about 11 GB, this takes a while"
      fi
      info "creating container $DORIS_CONTAINER"
      # TZ=UTC so the timestamps Doris logs line up with the proxy's, which are
      # UTC by construction. FE query port and FE web UI are the only ports the
      # demo touches; the all-in-one image runs FE and BE in one container and
      # they reach each other inside it.
      if ! docker run -d \
          --name "$DORIS_CONTAINER" \
          -e TZ=UTC \
          -p "$DORIS_PORT:9030" \
          -p "$DORIS_HTTP_PORT:8030" \
          "$DORIS_IMAGE" > /dev/null; then
        bad "could not create $DORIS_CONTAINER"
        exit 1
      fi
      ok "$DORIS_CONTAINER created"
      ;;
    *)
      # created, paused, restarting, dead. None of these becomes ready by
      # waiting, and guessing which recovery each one wants is how a demo
      # script starts destroying containers on its own initiative.
      bad "$DORIS_CONTAINER is in state '$(container_state)' — not a state this script handles"
      info "look at it with: docker ps -a --filter name=$DORIS_CONTAINER"
      info "or start over with: ./demo/00-doris-up.sh --rm && ./demo/00-doris-up.sh"
      exit 1
      ;;
  esac
fi

if [ "$WAIT" -eq 0 ]; then
  heading "Not waiting"
  info "Doris needs several minutes before it accepts a CREATE TABLE."
  info "check with: ./demo/00-doris-up.sh --status"
  exit 0
fi

# ----------------------------------------------------------------- waiting
heading "Waiting for Doris to become usable"
info "frontend accepting connections, then at least one live backend"
info "this normally takes 2-5 minutes on a first start; Ctrl-C is safe"

started=$SECONDS
fe_seen=0

while true; do
  elapsed=$((SECONDS - started))

  if [ "$fe_seen" -eq 0 ] && endpoint_alive "$DORIS_PORT" "$DORIS_USER" "$DORIS_PASS"; then
    fe_seen=1
    ok "frontend answering on $DORIS_PORT after ${elapsed}s"
    info "waiting for a backend to register"
  fi

  if [ "$fe_seen" -eq 1 ] && backend_alive; then
    ok "backend alive after ${elapsed}s"
    break
  fi

  if [ "$elapsed" -ge "$DORIS_READY_TIMEOUT" ]; then
    bad "Doris did not become ready within ${DORIS_READY_TIMEOUT}s"
    info "it may just be slow — re-run this script, or watch it with:"
    info "  docker logs -f $DORIS_CONTAINER"
    info "raise the limit with: DORIS_READY_TIMEOUT=1800 ./demo/00-doris-up.sh"
    exit 1
  fi

  # Only the elapsed counter is reprinted, on one line, so a long wait does not
  # scroll the reasons for it off the screen.
  printf '\r  %s%4ds elapsed%s' "$DIM" "$elapsed" "$RESET"
  sleep 5
done
printf '\r%*s\r' 20 ''

heading "Ready"
ok "Doris $DORIS_USER@127.0.0.1:$DORIS_PORT — web UI on http://127.0.0.1:$DORIS_HTTP_PORT"
info "next:  docker compose up -d      # MySQL, the other engine"
info "       ./demo/01-setup.sh        # seed both, start the proxy, verify"
