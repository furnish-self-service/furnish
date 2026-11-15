#!/usr/bin/env bash
# The example collection's VPN apps on a real system (CI): install acme-vpn, which pulls in
# openvpn-client, check the configuration where each OS's client looks for it, uninstall acme-vpn
# and check the configuration is gone and the client stays.
#
#   tests/vpn.sh path/to/furnish <commit of furnish-self-service/self-service-example>
#
# DSC's resources must be beside furnish (scripts/fetch-dsc.mjs). Changes the system: needs an
# administrator (Windows), passwordless sudo (macOS, Linux) and, on Linux, apt and NetworkManager.
set -euo pipefail
furnish=$(realpath "$1")
ref=${2:?usage: $0 <furnish> <example commit>}
tmp=$(mktemp -d)
if command -v cygpath >/dev/null; then tmp=$(cygpath -m "$tmp"); fi
export FURNISH_DATA_DIR="$tmp/data"

# Linux: acme-vpn writes to /etc and NetworkManager, so furnish runs as root (as the service
# would). Windows: the runner is an elevated administrator. macOS: furnish runs as the user,
# because Homebrew (openvpn-client) refuses root; acme-vpn's steps get root through sudo.
run=()  # bash 3.2 (macOS): an empty array is "unbound", hence ${run[@]+...} below
[ "$(uname -s)" = Linux ] && run=(sudo --preserve-env=FURNISH_DATA_DIR)

step() {
    echo "+ furnish $*" >&2
    local out rc=0
    out=$(${run[@]+"${run[@]}"} "$furnish" "$@" 2>&1) || rc=$?
    echo "$out" >&2
    echo "$out"
    return $rc
}
fail() { echo "FAIL: $*" >&2; exit 1; }
expect() { grep -q -- "$2" <<<"$1" || fail "expected '$2'"; }

# The collection at a fixed commit, so a change to the example can't break furnish's CI unnoticed.
git init -q "$tmp/example"
git -C "$tmp/example" fetch -q --depth 1 https://github.com/furnish-self-service/self-service-example "$ref"
git -C "$tmp/example" checkout -q -B main FETCH_HEAD
# root clones only repositories it owns.
[ "$(uname -s)" = Linux ] && sudo chown -R root "$tmp/example"

step collection add "$tmp/example" --name example >/dev/null
step option set example role office >/dev/null
expect "$(step list)" "·  acme-vpn "

expect "$(step -v install acme-vpn)" "Installed acme-vpn"
list=$(step list)
expect "$list" "✓  acme-vpn "
expect "$list" "✓  openvpn-client "

case "$(uname -s)" in
MINGW* | MSYS* | CYGWIN*)
    ovpn="C:/Program Files/OpenVPN/config/acme.ovpn"
    client="C:/Program Files/OpenVPN/bin/openvpn-gui.exe"
    [ -f "$ovpn" ] || fail "no $ovpn"
    [ -f "$client" ] || fail "no OpenVPN GUI at $client"
    ls -l "$ovpn" "$client"
    ;;
Darwin)
    ovpn="/Library/Application Support/Tunnelblick/Shared/Acme VPN.tblk/Contents/Resources/config.ovpn"
    client=/Applications/Tunnelblick.app
    sudo test -f "$ovpn" || fail "no $ovpn"
    [ -d "$client" ] || fail "no $client"
    sudo ls -lR "/Library/Application Support/Tunnelblick/Shared"
    ;;
Linux)
    ovpn=/etc/openvpn/acme-vpn.ovpn
    [ -f "$ovpn" ] || fail "no $ovpn"
    sudo nmcli connection show 'Acme VPN' | grep -E '^(connection\.(id|type)|vpn\.(service-type|data)):' \
        || fail "no NetworkManager connection 'Acme VPN'"
    dpkg -s network-manager-openvpn | grep '^Status:'
    ;;
esac
grep -q "remote vpn.acme.example 1194" "$ovpn" 2>/dev/null || sudo grep -q "remote vpn.acme.example 1194" "$ovpn" \
    || fail "$ovpn has other content"

expect "$(step -v uninstall acme-vpn)" "Uninstalled acme-vpn"
list=$(step list)
expect "$list" "·  acme-vpn "
expect "$list" "✓  openvpn-client "
case "$(uname -s)" in
Darwin) sudo test ! -e "${ovpn%/Contents/*}" || fail "the configuration is still there" ;;
*) [ ! -e "$ovpn" ] || fail "$ovpn is still there" ;;
esac
[ -e "$client" ] || [ "$(uname -s)" = Linux ] || fail "uninstalling acme-vpn removed $client"
if [ "$(uname -s)" = Linux ]; then
    if sudo nmcli connection show 'Acme VPN' >/dev/null 2>&1; then fail "the NetworkManager connection is still there"; fi
    dpkg -s network-manager-openvpn | grep -q '^Status: install ok installed' || fail "the client was removed"
fi
echo "OK: acme-vpn installed and uninstalled, openvpn-client kept"
