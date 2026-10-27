#!/bin/sh
# Release packaging replaces this default with its immutable tag. The source
# copy resolves latest once, then downloads every asset through that exact tag.
set -eu

DEFAULT_VERSION=latest
version=$DEFAULT_VERSION
bin_dir=${HOME:?HOME must name your user directory}/.local/bin
downloads=
staged=
releases=https://github.com/infagent/merl/releases
setup_mode=offer
unattended=no

fail() { printf 'merl-install: %s\n' "$*" >&2; exit 1; }
cleanup() {
    if [ -n "$staged" ]; then rm -f "$staged"; fi
    if [ -n "$downloads" ]; then rm -rf "$downloads"; fi
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

while [ "$#" -gt 0 ]; do
    case $1 in
        --setup|--no-setup)
            [ "$setup_mode" = offer ] || fail 'choose --setup or --no-setup once'
            case $1 in --setup) setup_mode=yes;; --no-setup) setup_mode=no;; esac
            shift
            ;;
        --unattended) unattended=yes; shift;;
        --) shift; break;;
        --version|--bin-dir)
            [ "$#" -ge 2 ] || fail "$1 needs a value"
            case $1 in --version) version=$2;; --bin-dir) bin_dir=$2;; esac
            shift 2
            ;;
        --help|-h)
            printf '%s\n' 'Usage: sh install.sh [--version vMAJOR.MINOR.PATCH] [--bin-dir /absolute/directory] [--setup|--no-setup] [--unattended] [-- INIT_ARGS...]' \
                'Default: latest release into ~/.local/bin, then reuse saved setup or offer first setup on the controlling terminal.' \
                'Without a terminal, or with --unattended, default to binary-only. --setup explicitly invokes merl init.' \
                'Unattended setup needs --yes (or --show/--dry-run); forward choices after --. No sudo or service installation.'
            exit 0
            ;;
        *) fail "unknown option: $1";;
    esac
done
[ "$#" -eq 0 ] || [ "$setup_mode" = yes ] || fail 'initializer arguments require --setup'

