#!/bin/sh
# Runs inside the test client container (see run.sh). Prints one line per check.
set -u
E=https://furnish.test
ISS=http://mock-oidc:8080
CA=/certs/ca.crt
OUT=/tmp/out                    # every response (headers + body) lands here for the leak check
REDIRECT=http://127.0.0.1:47123/callback
mkdir -p $OUT /tmp/w
export HOME=/tmp GIT_TERMINAL_PROMPT=0 GIT_SSL_CAINFO=$CA
git config --global user.name test && git config --global user.email test@test
failed=0 n=0

ok()  { echo "ok   $*"; }
bad() { echo "FAIL $*"; failed=1; }

# req <name> <expected status list> <curl args...>: one HTTP request, response saved to $OUT.
req() {
    name=$1 want=$2; shift 2; n=$((n + 1))
    code=$(curl -sS --cacert $CA -m 15 -o $OUT/$n.body -D $OUT/$n.head -w '%{http_code}' "$@" 2>>$OUT/$n.err)
    if grep -qE "TOP-SECRET-MARKER|FAKE-GITLAB-REACHED|content of secret" $OUT/$n.body 2>/dev/null; then
        bad "$name (got $code, upstream content in body)"
    elif echo " $want " | grep -q " $code "; then ok "$name ($code)"
    else bad "$name (got $code, want $want)"; fi
}

b64url() { openssl base64 -A | tr '+/' '-_' | tr -d '='; }

# token <issuer name> <client id> <extra claims JSON> <field>: authorization code flow with PKCE,
# as furnish does it, against the mock IdP (its login form is posted directly); prints the token
# response field (id_token or access_token).
token() {
    verifier=$(head -c 32 /dev/urandom | b64url)
    challenge=$(printf %s "$verifier" | openssl dgst -sha256 -binary | b64url)
    location=$(curl -sS -o /dev/null -w '%{redirect_url}' -X POST \
        "$ISS/$1/authorize?client_id=$2&response_type=code&scope=openid%20email&state=s&nonce=n&code_challenge=$challenge&code_challenge_method=S256&redirect_uri=$REDIRECT" \
        --data-urlencode username=alice --data-urlencode "claims=$3")
    code=$(echo "$location" | sed -n 's/.*[?&]code=\([^&]*\).*/\1/p')
    curl -sS -X POST "$ISS/$1/token" -d grant_type=authorization_code -d "code=$code" -d "client_id=$2" \
        -d "redirect_uri=$REDIRECT" -d "code_verifier=$verifier" | tr -d '\n' \
        | sed -n "s/.*\"$4\" *: *\"\([^\"]*\)\".*/\1/p"
}

