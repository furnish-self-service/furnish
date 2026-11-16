#!/bin/sh
# End-to-end test of the furnish endpoint (packaging/endpoint) with a fake IdP and a fake GitLab.
#
#   PROXY=nginx tests/endpoint/run.sh            # the proxy alone (caddy, nginx, traefik, envoy, haproxy)
#   FURNISH_BIN=<static furnish> tests/endpoint/run.sh   # and furnish signing in and fetching through it
#   RECEIVER=vector tests/endpoint/run.sh        # Vector instead of the OpenTelemetry Collector
#
# Runs packaging/endpoint/docker-compose.yml with packaging/endpoint/$PROXY/compose.yml (and
# vector/compose.yml with RECEIVER=vector), the files the docs show, plus tests/endpoint/compose.yml
# under a fresh compose project. Runs checks.sh in a client container, checks the fake GitLab's
# access log and what the receiver got, and removes everything on exit. Prints one line per check; exits non-zero on any failure.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
PROXY=${PROXY:-caddy}; export PROXY
[ -f "$root/packaging/endpoint/$PROXY/compose.yml" ] || { echo "no proxy '$PROXY' in packaging/endpoint" >&2; exit 2; }
RECEIVER=${RECEIVER:-collector}
# The Collector writes what it receives to OUT_DIR (collector.yml); Vector prints one line per record.
OUT_DIR=$(mktemp -d); export OUT_DIR; chmod 777 "$OUT_DIR"
case $RECEIVER in
    collector) receiver_file= test_file=$here/collector.yml ;;
    vector) receiver_file=$root/packaging/endpoint/vector/compose.yml test_file= ;;
    *) echo "RECEIVER is collector or vector" >&2; exit 2 ;;