case $bin_dir in /*) ;; *) fail '--bin-dir must be an absolute path';; esac
destination=$bin_dir/merl
check_destination() {
    [ ! -L "$bin_dir" ] || fail "installation directory is a symlink: $bin_dir"
    [ ! -e "$bin_dir" ] || [ -d "$bin_dir" ] || fail "not a directory: $bin_dir"
    [ ! -L "$destination" ] || fail "destination is a symlink: $destination"
    [ ! -e "$destination" ] || [ -f "$destination" ] || fail "destination is not a regular file: $destination"
}
check_destination

case $(uname -s):$(uname -m) in
    Linux:x86_64) target=x86_64-unknown-linux-gnu;;
    Linux:aarch64|Linux:arm64) target=aarch64-unknown-linux-gnu;;
    Darwin:x86_64) target=x86_64-apple-darwin;;
    Darwin:arm64|Darwin:aarch64) target=aarch64-apple-darwin;;
    *) fail 'unsupported platform; see docs/installation.md for supported systems';;
esac
for tool in curl tar mktemp awk grep chmod mv; do
    command -v "$tool" >/dev/null 2>&1 || fail "required tool missing: $tool"
done
if command -v sha256sum >/dev/null 2>&1; then
    checksum() { sha256sum "$1" | awk '{print $1}'; }
elif command -v shasum >/dev/null 2>&1; then
    checksum() { shasum -a 256 "$1" | awk '{print $1}'; }
else
    fail 'SHA256 verification needs sha256sum or shasum'
fi
fetch() {
    curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
        --retry 2 --connect-timeout 15 --max-time 300 --output "$2" "$1" \
        || fail "download failed: $1"
}
if [ "$version" = latest ]; then
    resolved=$(curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
        --retry 2 --connect-timeout 15 --max-time 60 --output /dev/null \
        --write-out '%{url_effective}' "$releases/latest") || fail 'cannot resolve latest release'
    case $resolved in
        "$releases/tag/"*) version=${resolved#"$releases/tag/"};;
        *) fail 'latest release did not resolve to an immutable Merl tag';;
    esac
fi
case $version in *[!0-9.v]*) fail 'version must be a stable tag such as v0.2.0';; esac
printf '%s\n' "$version" | LC_ALL=C grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$' \
    || fail 'version must be a stable tag such as v0.2.0'

downloads=$(mktemp -d "${TMPDIR:-/tmp}/merl-install.XXXXXXXX") || fail 'cannot create download directory'
asset=merl-$version-$target.tar.gz
fetch "$releases/download/$version/SHA256SUMS" "$downloads/SHA256SUMS"
expected=$(awk -v name="$asset" '$2 == name {print $1}' "$downloads/SHA256SUMS")
# Reject missing, duplicate and malformed entries before trusting the archive.
if [ "${#expected}" -ne 64 ] || ! printf '%s\n' "$expected" | LC_ALL=C grep -Eq '^[0-9a-f]{64}$'; then
    fail "missing or invalid SHA256 entry for $asset"
fi
fetch "$releases/download/$version/$asset" "$downloads/$asset"
actual=$(checksum "$downloads/$asset") || fail 'cannot compute archive SHA256'
[ "$actual" = "$expected" ] || fail "SHA256 mismatch for $asset; existing installation preserved"

mkdir -p "$bin_dir" || fail "cannot create installation directory: $bin_dir"
check_destination
# Stage on the destination filesystem so replacement is a single rename. Extract
# only the binary to stdout; archive paths never select filesystem destinations.
staged=$(mktemp "$bin_dir/.merl-install.XXXXXXXX") || fail "cannot write to $bin_dir"
tar -xzOf "$downloads/$asset" merl/merl > "$staged" || fail 'cannot extract release binary'
chmod 755 "$staged" || fail 'cannot mark release binary executable'
reported=$("$staged" --version) || fail 'release binary cannot run on this system; check OS/libc requirements'
[ "$reported" = "merl ${version#v}" ] || fail 'release binary version does not match selected tag'
check_destination
mv -f "$staged" "$destination" || fail "cannot replace $destination; existing installation preserved"
staged=
printf 'Installed merl %s to %s\n' "${version#v}" "$destination"
case :${PATH:-}: in
    *:"$bin_dir":*) ;;
    *) printf 'Add %s to PATH in your shell configuration to use merl by name.\n' "$bin_dir";;
esac
printf '%s\n' 'Verify: merl --version; merl help' \
    'Upgrade: rerun the latest release installer. Binary uninstall: remove this merl binary.'

# Script stdin may still contain the remainder of a curl|sh pipeline. Only the
# controlling terminal may answer the offer; init uses that terminal separately.
terminal=no
if [ "$unattended" = no ] && ( : < /dev/tty ) 2>/dev/null; then terminal=yes; fi
if [ "$setup_mode" = offer ]; then
    if [ "$terminal" = yes ] && "$destination" init --offer --help >/dev/null 2>&1; then
        # The initializer owns saved-state assessment and the one apply decision.
        "$destination" init --offer || fail "verified binary remains installed at $destination; setup incomplete. Retry $destination init with the same choices."
        setup_mode=no
    elif [ "$terminal" = yes ]; then
        printf 'Run guided setup now? [y/N]: ' > /dev/tty
        answer=
        IFS= read -r answer < /dev/tty || answer=
        case $answer in y|Y|yes|YES) setup_mode=yes;; *) setup_mode=no;; esac
    elif [ "$unattended" = yes ]; then
        printf '%s\n' 'Unattended installation: binary only. Run merl init later, or use --setup -- --host HOST --home PATH --yes.'
    else
        printf '%s\n' 'No controlling terminal: binary only. Run merl init later, or pass --setup -- --host HOST --home PATH --yes.'
    fi
fi
if [ "$setup_mode" = yes ]; then
    if [ "$terminal" = no ]; then
        safe_input=no
        needs_value=no
        for argument do
            if [ "$needs_value" = yes ]; then needs_value=no; continue; fi
            # Match init's argv boundaries: a value spelling --yes is not consent.
            # Unknown options fail closed until their grammar is supported here.
            case $argument in
                --home|--config|--host|--dispatcher|--codex-skills-dir|--claude-skills-dir|--format) needs_value=yes;;
                --yes|--show|--dry-run) safe_input=yes;;
                --global|--uninstall|--offer|--json|--help|-h) ;;
                *) fail "verified binary remains installed at $destination; setup incomplete: unrecognized unattended init argument";;
            esac
        done
        [ "$needs_value" = no ] || fail "verified binary remains installed at $destination; setup incomplete: init option needs a value"
        [ "$safe_input" = yes ] || fail "verified binary remains installed at $destination; setup incomplete: unattended setup requires --yes, --show or --dry-run"
    fi
    "$destination" init "$@" || fail "verified binary remains installed at $destination; setup incomplete. Retry $destination init with the same choices."
fi