ALICE='{"email":"alice@myorg.tld","email_verified":true,"groups":["staff","furnish-users"]}'
# by_proxy <default> <proxy>=<value>...: the value for this proxy.
by_proxy() {
    v=$1; shift
    for kv in "$@"; do [ "${kv%%=*}" = "${PROXY:-}" ] && v=${kv#*=}; done
    echo "$v"
}

# --- wait for the stack ------------------------------------------------------------------------
i=0; until [ -s $CA ] && curl -fsS --cacert $CA -o /dev/null $E/.well-known/furnish.json 2>/dev/null; do
    i=$((i + 1)); [ $i -gt 60 ] && { bad "endpoint did not come up"; exit 1; }; sleep 1; done
i=0; until GOOD=$(token acme furnish "$ALICE" id_token) && [ -n "$GOOD" ]; do
    i=$((i + 1)); [ $i -gt 60 ] && { bad "mock IdP did not come up"; exit 1; }; sleep 1; done
REFS="$E/git/acme.git/info/refs?service=git-upload-pack"
i=0; until [ "$(curl -s --cacert $CA -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $GOOD" "$REFS")" = "${READY_STATUS:-200}" ]; do
    i=$((i + 1)); [ $i -gt 60 ] && { bad "oauth2-proxy did not accept the token in time"; break; }; sleep 1; done

# checks.sh upstream-rejects: run.sh restarted the proxy with a wrong deploy token, and the token
# check with ALLOWED_GROUPS=furnish-users. 502 means a token passed the check.
if [ "${1:-}" = upstream-rejects ]; then
    req "upstream rejects the deploy token: 502, no password prompt" 502 -H "Authorization: Bearer $GOOD" "$REFS"
    grep -qi '^www-authenticate' $OUT/$n.head && bad "upstream WWW-Authenticate reached the client" \
        || ok "no WWW-Authenticate passed on from upstream"
    git -c http.extraHeader="Authorization: Bearer $GOOD" clone -q $E/git/acme.git /tmp/w/x >$OUT/x.log 2>&1 \
        && bad "git clone must fail when upstream rejects" || ok "git clone fails without prompting"
    req "ALLOWED_GROUPS: token without groups" "401 403" -H "Authorization: Bearer $(token acme furnish '{"email":"alice@myorg.tld","email_verified":true}' id_token)" "$REFS"
    req "ALLOWED_GROUPS: token with other groups" "401 403" -H "Authorization: Bearer $(token acme furnish '{"email":"alice@myorg.tld","email_verified":true,"groups":["staff"]}' id_token)" "$REFS"
    req "ALLOWED_GROUPS: the group as a string" 502 -H "Authorization: Bearer $(token acme furnish '{"email":"alice@myorg.tld","email_verified":true,"groups":"furnish-users"}' id_token)" "$REFS"
    req "ALLOWED_GROUPS: member, no email_verified (Entra ID)" 502 -H "Authorization: Bearer $(token acme furnish '{"email":"alice@myorg.tld","groups":["furnish-users"]}' id_token)" "$REFS"
    req "ALLOWED_GROUPS: member, email_verified=false" "401 403" -H "Authorization: Bearer $(token acme furnish '{"email":"alice@myorg.tld","email_verified":false,"groups":["furnish-users"]}' id_token)" "$REFS"
    exit $failed
fi

# --- tokens ------------------------------------------------------------------------------------
ACCESS=$(token acme furnish "$ALICE" access_token)       # same login, access token (aud=default)
WRONG_AUD=$(token acme other-client "$ALICE" id_token)   # right issuer, aud=other-client
OTHER_ISS=$(token evil furnish "$ALICE" id_token)        # other issuer, aud=furnish
FORGED=$(token evil furnish "{\"email\":\"alice@myorg.tld\",\"email_verified\":true,\"iss\":\"$ISS/acme\"}" id_token)
EXPIRED=$(token acme furnish "{\"email\":\"alice@myorg.tld\",\"email_verified\":true,\"exp\":$(($(date +%s) - 600))}" id_token)
WRONG_DOMAIN=$(token acme furnish '{"email":"mallory@elsewhere.tld","email_verified":true}' id_token)
UNVERIFIED=$(token acme furnish '{"email":"alice@myorg.tld","email_verified":false}' id_token)
NO_EMAIL=$(token acme furnish '{}' id_token)
NO_EV=$(token acme furnish '{"email":"alice@myorg.tld"}' id_token)
EV_STRING=$(token acme furnish '{"email":"alice@myorg.tld","email_verified":"true"}' id_token)
NOT_YET=$(token acme furnish "{\"email\":\"alice@myorg.tld\",\"email_verified\":true,\"nbf\":$(($(date +%s) + 600))}" id_token)
NBF_SKEW=$(token acme furnish "{\"email\":\"alice@myorg.tld\",\"email_verified\":true,\"nbf\":$(($(date +%s) + 60))}" id_token)
h=${GOOD%%.*}; rest=${GOOD#*.}; p=${rest%%.*}; s=${rest#*.}
TAMPERED="$h.$(printf '{"sub":"admin","aud":"furnish","iss":"%s/acme","email":"alice@myorg.tld","email_verified":true,"exp":9999999999}' $ISS | b64url).$s"
ALG_NONE="$(printf '{"alg":"none","typ":"JWT"}' | b64url).$p."

AUTH="Authorization: Bearer $GOOD"

# --- 7. discovery document ---------------------------------------------------------------------
req "furnish.json without auth" 200 $E/.well-known/furnish.json
grep -q '"collections"' $OUT/$n.body && grep -qi '^content-type: application/json' $OUT/$n.head \
    && ok "furnish.json is JSON with collections" || bad "furnish.json content/type"

# --- 1. clone and fetch, protocol v2 and v0 ----------------------------------------------------
for v in 2 0; do
    if GIT_TRACE_PACKET=$OUT/clone-v$v.trace GIT_TRACE_CURL=$OUT/clone-v$v.curl \
        git -c http.extraHeader="$AUTH" -c protocol.version=$v clone -q $E/git/acme.git /tmp/w/v$v >$OUT/clone-v$v.log 2>&1 \
        && [ -f /tmp/w/v$v/catalog.txt ]; then ok "git clone, protocol v$v"; else bad "git clone, protocol v$v"; fi
    if GIT_TRACE_CURL=$OUT/fetch-v$v.curl git -C /tmp/w/v$v -c http.extraHeader="$AUTH" -c protocol.version=$v \
        fetch -q origin >$OUT/fetch-v$v.log 2>&1; then ok "git fetch, protocol v$v"; else bad "git fetch, protocol v$v"; fi
done
grep -q 'version 2' $OUT/clone-v2.trace && ok "v2 clone really negotiated protocol v2" || bad "v2 clone did not negotiate v2"
! grep -q 'version 2' $OUT/clone-v0.trace && ok "v0 clone stayed on protocol v0" || bad "v0 clone negotiated v2"
git -c http.extraHeader="$AUTH" ls-remote $E/git/acme.git >$OUT/ls-remote.log 2>&1 && ok "git ls-remote" || bad "git ls-remote"

# --- 2./3. denied tokens -----------------------------------------------------------------------
req "no token" "401 403" "$REFS"
# Exercise batch negotiation with git-lfs and retrieve the bytes from a separate origin.
mkdir -p /tmp/storage
printf 'Furnish LFS fixture\n' > /tmp/storage/object
httpd -p 127.0.0.1:8971 -h /tmp/storage
oid=$(sha256sum /tmp/storage/object | cut -d' ' -f1)
printf 'version https://git-lfs.github.com/spec/v1\noid sha256:%s\nsize 20\n' "$oid" > /tmp/pointer
if SSL_CERT_FILE="$CA" git -C /tmp/w/v2 -c "http.$E/git/acme.git.extraHeader=$AUTH" lfs smudge fixture \
    < /tmp/pointer > /tmp/download && cmp -s /tmp/storage/object /tmp/download; then
    ok "git-lfs downloaded the expected object"
else
    bad "git-lfs batch or object download failed"
fi
req "no token, LFS batch" "401 403" -X POST "$E/git/acme.git/info/lfs/objects/batch"
req "no token, POST upload-pack" "401 403" -X POST -H 'Content-Type: application/x-git-upload-pack-request' --data-binary 0000 $E/git/acme.git/git-upload-pack
req "empty bearer" "401 403" -H "Authorization: Bearer" "$REFS"
req "garbage bearer" "401 403" -H "Authorization: Bearer not-a-jwt" "$REFS"
req "lowercase \"bearer\" scheme (oauth2-proxy only takes \"Bearer\")" 401 -H "Authorization: bearer $GOOD" "$REFS"
req "session cookie instead of a token" 401 -H "Cookie: _oauth2_proxy=forged" "$REFS"
req "ID token for another audience" "401 403" -H "Authorization: Bearer $WRONG_AUD" "$REFS"
req "ID token from another issuer" "401 403" -H "Authorization: Bearer $OTHER_ISS" "$REFS"
req "other issuer's key, forged iss claim" "401 403" -H "Authorization: Bearer $FORGED" "$REFS"
req "expired ID token" "401 403" -H "Authorization: Bearer $EXPIRED" "$REFS"
req "tampered payload, original signature" "401 403" -H "Authorization: Bearer $TAMPERED" "$REFS"
req "alg=none token" "401 403" -H "Authorization: Bearer $ALG_NONE" "$REFS"
req "access token (aud is not the client ID)" "401 403" -H "Authorization: Bearer $ACCESS" "$REFS"
req "e-mail outside ALLOWED_EMAIL_DOMAINS" "401 403" -H "Authorization: Bearer $WRONG_DOMAIN" "$REFS"
req "email_verified=false" "401 403" -H "Authorization: Bearer $UNVERIFIED" "$REFS"
req "no e-mail claim" "401 403" -H "Authorization: Bearer $NO_EMAIL" "$REFS"
# oauth2-proxy rejects only "email_verified": false and has no setting to require the claim;
# HAProxy requires JSON true unless ALLOWED_GROUPS is set (see docs/site/endpoint.md, Limits).
req "no email_verified claim" "$(by_proxy 200 haproxy=403)" -H "Authorization: Bearer $NO_EV" "$REFS"
req "email_verified=\"true\" (a string)" "$(by_proxy 200 haproxy=403)" -H "Authorization: Bearer $EV_STRING" "$REFS"
req "nbf 10 minutes ahead" "401 403" -H "Authorization: Bearer $NOT_YET" "$REFS"
req "nbf 1 minute ahead (clock skew)" 200 -H "Authorization: Bearer $NBF_SKEW" "$REFS"
req "JWT sent as Basic password" 401 -u "x:$GOOD" "$REFS"
# Two Authorization headers: refused (nginx answers 400 itself before any configuration runs).
BASIC_JWT="Authorization: Basic $(printf 'x:%s' "$GOOD" | openssl base64 -A)"
req "Basic JWT header, then a Bearer header" "400 401" -H "$BASIC_JWT" -H "$AUTH" "$REFS"
req "Bearer header, then a Basic JWT header" "400 401" -H "$AUTH" -H "$BASIC_JWT" "$REFS"
req "report: Basic JWT header, then a Bearer header" "400 401" -X POST -H "$BASIC_JWT" -H "$AUTH" -H 'Content-Type: application/json' \
    --data-binary '{"resourceLogs":[{"scopeLogs":[{"logRecords":[{"attributes":[{"key":"check","value":{"stringValue":"unauthenticated"}}]}]}]}]}' $E/v1/logs
for t in none "$WRONG_AUD" "$OTHER_ISS"; do
    if [ $t = none ]; then hdr="X-None: 1"; label="no token"; else hdr="Authorization: Bearer $t"; label="bad token"; fi
    git -c http.extraHeader="$hdr" clone -q $E/git/acme.git /tmp/w/denied >>$OUT/denied.log 2>&1 \
        && bad "git clone with $label must fail" || ok "git clone with $label fails"
    rm -rf /tmp/w/denied
done

# --- 4. push -----------------------------------------------------------------------------------
before=$(git -C /tmp/w/v2 rev-parse HEAD)
echo pushed > /tmp/w/v2/pushed.txt && git -C /tmp/w/v2 add . && git -C /tmp/w/v2 commit -qm push
for v in 2 0; do
    GIT_TRACE_CURL=$OUT/push-v$v.curl git -C /tmp/w/v2 -c http.extraHeader="$AUTH" -c protocol.version=$v \
        push -q origin HEAD:main >$OUT/push-v$v.log 2>&1 && bad "git push (v$v) must fail" || ok "git push (v$v) fails"
done
remote=$(git -c http.extraHeader="$AUTH" ls-remote $E/git/acme.git refs/heads/main | cut -f1)
[ "$remote" = "$before" ] && ok "upstream main unchanged after push attempts" || bad "upstream main changed: $remote"
req "POST git-receive-pack" 404 -H "$AUTH" -X POST -H 'Content-Type: application/x-git-receive-pack-request' --data-binary 0000 $E/git/acme.git/git-receive-pack

# --- 5. allowlist: none of these may reach anything upstream (valid token on all of them) ----------
while read -r method path; do
    [ -z "$method" ] && continue
    case $method in
        HEAD) req "$method $path" 404 -H "$AUTH" --path-as-is -I "$E$path" ;;
        POST) req "$method $path" 404 -H "$AUTH" --path-as-is -X POST -H 'Content-Type: application/x-git-upload-pack-request' --data-binary 0000 "$E$path" ;;
        *)    want=404
              # Traefik decodes %XX of unreserved characters before routing (RFC 3986 calls the forms
              # equivalent); GitLab still only gets the fixed URI. See packaging/endpoint/traefik/README.md.
              [ "${PROXY:-}" = traefik ] && [ "$path" = "/git/acme.git/info/r%65fs?service=git-upload-pack" ] && want=200
              req "$method $path" "$want" -H "$AUTH" --path-as-is -X "$method" "$E$path" ;;
    esac
