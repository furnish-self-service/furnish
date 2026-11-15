#!/bin/sh
# furnish against the endpoint (runs in the test client container, see run.sh): add it by its
# address, sign in through the browser (here: a script posting the fake provider's login form),
# get its collection over git with the token, keep updating it; signed out, it asks to sign in.
set -u
CA=/caddy-data/caddy/pki/authorities/local/root.crt
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
export HOME=/tmp FURNISH_DATA_DIR=/tmp/furnish FURNISH_TOKEN_STORE=file FURNISH_BROWSER=/usr/local/bin/fake-browser

out=$(furnish collection add furnish.test 2>&1)
echo "$out" | grep -q "Added 'acme'" && ok "added furnish.test: signed in, got its collection" || bad "add: $out"
grep -q "code_challenge_method=S256" /tmp/sign-in-url && ok "signs in with PKCE" || bad "no PKCE in $(cat /tmp/sign-in-url)"
furnish collection update acme >/dev/null 2>&1 && ok "updates with the token" || bad "update failed"
! grep -rq "Bearer\|eyJ" /tmp/furnish/collections/acme/.git/config && ok "the token isn't in the repository's config" || bad "token in .git/config"
[ "$(stat -c %a /tmp/furnish/endpoints/furnish.test.token)" = 600 ] && ok "the sign-in is readable only by its owner" || bad "token file permissions"
furnish collection list 2>/dev/null | grep -q "acme" && ok "lists the collection" || bad "not listed"

furnish logout furnish.test >/dev/null
out=$(furnish collection update acme 2>&1) && bad "updated while signed out"
echo "$out" | grep -q "sign in to Acme first: furnish login furnish.test" && ok "signed out, it says how to sign in" || bad "unexpected: $out"
out=$(furnish login furnish.test 2>&1) && echo "$out" | grep -q "Signed in to Acme" && ok "furnish login signs in again" || bad "login: $out"
furnish collection update acme >/dev/null 2>&1 && ok "updates again after signing in" || bad "update after login failed"

out=$(furnish collection add https://furnish.test/git/acme.git --name direct 2>&1)
echo "$out" | grep -q "Added 'direct'" && ok "a collection URL at the endpoint uses the sign-in too" || bad "direct URL: $out"
exit $failed
