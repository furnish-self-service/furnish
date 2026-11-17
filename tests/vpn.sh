#!/usr/bin/env bash
# The example collection's VPN apps on a real system (CI): install acme-vpn, which pulls in
# openvpn-client, check the configuration where each OS's client looks for it, uninstall acme-vpn
# and check the configuration is gone and the client stays. On macOS and Linux also a newer
# profile: the app shows an update, installing imports it again, and uninstalling works while an
# update is waiting.
#
#   tests/vpn.sh path/to/furnish <commit of furnish-self-service/self-service-example>
#
# DSC's resources must be beside furnish (scripts/fetch-dsc.mjs). Changes the system: needs an
# administrator (Windows), passwordless sudo (macOS, Linux) and, on Linux, apt and NetworkManager.
# macOS: installs the furnish service, creates the user carol and needs Homebrew.
set -euo pipefail
furnish=$(realpath "$1")
ref=${2:?usage: $0 <furnish> <example commit>}
tmp=$(mktemp -d)
if command -v cygpath >/dev/null; then tmp=$(cygpath -m "$tmp"); fi
export FURNISH_DATA_DIR="$tmp/data"

# Linux: acme-vpn writes to /etc and NetworkManager, so furnish runs as root (as the service
# would). Windows: the runner is an elevated administrator. macOS: through the furnish service,
# as a user without administrator rights; the service runs Homebrew (openvpn-client) as the
# account that owns it, the runner.
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
case "$(uname -s)" in Linux | Darwin) sudo chown -R root "$tmp/example" ;; esac

if [ "$(uname -s)" = Darwin ]; then
    # The service runs only a binary (and DSC's resources beside it) that root owns, in folders
    # only root can change.
    sudo mkdir -p /Library/furnish-vpn
    sudo cp -R "$furnish" "$(dirname "$furnish")/dsc" /Library/furnish-vpn/
    sudo chown -R root:wheel /Library/furnish-vpn && sudo chmod -R go-w /Library/furnish-vpn
    furnish=/Library/furnish-vpn/furnish
    sudo sysadminctl -addUser carol -password Furnish-Test-1 >/dev/null 2>&1
    sudo dseditgroup -o create furnish-users
    sudo dseditgroup -o edit -a carol -t user furnish-users
    if dseditgroup -o checkmember -m carol admin >/dev/null 2>&1; then fail "carol is an administrator"; fi
    # The example as a preset collection, as an MDM profile would deploy the policy.
    printf '{"collections": {"preset": [{"name": "example", "url": "%s"}], "usersCanAdd": false}, "parameters": {"example": {"role": {"value": "office"}}}, "service": {"allowedGroup": "furnish-users"}}' \
        "$tmp/example" > "$tmp/policy.json"
    sudo mkdir -p "/Library/Managed Preferences"
    plutil -convert xml1 -o - "$tmp/policy.json" | sudo tee "/Library/Managed Preferences/io.github.furnish-self-service.furnish.plist" >/dev/null
    sudo "$furnish" service install
    for _ in $(seq 100); do [ -S /var/run/furnish.sock ] && [ -d "/Library/Application Support/furnish/collections/example/apps" ] && break; sleep 0.3; done
    [ -S /var/run/furnish.sock ] || { sudo cat /var/log/furnish.log; fail "the service didn't start"; }
    run=(sudo -u carol env FURNISH_DATA_DIR=/tmp/carol-data HOME=/tmp PATH="$PATH")
    trap '[ $? = 0 ] || { echo "--- service log"; sudo cat /var/log/furnish.log; }' EXIT
else
    step collection add "$tmp/example" --name example >/dev/null
    step option set example role office >/dev/null
fi
expect "$(step list)" "·  acme-vpn "

expect "$(step -v install acme-vpn)" "Installed acme-vpn"
list=$(step list)
if ! grep -q "✓  acme-vpn " <<<"$list"; then
    # What the user sees of the configuration, and the steps furnish finds not set up.
    step install --dry-run acme-vpn || true
    if [ "$(uname -s)" = Darwin ]; then
        tb="/Library/Application Support/Tunnelblick"
        sudo ls -leR "$tb" || true
        for p in "$tb" "$tb/Temp" "$tb/Temp/Acme VPN.tblk/Contents/Resources/config.ovpn" "$tb/Shared" \
            "$tb/Shared/Acme VPN.tblk" "$tb/Shared/Acme VPN.tblk/Contents/Resources/config.ovpn"; do
            ${run[@]+"${run[@]}"} ls -ld "$p" 2>&1 || true
        done
    fi