done <<'EOF'
GET /git/secret.git/info/refs?service=git-upload-pack
POST /git/secret.git/git-upload-pack
GET /git/acme.git/../secret.git/info/refs?service=git-upload-pack
POST /git/acme.git/../secret.git/git-upload-pack
GET /git/acme.git/%2e%2e/secret.git/info/refs?service=git-upload-pack
GET /git/acme.git/%2E%2E/secret.git/info/refs?service=git-upload-pack
GET /git/acme.git%2f..%2fsecret.git/info/refs?service=git-upload-pack
GET /git/acme.git%2F..%2Fsecret.git/info/refs?service=git-upload-pack
GET /git/acme.git/info%2frefs?service=git-upload-pack
GET /git/acme.git/info/r%65fs?service=git-upload-pack
GET /git/acme.git/info/refs%3fservice=git-upload-pack
GET /git/acme.git/info/refs?service=git%2dupload-pack
GET /git/acme.git/info/refs?service=git-upload-pack%00
GET //git/acme.git/info/refs?service=git-upload-pack
GET /git//acme.git/info/refs?service=git-upload-pack
GET /git/acme.git//info/refs?service=git-upload-pack
GET /git/./acme.git/info/refs?service=git-upload-pack
GET /git/acme.git/./info/refs?service=git-upload-pack
GET /git/ACME.git/info/refs?service=git-upload-pack
GET /git/acme/info/refs?service=git-upload-pack
GET /git/acme.git/info/refs/?service=git-upload-pack
GET /git/acme.git/info/refs/x?service=git-upload-pack
GET /git/acme.git/info/refs;x?service=git-upload-pack
GET /git/acme.git/info/refs
GET /git/acme.git/info/refs?
GET /git/acme.git/info/refs?service=git-receive-pack
GET /git/acme.git/info/refs?service=git-upload-pack&service=git-receive-pack
GET /git/acme.git/info/refs?service=git-upload-pack&x=1
GET /git/acme.git/info/refs?x=1&service=git-upload-pack
POST /git/acme.git/info/refs?service=git-upload-pack
GET /git/acme.git/info/lfs/objects/batch
POST /git/acme.git/info/lfs/objects/batch?extra=1
POST /git/secret.git/info/lfs/objects/batch
POST /git/acme.git/git-upload-pack?x=1
POST /git/acme.git/git-upload-pack/
POST /git/acme.git/../secret.git/git-upload-pack
POST /git/acme.git/git-receive-pack
GET /git/acme.git/git-upload-pack
GET /git/acme.git/HEAD
GET /git/acme.git/config
GET /git/acme.git/objects/info/packs
GET /git/acme.git/objects/info/alternates
GET /git/acme.git/objects/00/0000000000000000000000000000000000000
GET /git/acme.git/refs/heads/main
GET /git/acme.git/
GET /git/acme.git
GET /git/
GET /
GET /it/catalog.git/info/refs?service=git-upload-pack
GET /it/secret.git/info/refs?service=git-upload-pack
GET /api/v4/projects
GET /api/v4/projects?private_token=x
GET /oauth2/auth
GET /oauth2/sign_in
GET /oauth2/start
GET /.well-known/furnish.json/../../git/secret.git/info/refs?service=git-upload-pack
HEAD /git/acme.git/info/refs?service=git-upload-pack
OPTIONS /git/acme.git/info/refs?service=git-upload-pack
PUT /git/acme.git/info/refs?service=git-upload-pack
DELETE /git/acme.git/info/refs?service=git-upload-pack
PATCH /git/acme.git/info/refs?service=git-upload-pack
PUT /git/acme.git/git-upload-pack
DELETE /git/acme.git/git-upload-pack
OPTIONS /git/acme.git/git-upload-pack
EOF
# Same allowlist without a token: must also never be proxied.
req "no token: GET /git/secret.git/info/refs" "404" --path-as-is "$E/git/secret.git/info/refs?service=git-upload-pack"
req "no token: GET /api/v4/projects" "404" "$E/api/v4/projects"
# The client's own GitLab-style credentials must not get anything extra either.
req "Private-Token header on allowed path" 200 -H "$AUTH" -H "Private-Token: glpat-x" "$REFS"
# What the client says about itself never reaches GitLab (run.sh checks GitLab's log).
req "spoofed X-Forwarded-*, X-Real-IP, Forwarded" 200 -H "$AUTH" -H 'X-Forwarded-For: 6.6.6.6' -H 'X-Real-IP: 6.6.6.6' \
    -H 'X-Forwarded-Host: evil.example' -H 'X-Forwarded-Proto: spoofed' -H 'Forwarded: for=6.6.6.6;host=evil.example' \
    -H 'X-Forwarded-Port: 6666' -H 'X-Forwarded-Prefix: /spoofed' "$REFS"
