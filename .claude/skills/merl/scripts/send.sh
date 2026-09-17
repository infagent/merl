#!/usr/bin/env bash
# Send one message: header from flags, body from stdin, written atomically.
# Usage: send.sh [--from ME] --to THEM --type TYPE [--re "#12 #34"] [--in-reply-to FILE] [--dir DIR] < body.txt
set -euo pipefail
dir="${AGENT_MAIL_DIR:-$HOME/projects/messages}"; from="${AGENT_MAIL_IDENTITY:-}"; to=""; type=""; re="none"; reply="none"
while [ $# -gt 0 ]; do case "$1" in
  --from) from=$2; shift 2;; --to) to=$2; shift 2;; --type) type=$2; shift 2;;
  --re) re=$2; shift 2;; --in-reply-to) reply=$2; shift 2;; --dir) dir=$2; shift 2;;
  *) echo "unknown flag $1" >&2; exit 2;; esac; done
[ -n "$from" ] && [ -n "$to" ] && [ -n "$type" ] || { echo "--from (or AGENT_MAIL_IDENTITY), --to and --type are required" >&2; exit 2; }
case "$type" in assign|ready|review|blocked|question|reply|ack) ;; *) echo "type must be assign|ready|review|blocked|question|reply|ack" >&2; exit 2;; esac
if [ "$reply" != "none" ]; then
  reply=$(basename "$reply")
  [ -f "$dir/$reply" ] || { echo "in-reply-to $reply does not exist in $dir" >&2; exit 2; }
  # An answer closes the message it names, so it must go back to whoever wrote that
  # message. Forwarding a third party's message references it in --re instead.
  author=$(sed -n 's/^From: //p' "$dir/$reply" | head -1)
  [ "$author" = "$to" ] || { echo "in-reply-to $reply was written by $author, not by $to; a reply must go to its author (reference it in --re to forward)" >&2; exit 2; }
fi
mkdir -p "$dir"
stamp=$(date +%Y-%m-%dT%H%M%S); iso=$(date +%Y-%m-%dT%H:%M:%S)
name="$stamp-$from-to-$to.md"
while [ -e "$dir/$name" ]; do sleep 1; stamp=$(date +%Y-%m-%dT%H%M%S); name="$stamp-$from-to-$to.md"; done
tmp="$dir/.$name.tmp"
{ printf 'From: %s\nTo: %s\nDate: %s\nType: %s\nRe: %s\nIn-Reply-To: %s\n\n' "$from" "$to" "$iso" "$type" "$re" "$reply"; cat; } > "$tmp"
mv "$tmp" "$dir/$name"
echo "$dir/$name"