esac
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
# The containers' root has no capability to bypass file permissions (cap_drop: ALL), so like
# an admin's root-owned files, these must be readable by their user.
chmod 644 "$TLS_DIR"/* && chmod 755 "$TLS_DIR"
export OIDC_ISSUER_URL=http://mock-oidc:8080/acme OIDC_CLIENT_ID=furnish ALLOWED_EMAIL_DOMAINS=myorg.tld
SECRETS_DIR=$TLS_DIR/secrets; export SECRETS_DIR
mkdir -p "$SECRETS_DIR" && rand > "$SECRETS_DIR/oauth2_proxy_cookie_secret"   # 32 hex characters = 32 bytes
chmod 644 "$SECRETS_DIR/oauth2_proxy_cookie_secret"
DEPLOY_USER=gitlab+deploy-token-1 DEPLOY_TOKEN=gldt-$(rand)
export DEPLOY_USER DEPLOY_TOKEN
ACME_GITLAB_AUTH=$(printf '%s:%s' "$DEPLOY_USER" "$DEPLOY_TOKEN" | base64 | tr -d '\n'); export ACME_GITLAB_AUTH

dc() { docker compose -p "$project" --project-directory "$root/packaging/endpoint" -f "$root/packaging/endpoint/docker-compose.yml" \
    -f "$root/packaging/endpoint/$PROXY/compose.yml" ${receiver_file:+-f "$receiver_file"} -f "$here/compose.yml" ${test_file:+-f "$test_file"} "$@"; }
trap 'dc --profile client down -v -t 2 --remove-orphans >/dev/null 2>&1 || true; rm -rf "$TLS_DIR" "$OUT_DIR"' EXIT
trap 'exit 130' INT TERM

dc --profile client build -q
# Optional per-proxy preparation (e.g. HAProxy needs the IdP's public key), sourced here.
[ -f "$here/$PROXY.sh" ] && . "$here/$PROXY.sh"
dc up -d --quiet-pull proxy oauth2-proxy receiver mock-oidc gitlab 2>$TLS_DIR/up.log || { cat $TLS_DIR/up.log >&2; exit 1; }
echo "proxy: $PROXY, receiver: $RECEIVER"

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
# What the proxy tells GitLab about the client: from the connection, never what the client sent.
spoofed=$(echo "$log" | grep -E '6\.6\.6\.6|evil\.example|spoofed|6666' || true)
[ -z "$spoofed" ] && ok "spoofed X-Forwarded-*, X-Real-IP and Forwarded never reached upstream" || bad "upstream got spoofed client headers: $spoofed"
unset_fwd=$(echo "$log" | grep -vE ' xff="[0-9.]+" xri="[0-9.]+" xfh="furnish\.test" xfp="https" ' || true)
[ -z "$unset_fwd" ] && ok "upstream got X-Forwarded-For, X-Real-IP, -Host and -Proto from the connection" || bad "upstream without the proxy's forwarding headers: $(echo "$unset_fwd" | head -3)"

# What the receiver got (checks.sh sent reports, with and without attempts to pose as someone else):
# one JSON object per log record with the attributes of its resource and its own. The Collector
# prints OTLP requests, Vector one event per record.
received() { if [ -n "$test_file" ]; then cat "$OUT_DIR/reports.ndjson"; else dc logs --no-color --no-log-prefix receiver | grep '^{'; fi; }
# furnish sent its report just before; the Collector batches and writes it within a second or two.
i=0; while [ -n "${FURNISH_BIN:-}" ] && ! received 2>/dev/null | grep -q collectionUpdate && [ $i -lt 15 ]; do i=$((i + 1)); sleep 1; done
reports=$(received | jq -c '
    def flat: map({(.key): (.value.stringValue // .value)}) | add // {};
    if .resourceLogs then .resourceLogs[] | (.resource.attributes // [] | flat) as $r
        | .scopeLogs[] | (.scope.attributes // [] | flat) as $s | .logRecords[] | $r + $s + (.attributes // [] | flat)
    else (.resources // {}) + (.attributes // {}) end' 2>/dev/null | grep -v '"check":"size"' || true)
check() { echo "$reports" | jq -e -s "$1" >/dev/null; }
check 'any(.check == "events")' && ok "the receiver got the reports" || bad "the receiver got no reports: $reports"
check 'map(select(.check == "events" or .check == "forged")) | length >= 3 and all(."user.email" == "alice@myorg.tld")' \
    && ok "every report carries the signed-in user" || bad "a report without the signed-in user: $reports"
echo "$reports" | grep -q mallory && bad "a client named another user in a report: $(echo "$reports" | grep mallory)" || ok "clients can't name another user"
check 'map(select(.check == "forged")) | length == 1 and all(keys - ["check", "furnish.event", "service.name", "user.email"] == [])' \
    && ok "user attributes the client sent (user.*, furnish.user) are dropped" || bad "client user attributes kept: $(echo "$reports" | grep forged)"
check 'all(."service.name" == "furnish")' && ok "the receiver sets service.name" || bad "records without service.name furnish"
check 'any(.check == "unauthenticated")' && bad "a report without sign-in reached the receiver" || ok "reports without sign-in never reach the receiver"
if [ -n "${FURNISH_BIN:-}" ]; then
    check 'any(."furnish.event" == "collectionUpdate" and ."user.email" == "alice@myorg.tld" and ."device.id" != null and ."service.version" != null)' \
        && ok "furnish's own report arrived as OTLP with the signed-in user" || bad "no collectionUpdate report from furnish: $reports"
fi

# Misconfigured deploy token: GitLab answers 401, the client must get 502 and no Basic challenge.
# The token check now also requires a group (502 = a token passed it).
ACME_GITLAB_AUTH=$(printf 'gitlab+deploy-token-1:wrong' | base64 | tr -d '\n')
export ALLOWED_GROUPS=furnish-users
dc up -d proxy oauth2-proxy 2>/dev/null
dc run --rm -T -e READY_STATUS=502 client sh /checks.sh upstream-rejects || failed=1

if [ $failed -ne 0 ]; then
    echo "--- container logs ---"
    dc logs --no-color
    echo "FAILED"
else
    echo "PASSED"
fi
exit $failed
