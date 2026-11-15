#!/bin/sh
# Prints an OIDC issuer's RSA signing key as a PEM public key, for jwt_verify in haproxy.cfg.
# HAProxy cannot read a JWKS; it verifies with one key file that it loads at startup.
#
#   haproxy/jwks-to-pem.sh <issuer URL> [<key ID>] > haproxy/idp.pem
#
# Without a key ID, the JWKS must hold exactly one RSA signing key; otherwise the key IDs are
# listed and you pick the one the IdP currently signs with (the "kid" in a fresh ID token's header).
# Needs curl, openssl, sed, tr and od.
set -eu
issuer=${1:?usage: jwks-to-pem.sh <issuer URL> [<key ID>]}
kid=${2:-}

flat() { tr -d ' \t\r\n'; }
config=$(curl -fsS "${issuer%/}/.well-known/openid-configuration" | flat)
jwks_uri=$(printf '%s' "$config" | sed -n 's/.*"jwks_uri":"\([^"]*\)".*/\1/p')
[ -n "$jwks_uri" ] || { echo "no jwks_uri in $issuer/.well-known/openid-configuration" >&2; exit 1; }

# One JWK per line; keep RSA keys that may sign (no "use", or "use":"sig").
keys=$(curl -fsS "$jwks_uri" | flat | sed 's/},{/}\n{/g' | grep '"kty":"RSA"' | grep -v '"use":"enc"' || true)
if [ -n "$kid" ]; then keys=$(printf '%s\n' "$keys" | grep -F "\"kid\":\"$kid\"" || true); fi
field() { printf '%s' "$key" | sed -n "s/.*\"$1\":\"\([^\"]*\)\".*/\1/p"; }
count=$(printf '%s' "$keys" | grep -c . || true)
if [ "$count" -ne 1 ]; then
    echo "found $count matching RSA signing keys in $jwks_uri; pass one key ID:" >&2
    printf '%s\n' "$keys" | while read -r key; do [ -n "$key" ] && echo "  $(field kid)" >&2; done
    exit 1
fi
key=$keys

# JWK "n" and "e" are base64url big-endian integers; build an RSAPublicKey from them.
hex() {
    s=$(printf '%s' "$1" | tr '_-' '/+')
    case $((${#s} % 4)) in 2) s="$s==" ;; 3) s="$s=" ;; esac
    printf '%s' "$s" | openssl base64 -d -A | od -An -v -tx1 | tr -d ' \n'
}
n=$(field n) e=$(field e)
[ -n "$n" ] && [ -n "$e" ] || { echo "key has no n/e" >&2; exit 1; }
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
printf 'asn1=SEQUENCE:k\n[k]\nn=INTEGER:0x%s\ne=INTEGER:0x%s\n' "$(hex "$n")" "$(hex "$e")" > "$tmp/key.cnf"
openssl asn1parse -genconf "$tmp/key.cnf" -noout -out "$tmp/key.der"
openssl rsa -RSAPublicKey_in -inform DER -in "$tmp/key.der" -pubout 2>/dev/null
echo "exported key $(field kid) from $jwks_uri" >&2
