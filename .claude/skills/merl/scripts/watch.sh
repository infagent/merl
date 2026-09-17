#!/usr/bin/env bash
# Poll for inbound messages this identity has not yet seen. Prints each new filename.
# --once: exit 0 as soon as at least one new message exists (for a background wake-up);
#         exit 3 when --timeout passes with nothing.
# --stop: stop this identity's running watcher, if any, and exit. The only correct way to
#         stop a watcher by hand. Never pkill by script name: every agent's watcher runs
#         this same file, and one pkill has already taken down all of them at once.
# One watcher per identity: arming a new one replaces the old one, so a restart never
# needs a kill at all. Seen state lives in DIR/.seen-ME and the pid in DIR/.watch-ME.pid;
# both are dotfiles, so neither is ever mistaken for a message.
# Usage: watch.sh [--me ME] [--interval SECONDS] [--once] [--timeout SECONDS] [--stop] [--dir DIR]
set -euo pipefail
dir="${AGENT_MAIL_DIR:-$HOME/projects/messages}"; me="${AGENT_MAIL_IDENTITY:-}"; interval=30; once=0; timeout=0; stop=0
while [ $# -gt 0 ]; do case "$1" in
  --me) me=$2; shift 2;; --interval) interval=$2; shift 2;; --once) once=1; shift;;
  --timeout) timeout=$2; shift 2;; --stop) stop=1; shift;; --dir) dir=$2; shift 2;;
  *) echo "unknown flag $1" >&2; exit 2;; esac; done
[ -n "$me" ] || { echo "--me is required, or set AGENT_MAIL_IDENTITY" >&2; exit 2; }
mkdir -p "$dir"; seen="$dir/.seen-$me"; pidfile="$dir/.watch-$me.pid"; touch "$seen"

previous_stop() {
  [ -f "$pidfile" ] || return 0
  local old; old=$(cat "$pidfile" 2>/dev/null || true)
  if [ -n "$old" ] && [ "$old" != "$$" ] && kill -0 "$old" 2>/dev/null; then
    kill "$old" 2>/dev/null || true
    # Wait for it to be gone before writing our own pid, or its exit trap races ours.
    for _ in $(seq 1 50); do kill -0 "$old" 2>/dev/null || break; sleep 0.1; done
    echo "stopped previous watcher for $me (pid $old)" >&2
  fi
  rm -f "$pidfile"
}

if [ $stop -eq 1 ]; then previous_stop; exit 0; fi
previous_stop
echo $$ > "$pidfile"
# Remove the pidfile only if it is still ours; a replacement may have written its pid.
trap '[ "$(cat "$pidfile" 2>/dev/null)" = "$$" ] && rm -f "$pidfile"' EXIT

start=$(date +%s)
while :; do
  found=0
  for f in "$dir"/*.md; do
    [ -e "$f" ] || continue
    b=$(basename "$f")
    # Anchored on the stamp so a hyphen-suffix identity never matches another's files.
    [[ "$b" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{6}-(.+)-to-${me}\.md$ ]] || continue
    if ! grep -qxF "$b" "$seen"; then echo "$b" >> "$seen"; echo "$f"; found=1; fi
  done
  if [ $found -eq 1 ] && [ $once -eq 1 ]; then exit 0; fi
  if [ "$timeout" -gt 0 ] && [ $(( $(date +%s) - start )) -ge "$timeout" ]; then exit 3; fi
  sleep "$interval"
done
