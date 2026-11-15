#!/bin/sh
# End-to-end test of the furnish endpoint (packaging/endpoint) with a fake IdP and a fake GitLab.
#
#   PROXY=nginx tests/endpoint/run.sh            # the proxy alone (caddy, nginx, traefik, envoy, haproxy)
#   FURNISH_BIN=<static furnish> tests/endpoint/run.sh   # and furnish signing in and fetching through it
#
# Runs packaging/endpoint/docker-compose.yml with packaging/endpoint/$PROXY/compose.yml, the files
# the docs show, plus tests/endpoint/compose.yml under a fresh compose project. Runs checks.sh in a
# client container, checks the fake GitLab's access log and what Vector received, and removes
# everything on exit. Prints one line per check; exits non-zero on any failure.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
PROXY=${PROXY:-caddy}
[ -f "$root/packaging/endpoint/$PROXY/compose.yml" ] || { echo "no proxy '$PROXY' in packaging/endpoint" >&2; exit 2; }
project=furnish-endpoint-test-$PROXY-$(date +%s)-$$

rand() { od -An -N16 -tx1 /dev/urandom | tr -d ' \n'; }
export TEST_DIR="$here" FURNISH_JSON="$here/furnish.json"
export FURNISH_HOST=furnish.test GITLAB_URL=http://gitlab
# A test CA and a certificate for furnish.test, like an organization's own; the client trusts the CA.
TLS_DIR=$(mktemp -d); export TLS_DIR FURNISH_TLS="/certs/tls.crt /certs/tls.key"
openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj /CN=furnish-test-ca -keyout "$TLS_DIR/ca.key" -out "$TLS_DIR/ca.crt" 2>/dev/null
openssl req -newkey rsa:2048 -nodes -subj /CN=furnish.test -keyout "$TLS_DIR/tls.key" -out "$TLS_DIR/tls.csr" 2>/dev/null
printf 'subjectAltName=DNS:furnish.test\nextendedKeyUsage=serverAuth\n' > "$TLS_DIR/ext"
openssl x509 -req -in "$TLS_DIR/tls.csr" -CA "$TLS_DIR/ca.crt" -CAkey "$TLS_DIR/ca.key" -CAcreateserial -days 2 -extfile "$TLS_DIR/ext" -out "$TLS_DIR/tls.crt" 2>/dev/null
chmod 644 "$TLS_DIR"/*
export OIDC_ISSUER_URL=http://mock-oidc:8080/acme OIDC_CLIENT_ID=furnish ALLOWED_EMAIL_DOMAINS=myorg.tld
OAUTH2_PROXY_COOKIE_SECRET=$(rand)                  # 32 hex characters = 32 bytes
DEPLOY_USER=gitlab+deploy-token-1 DEPLOY_TOKEN=gldt-$(rand)
export OAUTH2_PROXY_COOKIE_SECRET DEPLOY_USER DEPLOY_TOKEN
ACME_GITLAB_AUTH=$(printf '%s:%s' "$DEPLOY_USER" "$DEPLOY_TOKEN" | base64 | tr -d '\n'); export ACME_GITLAB_AUTH

dc() { docker compose -p "$project" --project-directory "$root/packaging/endpoint" -f "$root/packaging/endpoint/docker-compose.yml" \
    -f "$root/packaging/endpoint/$PROXY/compose.yml" -f "$here/compose.yml" "$@"; }
trap 'dc --profile client down -v -t 2 --remove-orphans >/dev/null 2>&1 || true; rm -rf "$TLS_DIR"' EXIT
trap 'exit 130' INT TERM

dc --profile client build -q
# Optional per-proxy preparation (e.g. HAProxy needs the IdP's public key), sourced here.
[ -f "$here/$PROXY.sh" ] && . "$here/$PROXY.sh"
dc up -d --quiet-pull proxy oauth2-proxy vector mock-oidc gitlab 2>/dev/null
echo "proxy: $PROXY"

failed=0
dc run --rm -T client sh /checks.sh || failed=1
# furnish itself against the endpoint, with FURNISH_BIN (a static Linux binary).
if [ -n "${FURNISH_BIN:-}" ]; then dc run --rm -T client sh /furnish.sh || failed=1; fi

# What the fake GitLab saw: only the two fixed URIs, always with Basic auth, never the secret repo.
log=$(dc logs --no-color --no-log-prefix gitlab | grep '^upstream ' || true)
# ok/bad always succeed, so `cond && ok || bad` is a safe if/else here.
ok()  { echo "ok   $*"; }
bad() { echo "FAIL $*"; failed=1; }
[ -n "$log" ] && ok "upstream received requests ($(echo "$log" | wc -l))" || bad "upstream received no requests"
other=$(echo "$log" | grep -vE '^upstream (GET /it/catalog\.git/info/refs\?service=git-upload-pack|POST /it/catalog\.git/git-upload-pack) ' || true)
[ -z "$other" ] && ok "upstream saw only the two allowlisted URIs" || bad "upstream saw other requests: $other"
echo "$log" | grep -q secret && bad "upstream saw the secret repository" || ok "upstream never saw the secret repository"
echo "$log" | grep -vq ' auth=Basic ' && bad "upstream got a request without the deploy token (or with the client's bearer)" \
    || ok "upstream got the deploy token (Basic) on every request, never the client's bearer"
echo "$log" | grep -q 'proto="version=2"' && ok "Git-Protocol: version=2 header reached upstream" || bad "no protocol v2 request upstream"
echo "$log" | grep -q ' 401 ' && bad "upstream rejected a request" || ok "upstream accepted the deploy token"

# What Vector received (checks.sh sent reports, with and without attempts to pose as someone else).
reports=$(dc logs --no-color --no-log-prefix vector | grep '"event"' || true)
echo "$reports" | grep -q '"check":"events"' && ok "Vector received the reports" || bad "Vector received no reports"
echo "$reports" | grep '"check":"events"' | grep -vq '"user":"alice@myorg.tld"' \
    && bad "a report reached Vector without the signed-in user: $(echo "$reports" | grep -v alice | head -1)" \
    || ok "every report carries the signed-in user"
echo "$reports" | grep -q mallory && bad "a client named another user in a report" || ok "clients can't name another user"
echo "$reports" | grep -q '"check":"unauthenticated"' && bad "a report without sign-in reached Vector" || ok "reports without sign-in never reach Vector"
if [ -n "${FURNISH_BIN:-}" ]; then
    echo "$reports" | grep '"event":"collectionUpdate"' | grep -q '"user":"alice@myorg.tld"' \
        && ok "furnish's own report arrived with the signed-in user" || bad "no collectionUpdate report from furnish: $reports"
fi

# Misconfigured deploy token: GitLab answers 401, the client must get 502 and no Basic challenge.
ACME_GITLAB_AUTH=$(printf 'gitlab+deploy-token-1:wrong' | base64 | tr -d '\n')
dc up -d proxy 2>/dev/null
dc run --rm -T -e READY_STATUS=502 client sh /checks.sh upstream-rejects || failed=1

if [ $failed -ne 0 ]; then
    echo "--- container logs ---"
    dc logs --no-color
    echo "FAILED"
else
    echo "PASSED"
fi
exit $failed
