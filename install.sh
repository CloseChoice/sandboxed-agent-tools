#!/usr/bin/env bash
# Install SAT, optionally followed by default-tools or a comma-separated tool list.
set -euo pipefail

repo="https://github.com/CloseChoice/sandboxed-agent-tools"
version="latest"
platform=""
bin_dir="${HOME}/.local/bin"
tools=()

fail() { printf 'sat install: %s\n' "$*" >&2; exit 1; }
usage() {
    echo "Usage: install.sh [default-tools|python,jaq,...] [--version VERSION] [--platform PLATFORM] [--bin-dir DIR]"
    echo "Platforms: linux-x64, macos-x64, macos-arm64 (default: detect this machine)"
}
while [[ $# -gt 0 ]]; do
    case "$1" in
        --version|--platform|--bin-dir)
            [[ $# -ge 2 && -n "$2" ]] || fail "$1 requires a value"
            case "$1" in
                --version) version="$2" ;;
                --platform) platform="$2" ;;
                --bin-dir) bin_dir="$2" ;;
            esac
            shift 2 ;;
        -h|--help) usage; exit 0 ;;
        --*) fail "unknown option: $1" ;;
        default-tools) tools+=(python jaq code markitdown); shift ;;
        *)
            IFS=',' read -r -a selected <<< "$1"
            tools+=("${selected[@]}")
            shift ;;
    esac
done
for tool in "${tools[@]-}"; do
    [[ -n "$tool" ]] || continue
    case "$tool" in python|jaq|code|markitdown) ;; *) fail "unknown tool: $tool" ;; esac
done

if [[ -z "$platform" ]]; then
    case "$(uname -s):$(uname -m)" in
        Linux:x86_64) platform=linux-x64 ;;
        Darwin:x86_64) platform=macos-x64 ;;
        Darwin:arm64) platform=macos-arm64 ;;
        *) fail "unsupported machine; use install.ps1 for Windows" ;;
    esac
fi
case "$platform" in
    linux-x64) target=x86_64-unknown-linux-gnu ;;
    macos-x64) target=x86_64-apple-darwin ;;
    macos-arm64) target=aarch64-apple-darwin ;;
    *) fail "unsupported platform: $platform" ;;
esac

if [[ "$version" == latest ]]; then
    release_url=$(curl -fsSL -o /dev/null -w '%{url_effective}' "$repo/releases/latest") ||
        fail "no latest release found; publish a release first or specify --version"
    version="${release_url##*/}"
fi
version="${version#v}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]] ||
    fail "invalid version: $version"
base="$repo/releases/download/v$version"
asset="sat-$version-$target.tar.gz"
work_dir=$(mktemp -d)
trap 'rm -rf "$work_dir"' EXIT

curl -fsSL "$base/SHA256SUMS" -o "$work_dir/SHA256SUMS"
download() {
    local name="$1" hash filename expected="" actual
    curl -fsSL "$base/$name" -o "$work_dir/$name"
    while read -r hash filename; do
        if [[ "$filename" == "$name" ]]; then expected="$hash"; break; fi
    done < "$work_dir/SHA256SUMS"
    [[ "$expected" =~ ^[0-9a-fA-F]{64}$ ]] || fail "missing checksum for $name"
    if command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "$work_dir/$name")
    else
        actual=$(shasum -a 256 "$work_dir/$name")
    fi
    [[ "${actual%% *}" == "$expected" ]] || fail "checksum mismatch for $name"
}
# Verify every selected download before replacing or executing the host.
download "$asset"
for tool in "${tools[@]-}"; do
    [[ -n "$tool" ]] || continue
    download "$tool.tar"
done
tar -xzf "$work_dir/$asset" -C "$work_dir" sat
chmod 755 "$work_dir/sat"
mkdir -p "$bin_dir"
bin_dir=$(cd "$bin_dir" && pwd)
mv -f "$work_dir/sat" "$bin_dir/sat"

for tool in "${tools[@]-}"; do
    [[ -n "$tool" ]] || continue
    # Keep registration out of the caller's project and grant no default authority.
    (cd "$work_dir"
     printf '[]\n[]\n[]\n[]\n[]\nunset\n' |
         "$bin_dir/sat" register --force "$work_dir/$tool.tar" >/dev/null)
    echo "Installed tool: $tool"
done
echo "Installed SAT $version to $bin_dir/sat"
printf 'Add to PATH if needed: export PATH=%q:"$PATH"\n' "$bin_dir"
