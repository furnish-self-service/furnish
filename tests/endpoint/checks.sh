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

ALICE='{"email":"alice@myorg.tld","email_verified":true}'

# --- wait for the stack ------------------------------------------------------------------------
i=0; until [ -s $CA ] && curl -fsS --cacert $CA -o /dev/null $E/.well-known/furnish.json 2>/dev/null; do
    i=$((i + 1)); [ $i -gt 60 ] && { bad "endpoint did not come up"; exit 1; }; sleep 1; done
i=0; until GOOD=$(token acme furnish "$ALICE" id_token) && [ -n "$GOOD" ]; do
    i=$((i + 1)); [ $i -gt 60 ] && { bad "mock IdP did not come up"; exit 1; }; sleep 1; done
REFS="$E/git/acme.git/info/refs?service=git-upload-pack"
i=0; until [ "$(curl -s --cacert $CA -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $GOOD" "$REFS")" = "${READY_STATUS:-200}" ]; do
    i=$((i + 1)); [ $i -gt 60 ] && { bad "oauth2-proxy did not accept the token in time"; break; }; sleep 1; done

# checks.sh upstream-rejects: run.sh restarted Caddy with a wrong deploy token.
if [ "${1:-}" = upstream-rejects ]; then
    req "upstream rejects the deploy token: 502, no password prompt" 502 -H "Authorization: Bearer $GOOD" "$REFS"
    grep -qi '^www-authenticate' $OUT/$n.head && bad "upstream WWW-Authenticate reached the client" \
        || ok "no WWW-Authenticate passed on from upstream"
    git -c http.extraHeader="Authorization: Bearer $GOOD" clone -q $E/git/acme.git /tmp/w/x >$OUT/x.log 2>&1 \
        && bad "git clone must fail when upstream rejects" || ok "git clone fails without prompting"
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
req "JWT sent as Basic password" 401 -u "x:$GOOD" "$REFS"
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
        *)    req "$method $path" 404 -H "$AUTH" --path-as-is -X "$method" "$E$path" ;;
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

# --- 8. reports (POST /events) -----------------------------------------------------------------
EV='{"event":"install","outcome":"failed","app":"acme/x","check":"events"}'
req "report without a token" "401 403" -X POST -H 'Content-Type: application/x-ndjson' --data-binary '{"event":"x","check":"unauthenticated"}' $E/events
req "report with a bad token" "401 403" -X POST -H "Authorization: Bearer $WRONG_AUD" --data-binary '{"event":"x","check":"unauthenticated"}' $E/events
req "report as JWT in a Basic password" 401 -X POST -u "x:$GOOD" --data-binary '{"event":"x","check":"unauthenticated"}' $E/events
req "reports, signed in" "200 204" -X POST -H "$AUTH" -H 'Content-Type: application/x-ndjson' --data-binary "$EV
$EV" $E/events
req "report naming another user in a header" "200 204" -X POST -H "$AUTH" -H 'X-Furnish-User: mallory@myorg.tld' \
    -H 'X-Auth-Request-Email: mallory@myorg.tld' --data-binary "$EV" $E/events
req "report naming another user in its fields" "200 204" -X POST -H "$AUTH" \
    --data-binary '{"event":"install","check":"events","user":"mallory@myorg.tld","X-Furnish-User":"mallory@myorg.tld"}' $E/events
head -c 1200000 /dev/zero | tr '\0' 'x' > /tmp/big
req "report over 1 MiB" 413 -X POST -H "$AUTH" --data-binary @/tmp/big $E/events
req "GET /events" "404 405" -H "$AUTH" $E/events
req "POST /events/x" 404 -X POST -H "$AUTH" --data-binary "$EV" $E/events/x
req "POST /events?x=1" 404 -X POST -H "$AUTH" --data-binary "$EV" "$E/events?x=1"

echo "$n HTTP requests checked"
exit $failed
