#!/usr/bin/env bash
# Print every inbound message this identity has not read yet, oldest first, then mark
# them read in DIR/.read-ME. Independent of the watcher's .seen file on purpose: the
# watcher marks a file the moment it prints a filename, which is not the same as the
# agent having read it. Two messages landing together, or a wake-up handled by reading
# only the newest file, are what this exists for. Replies and acks show here even
# though inbox.sh never lists them as open.
# Usage: read.sh [--me ME] [--from SENDER] [--peek] [--dir DIR]
#   --peek prints without marking read.
set -euo pipefail
dir="${AGENT_MAIL_DIR:-$HOME/projects/messages}"; me="${AGENT_MAIL_IDENTITY:-}"; from=""; peek=0
while [ $# -gt 0 ]; do case "$1" in
  --me) me=$2; shift 2;; --from) from=$2; shift 2;; --peek) peek=1; shift;; --dir) dir=$2; shift 2;;
  *) echo "unknown flag $1" >&2; exit 2;; esac; done
[ -n "$me" ] || { echo "--me is required, or set AGENT_MAIL_IDENTITY" >&2; exit 2; }
[ -d "$dir" ] || exit 0
marker="$dir/.read-$me"; touch "$marker"
count=0
for f in "$dir"/*.md; do
  [ -e "$f" ] || continue
  b=$(basename "$f")
  [[ "$b" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{6}-(.+)-to-${me}\.md$ ]] || continue
  sender=${BASH_REMATCH[1]}
  [ -z "$from" ] || [ "$sender" = "$from" ] || continue
  grep -qxF "$b" "$marker" && continue
  echo "===== $b"; cat "$f"; echo
  [ $peek -eq 1 ] || echo "$b" >> "$marker"
  count=$((count+1))
done
[ $count -gt 0 ] || echo "(no unread messages for $me)"