fi
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
    # The copy Tunnelblick installed from, which furnish compares with the one in Shared/.
    profile="/Library/Application Support/Tunnelblick/Temp/Acme VPN.tblk/Contents/Resources/config.ovpn"
    ls -l "$profile"
    ;;
Linux)
    ovpn=/etc/openvpn/acme-vpn.ovpn
    client=/usr/lib/NetworkManager/VPN/nm-openvpn-service.name
    [ -f "$client" ] || fail "no NetworkManager OpenVPN plugin ($client)"
    [ -f "$ovpn" ] || fail "no $ovpn"
    sudo nmcli connection show 'Acme VPN' | grep -E '^(connection\.(id|type)|vpn\.(service-type|data)):' \
        || fail "no NetworkManager connection 'Acme VPN'"
    dpkg -s network-manager-openvpn | grep '^Status:'
    # nmcli copied the CA certificate into root's home; uninstalling must delete it.
    ca=$(sudo nmcli -g vpn.data connection show id 'Acme VPN' | tr ',' '\n' | sed -n 's/^ *ca = //p')
    [ -n "$ca" ] && sudo test -f "$ca" || fail "no CA certificate in the connection's vpn.data ($ca)"
    echo "CA certificate: $ca"
    # Only root may read the profile; the stamp has its date and tells which version was imported.
    profile=$ovpn
    stamp=/etc/openvpn/acme-vpn.imported
    ls -l "$ovpn" "$stamp"
    [ "$(stat -c %a "$ovpn")" = 600 ] || fail "$ovpn isn't mode 600"
    [ "$(stat -c %Y "$ovpn")" = "$(stat -c %Y "$stamp")" ] || fail "$stamp hasn't the profile's date"
    ;;
esac
grep -q "remote vpn.acme.example 1194" "$ovpn" 2>/dev/null || sudo grep -q "remote vpn.acme.example 1194" "$ovpn" \
    || fail "$ovpn has other content"

# A newer profile, as when the collection brings a changed one: the configuration is there but
# outdated. Installing imports it again; uninstalling works while the update waits.
newer() { sleep 1; sudo touch "$profile"; expect "$(step list)" "↑  acme-vpn "; }
case "$(uname -s)" in
Darwin | Linux)
    newer
    expect "$(step -v install acme-vpn)" "Installed acme-vpn"
    expect "$(step list)" "✓  acme-vpn "
    [ "$(uname -s)" = Darwin ] || [ "$(stat -c %Y "$ovpn")" = "$(stat -c %Y "$stamp")" ] || fail "$stamp wasn't renewed"
    newer
    ;;
esac

expect "$(step -v uninstall acme-vpn)" "Uninstalled acme-vpn"
list=$(step list)
expect "$list" "·  acme-vpn "
expect "$list" "✓  openvpn-client "
case "$(uname -s)" in
Darwin) sudo test ! -e "${ovpn%/Contents/*}" || fail "the configuration is still there"
    [ ! -e "${profile%/Contents/*}" ] || fail "${profile%/Contents/*} is still there" ;;
Linux) [ ! -e "$ovpn" ] || fail "$ovpn is still there"
    [ ! -e "$stamp" ] || fail "$stamp is still there" ;;
*) [ ! -e "$ovpn" ] || fail "$ovpn is still there" ;;
esac
[ -e "$client" ] || fail "uninstalling acme-vpn removed $client"
if [ "$(uname -s)" = Linux ]; then
    if sudo nmcli connection show 'Acme VPN' >/dev/null 2>&1; then fail "the NetworkManager connection is still there"; fi
    if sudo test -e "$ca"; then fail "the CA certificate $ca is still there"; fi
    dpkg -s network-manager-openvpn | grep -q '^Status: install ok installed' || fail "the client was removed"
fi
echo "OK: acme-vpn installed and uninstalled, openvpn-client kept"
