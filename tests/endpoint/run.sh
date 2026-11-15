#!/bin/sh
# End-to-end test of the furnish endpoint (packaging/endpoint) with a fake IdP and a fake GitLab.
#
#   tests/endpoint/run.sh                       # the proxy alone
#   FURNISH_BIN=<static furnish> tests/endpoint/run.sh   # and furnish signing in and fetching through it
#
# Runs packaging/endpoint/docker-compose.yml plus tests/endpoint/compose.yml under a fresh compose
# project, runs checks.sh in a client container, checks the fake GitLab's access log, and removes
# everything on exit. Prints one line per check; exits non-zero on any failure.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
project=furnish-endpoint-test-$(date +%s)-$$

rand() { od -An -N16 -tx1 /dev/urandom | tr -d ' \n'; }
export TEST_DIR="$here"
export FURNISH_HOST=furnish.test FURNISH_TLS=internal GITLAB_URL=http://gitlab
export OIDC_ISSUER_URL=http://mock-oidc:8080/acme OIDC_CLIENT_ID=furnish ALLOWED_EMAIL_DOMAINS=myorg.tld
OAUTH2_PROXY_COOKIE_SECRET=$(rand)                  # 32 hex characters = 32 bytes
DEPLOY_USER=gitlab+deploy-token-1 DEPLOY_TOKEN=gldt-$(rand)
export OAUTH2_PROXY_COOKIE_SECRET DEPLOY_USER DEPLOY_TOKEN
ACME_GITLAB_AUTH=$(printf '%s:%s' "$DEPLOY_USER" "$DEPLOY_TOKEN" | base64 | tr -d '\n'); export ACME_GITLAB_AUTH

dc() { docker compose -p "$project" -f "$root/packaging/endpoint/docker-compose.yml" -f "$here/compose.yml" "$@"; }
trap 'dc --profile client down -v -t 2 --remove-orphans >/dev/null 2>&1 || true' EXIT
trap 'exit 130' INT TERM

dc --profile client build -q
dc up -d --quiet-pull caddy oauth2-proxy mock-oidc gitlab 2>/dev/null

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

# Misconfigured deploy token: GitLab answers 401, the client must get 502 and no Basic challenge.
ACME_GITLAB_AUTH=$(printf 'gitlab+deploy-token-1:wrong' | base64 | tr -d '\n')
dc up -d caddy 2>/dev/null
dc run --rm -T -e READY_STATUS=502 client sh /checks.sh upstream-rejects || failed=1

if [ $failed -ne 0 ]; then
    echo "--- container logs ---"
    dc logs --no-color
    echo "FAILED"
else
    echo "PASSED"
fi
exit $failed
