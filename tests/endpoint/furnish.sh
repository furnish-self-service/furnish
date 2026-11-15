#!/bin/sh
# furnish against the endpoint (runs in the test client container, see run.sh): add it by its
# address, sign in through the browser (here: a script posting the fake provider's login form),
# get its collection over git with the token, keep updating it; signed out, it asks to sign in.
set -u
CA=/certs/ca.crt
failed=0
ok()  { echo "ok   furnish: $*"; }
bad() { echo "FAIL furnish: $*"; failed=1; }

# The organization's certificate authority, installed like on its computers.
cp $CA /usr/local/share/ca-certificates/furnish-test.crt && update-ca-certificates >/dev/null 2>&1
# The "browser": the user signs in (posting the login form), the provider sends it back to furnish.
cat > /usr/local/bin/fake-browser <<'SH'
#!/bin/sh
echo "$1" > /tmp/sign-in-url
curl -sS -L -o /dev/null "$1" --data-urlencode username=alice \
    --data-urlencode 'claims={"email":"alice@myorg.tld","email_verified":true}' >/tmp/browser.log 2>&1 &
SH
chmod +x /usr/local/bin/fake-browser
# The test identity provider speaks plain http; furnish allows that only for hosts named here.
export HOME=/tmp FURNISH_DATA_DIR=/tmp/furnish FURNISH_TOKEN_STORE=file FURNISH_BROWSER=/usr/local/bin/fake-browser FURNISH_ALLOW_HTTP=mock-oidc

# Without FURNISH_ALLOW_HTTP the plain-http identity provider is refused before anyone signs in.
out=$(FURNISH_ALLOW_HTTP= furnish collection add furnish.test 2>&1) && bad "signed in at a plain-http identity provider"
echo "$out" | grep -q "isn't an https address" && ok "refuses a plain-http identity provider" || bad "plain http: $out"
[ ! -e /tmp/furnish/endpoints/https_furnish.test.json ] && ok "keeps nothing of an address it didn't sign in to" || bad "kept the description"

out=$(furnish collection add furnish.test 2>&1)
echo "$out" | grep -q "Added 'acme'" && ok "added furnish.test: signed in, got its collection" || bad "add: $out"
echo "$out" | grep -q "You sign in at mock-oidc:8080 for furnish.test" && ok "says where people sign in" || bad "no sign-in host: $out"
echo "$out" | grep -q "Installs right away: acme/vpn$" && ok "lists the apps it installs, only from its own collections" || bad "auto-install: $out"
# What the address sets: allowed settings for everything, per-collection ones for its own only.
out=$(furnish settings 2>&1)
echo "$out" | grep -q "every 7 minutes" && ok "an address sets the update interval" || bad "update interval: $out"
echo "$out" | grep -q "automatic for all apps" && ok "an address can't turn off updates of other apps" || bad "noAutoUpdate: $out"
furnish collection settings acme 2>&1 | grep -q "when an install fails: nothing" && ok "its per-collection settings apply to its collection" \
    || bad "acme's settings: $(furnish collection settings acme 2>&1)"
echo "$out" | grep -q "when an install fails: offer the previous version" && ok "its per-collection settings don't apply to others" \
    || bad "general fallback: $out"
grep -q "code_challenge_method=S256" /tmp/sign-in-url && ok "signs in with PKCE" || bad "no PKCE in $(cat /tmp/sign-in-url)"
furnish collection update acme >/dev/null 2>&1 && ok "updates with the token" || bad "update failed"
! grep -rq "Bearer\|eyJ" /tmp/furnish/collections/acme/.git/config && ok "the token isn't in the repository's config" || bad "token in .git/config"
[ "$(stat -c %a /tmp/furnish/endpoints/https_furnish.test.token)" = 600 ] && ok "the sign-in is readable only by its owner" || bad "token file permissions"
[ "$(stat -c %a /tmp/furnish/endpoints)" = 700 ] && ok "the endpoints folder is private" || bad "endpoints folder permissions"

# The address names another identity provider now: the sign-in doesn't go there, people sign in again.
desc=/tmp/furnish/endpoints/https_furnish.test.json
cp $desc /tmp/desc.bak
sed -i 's|mock-oidc:8080/acme|mock-oidc:8080/other|' $desc
out=$(furnish collection update acme 2>&1) && bad "used a sign-in from another identity provider"
echo "$out" | grep -q "sign in to Acme first" && ok "a sign-in only counts for the identity provider it came from" || bad "issuer change: $out"
cp /tmp/desc.bak $desc
furnish collection list 2>/dev/null | grep -q "acme" && ok "lists the collection" || bad "not listed"

furnish logout furnish.test >/dev/null
out=$(furnish collection update acme 2>&1) && bad "updated while signed out"
echo "$out" | grep -q "sign in to Acme first: furnish login furnish.test" && ok "signed out, it says how to sign in" || bad "unexpected: $out"
out=$(furnish login furnish.test 2>&1) && echo "$out" | grep -q "Signed in to Acme" && ok "furnish login signs in again" || bad "login: $out"
furnish collection update acme >/dev/null 2>&1 && ok "updates again after signing in" || bad "update after login failed"

# Reports: queued, sent with the sign-in when the command ends (run.sh checks what Vector got).
furnish settings --reporting all >/dev/null
[ "$(stat -c %a /tmp/furnish/settings.json)" = 600 ] && ok "settings.json is readable only by its owner" || bad "settings.json permissions"
furnish collection update acme >/dev/null 2>&1
out=$(furnish reports 2>&1)
echo "$out" | grep -q "Sent to: Acme" && ok "furnish reports names where reports go" || bad "reports: $out"
echo "$out" | grep -q "Waiting: 0" && ok "the report was sent" || bad "reports still waiting: $out"
[ "$(stat -c %a /tmp/furnish/reports/queue.ndjson)" = 600 ] && ok "the report queue is readable only by its owner" || bad "queue permissions"
furnish settings --reporting off >/dev/null

out=$(furnish collection add https://furnish.test/git/acme.git --name direct 2>&1)
echo "$out" | grep -q "Added 'direct'" && ok "a collection URL at the endpoint uses the sign-in too" || bad "direct URL: $out"
exit $failed
