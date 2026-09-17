#!/usr/bin/env bash
# List inbound messages for an identity. Default: only open ones, meaning requests
# (assign|ready|review|blocked|question) that no file of mine names in In-Reply-To.
# Inbound reply and ack files are answers and are never open.
# Usage: inbox.sh [--me ME] [--from SENDER] [--all] [--dir DIR]
set -euo pipefail
dir="${AGENT_MAIL_DIR:-$HOME/projects/messages}"; me="${AGENT_MAIL_IDENTITY:-}"; all=0; from=""
while [ $# -gt 0 ]; do case "$1" in
  --me) me=$2; shift 2;; --from) from=$2; shift 2;; --all) all=1; shift;; --dir) dir=$2; shift 2;;
  *) echo "unknown flag $1" >&2; exit 2;; esac; done
[ -n "$me" ] || { echo "--me is required, or set AGENT_MAIL_IDENTITY" >&2; exit 2; }
[ -d "$dir" ] || exit 0
# Names are <YYYY-MM-DD>T<HHMMSS>-<from>-to-<to>.md. Anchoring on the fixed-width stamp is
# what keeps an identity that is a hyphen-suffix of another (dev, lumen-dev) from matching
# the other's files.
stamp='[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{6}'
answered=""
for f in "$dir"/*.md; do
  [ -e "$f" ] || continue
  b=$(basename "$f")
  if [[ "$b" =~ ^${stamp}-${me}-to-(.+)\.md$ ]]; then
    answered+=$(sed -n 's/^In-Reply-To: //p' "$f" | head -1)$'\n'
  fi
done
for f in "$dir"/*.md; do
  [ -e "$f" ] || continue
  b=$(basename "$f")
  [[ "$b" =~ ^${stamp}-(.+)-to-${me}\.md$ ]] || continue
  sender=${BASH_REMATCH[1]}
  [ -z "$from" ] || [ "$sender" = "$from" ] || continue
  type=$(sed -n 's/^Type: //p' "$f" | head -1)
  case "$type" in reply|ack) is_request=0;; *) is_request=1;; esac
  if [ $all -eq 1 ] || { [ $is_request -eq 1 ] && ! grep -qxF "$b" <<<"$answered"; }; then
    printf '%s\t%s\t%s\t%s\n' "$b" "$sender" "$type" "$(sed -n 's/^Re: //p' "$f" | head -1)"
  fi
done