# Requests Caddy cannot route to the site at all may get an empty 200 from Caddy; the body check
# in req and the upstream log check in run.sh make sure nothing was proxied.
req "Host header for another site" "200 404 421" --http1.1 -H "$AUTH" -H "Host: gitlab" "$REFS"
req "absolute-form URI to the upstream" "200 400 404 421" --http1.1 -H "$AUTH" --request-target "http://gitlab/it/secret.git/info/refs?service=git-upload-pack" "$E/"
req "OPTIONS *" "200 400 404" --http1.1 -H "$AUTH" -X OPTIONS --request-target '*' "$E/"

# --- 6. the deploy token never reaches a client ------------------------------------------------
if grep -rqF -e "$DEPLOY_TOKEN" -e "$ACME_GITLAB_AUTH" $OUT; then bad "deploy token appears in a response"
else ok "deploy token in no response (headers, bodies, git traces)"; fi
if grep -rqiE 'fake-gitlab|www-authenticate: basic|set-cookie' $OUT/*.head $OUT/*.curl; then bad "upstream auth challenge or cookie leaked"
else ok "no upstream WWW-Authenticate or Set-Cookie in responses"; fi

# --- 8. reports (POST /v1/logs, OpenTelemetry logs) ---------------------------------------------
# otlp <check> [resource attributes] [log record attributes]: an OTLP/HTTP JSON request with one
# log record whose attribute "check" names the check (run.sh looks for it in the receiver's output).
otlp() {
    printf '{"resourceLogs":[{"resource":{"attributes":[%s]},"scopeLogs":[{"scope":{"name":"furnish"},"logRecords":[{"severityText":"ERROR","body":{"stringValue":"install acme/x failed"},"attributes":[{"key":"check","value":{"stringValue":"%s"}},{"key":"furnish.event","value":{"stringValue":"install"}}%s]}]}]}]}' \
        "${2:-}" "$1" "${3:+,$3}"
}
kv() { printf '{"key":"%s","value":{"stringValue":"%s"}}' "$1" "$2"; }
EV=$(otlp events)
UNAUTH=$(otlp unauthenticated)
JSON='Content-Type: application/json'
req "report without a token" "401 403" -X POST -H "$JSON" --data-binary "$UNAUTH" $E/v1/logs
req "report with a bad token" "401 403" -X POST -H "$JSON" -H "Authorization: Bearer $WRONG_AUD" --data-binary "$UNAUTH" $E/v1/logs
req "report as JWT in a Basic password" 401 -X POST -H "$JSON" -u "x:$GOOD" --data-binary "$UNAUTH" $E/v1/logs
req "reports, signed in" 200 -X POST -H "$AUTH" -H "$JSON" --data-binary "$EV" $E/v1/logs
req "report naming another user in a header" 200 -X POST -H "$AUTH" -H "$JSON" -H 'X-Furnish-User: mallory@myorg.tld' \
    -H 'X-Auth-Request-Email: mallory@myorg.tld' --data-binary "$EV" $E/v1/logs
# A client naming users in the attributes of its resource and record: run.sh checks they are gone.
req "report naming another user in its attributes" 200 -X POST -H "$AUTH" -H "$JSON" --data-binary "$(otlp forged \
    "$(kv user.email mallory@myorg.tld),$(kv user.name mallory)" \
    "$(kv user.email mallory@myorg.tld),$(kv user.id mallory),$(kv furnish.user mallory@myorg.tld)")" $E/v1/logs
# report_of_size <bytes> <file>: one valid OTLP request of exactly that many bytes.
report_of_size() {
    pre='{"resourceLogs":[{"scopeLogs":[{"logRecords":[{"attributes":[{"key":"check","value":{"stringValue":"size"}}],"body":{"stringValue":"'
    post='"}}]}]}]}'
    { printf %s "$pre"; head -c $(($1 - ${#pre} - ${#post})) /dev/zero | tr '\0' 'x'; printf %s "$post"; } > "$2"
}
report_of_size 1015812 /tmp/r1; report_of_size 1048576 /tmp/r2; report_of_size 1048577 /tmp/r3
[ "$(wc -c < /tmp/r3)" = 1048577 ] || bad "report_of_size makes the wrong size"
req "report of 1,015,812 bytes" 200 -X POST -H "$AUTH" -H "$JSON" --data-binary @/tmp/r1 $E/v1/logs
req "report of 1 MiB" 200 -X POST -H "$AUTH" -H "$JSON" --data-binary @/tmp/r2 $E/v1/logs
req "report of 1 MiB + 1 byte" 413 -X POST -H "$AUTH" -H "$JSON" --data-binary @/tmp/r3 $E/v1/logs
# Without a length (chunked): HAProxy only checks the declared length and refuses the request.
req "report of 1 MiB + 1 byte, chunked" "$(by_proxy 413 haproxy=411)" --http1.1 -X POST -H "$AUTH" -H "$JSON" -H 'Transfer-Encoding: chunked' --data-binary @/tmp/r3 $E/v1/logs
head -c 33554433 /dev/zero > /tmp/pack
req "git-upload-pack request over 32 MiB" 413 -X POST -H "$AUTH" -H 'Content-Type: application/x-git-upload-pack-request' --data-binary @/tmp/pack $E/git/acme.git/git-upload-pack
rm -f /tmp/r1 /tmp/r2 /tmp/r3 /tmp/pack
req "GET /v1/logs" "404 405" -H "$AUTH" $E/v1/logs
req "POST /v1/logs/x" 404 -X POST -H "$AUTH" -H "$JSON" --data-binary "$EV" $E/v1/logs/x
req "POST /v1/logs?x=1" 404 -X POST -H "$AUTH" -H "$JSON" --data-binary "$EV" "$E/v1/logs?x=1"
req "POST /v1/traces" 404 -X POST -H "$AUTH" -H "$JSON" --data-binary '{}' $E/v1/traces
req "POST /events (the path before OTLP)" 404 -X POST -H "$AUTH" -H "$JSON" --data-binary "$EV" $E/events

# --- 9. bodies of requests that aren't signed in are never read -----------------------------------
# A client that isn't signed in and sends 1 MB at 20 KB/s must get its 401 at once: the proxy may
# not wait for (and hold) the body before checking the token.
head -c 1000000 /dev/zero > /tmp/slow
for http in --http1.1 --http2; do
    for path in /v1/logs /git/acme.git/git-upload-pack; do
        n=$((n + 1)); t0=$(date +%s)
        code=$(curl -sS --cacert $CA -m 8 $http --limit-rate 20k -o /dev/null -w '%{http_code}' -X POST \
            -H 'Authorization: Bearer eyJhbGciOiJSUzI1NiJ9.e30.x' --data-binary @/tmp/slow "$E$path" 2>>$OUT/$n.err)
        case $code in 401|403) ok "unsigned $http POST $path with a slow body: $code after $(($(date +%s) - t0)) s" ;;
                      *) bad "unsigned $http POST $path with a slow body: got $code, want 401 or 403 before the body arrives" ;; esac
    done
done
rm -f /tmp/slow

# --- 10. rate limit on git requests (last: it uses up bob's allowance) -------------------------
BOB=$(token acme furnish '{"email":"bob@myorg.tld","email_verified":true,"groups":["furnish-users"]}' id_token)
urls=$(i=0; while [ $i -lt 100 ]; do echo "-o /dev/null $REFS"; i=$((i + 1)); done)
codes=$(curl -sS --cacert $CA -m 60 -H "Authorization: Bearer $BOB" -w '%{http_code}\n' $urls 2>>$OUT/rate.err)
first=$(echo "$codes" | head -40 | sort -u | tr '\n' ' ')
limited=$(echo "$codes" | grep -c '^429$')
[ "$first" = "200 " ] && ok "rate limit: the first 40 git requests in a row pass" || bad "rate limit: first 40 requests got $first"
if [ "${PROXY:-}" = caddy ]; then
    # Caddy has no rate limit without a plugin (packaging/endpoint/caddy/README.md).
    [ "$limited" = 0 ] && ok "rate limit: none with Caddy" || bad "rate limit: Caddy answered 429"
else
    [ "$limited" -gt 0 ] && ok "rate limit: $limited of 100 git requests in a row got 429" || bad "rate limit: no 429 for 100 git requests in a row"
fi

echo "$n HTTP requests checked"
exit $failed
