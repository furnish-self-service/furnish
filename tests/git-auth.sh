#!/bin/sh
# Private git collections, like plain git: HTTPS with a username and password (or token), kept
# by furnish; SSH with a key file or the agent. Runs a git server and a client in Docker.
#
#   tests/git-auth.sh <path/to/furnish>   (a binary that runs on Debian trixie)
set -eu
if [ "${1:-}" != --inside ]; then
    binary=$(realpath "${1:?usage: $0 <furnish-binary>}")
    here=$(cd "$(dirname "$0")" && pwd)
    net=furnish-git-auth-$$
    cleanup() { docker rm -f "$net-server" >/dev/null 2>&1; docker network rm "$net" >/dev/null 2>&1 || true; }
    trap cleanup EXIT
    docker build -q -t furnish-git-auth "$here/git-auth" >/dev/null
    docker network create "$net" >/dev/null
    ssh-keygen -q -t ed25519 -N '' -f "/tmp/$net-key" && chmod 644 "/tmp/$net-key"
    docker run -d --name "$net-server" --network "$net" --network-alias server -v "/tmp/$net-key.pub:/key.pub:ro" furnish-git-auth sh -c '
        for r in apps other; do git init -q --bare /srv/git/$r.git; done
        mkdir -p /tmp/w && cd /tmp/w && git init -q -b main && mkdir apps &&
        printf "metadata:\n  furnish:\n    description: Private app\nresources: []\n" > apps/private.dsc.yaml &&
        git add -A && git -c user.name=t -c user.email=t@t commit -qm private && git push -q /srv/git/apps.git main &&
        git --git-dir=/srv/git/apps.git symbolic-ref HEAD refs/heads/main &&
        chown -R www-data /srv/git && git config --system --add safe.directory "*" && cp /key.pub /home/git/.ssh/authorized_keys && chown -R git /home/git/.ssh &&
        ln -s /srv/git /home/git/git && chown -h git /home/git/git && chmod -R a+rX /srv/git &&
        /usr/sbin/sshd && exec apachectl -D FOREGROUND' >/dev/null
    docker run --rm --network "$net" -v "$binary:/usr/local/bin/furnish:ro" -v "/tmp/$net-key:/key:ro" \
        -v "$here/git-auth.sh:/t.sh:ro" furnish-git-auth sh /t.sh --inside
    status=$?; rm -f "/tmp/$net-key" "/tmp/$net-key.pub"; exit $status
fi

fail() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "ok    $*"; }
for _ in $(seq 50); do (exec 3<>/dev/tcp/server/80) 2>/dev/null && break; sleep 0.2; done 2>/dev/null || true
sleep 1
useradd -m user && cp /key /home/user/key && chown user /home/user/key && chmod 600 /home/user/key
as() { su user -s /bin/sh -c "export HOME=/home/user FURNISH_DATA_DIR=/home/user/data FURNISH_TOKEN_STORE=file; $1"; }
url=http://server/git/apps.git

out=$(as "furnish collection add $url --name h </dev/null 2>&1") && fail "added without credentials"
echo "$out" | grep -q "needs a username and password" && ok "HTTPS without credentials: furnish asks for them" || fail "unexpected: $out"

out=$(as "echo wrong | furnish collection add $url --name h --username alice --password-stdin 2>&1") && fail "added with a wrong password"
as "test ! -s /home/user/data/git-credentials.json || ! grep -q alice /home/user/data/git-credentials.json" && ok "wrong credentials are refused and not kept" || fail "kept wrong credentials"

as "echo s3cret-token | furnish collection add $url --name h --username alice --password-stdin >/dev/null" || fail "couldn't add with credentials"
as "furnish list" | grep -q "private" && ok "HTTPS with username and token" || fail "app missing"
as "furnish collection update h >/dev/null" && ok "updates use the kept credentials" || fail "update failed"
as "grep -rq s3cret-token /home/user/data/collections/h/.git/config" && fail "the token is in the repository's config" || ok "the token isn't in the repository's config"
[ "$(as 'stat -c %a /home/user/data/git-credentials.json')" = 600 ] && ok "the credential file is readable only by its owner" || fail "credential file permissions"

out=$(as "furnish collection add ssh://git@server/home/git/git/apps.git --name s 2>&1") && fail "unknown host key accepted"
echo "$out" | grep -q "host key isn't known" && ok "SSH: an unknown host key explains what to do" || fail "unexpected: $out"
as "mkdir -p ~/.ssh && ssh-keyscan -q server > ~/.ssh/known_hosts 2>/dev/null"
as "GIT_SSH_COMMAND='ssh -i /home/user/key' furnish collection add ssh://git@server/home/git/git/apps.git --name s >/dev/null" || fail "SSH with a key file"
ok "SSH with a key file"
as "eval \$(ssh-agent -s) >/dev/null && ssh-add -q /home/user/key 2>/dev/null && mv /home/user/key /home/user/key.away && furnish collection add ssh://git@server/home/git/git/apps.git --name a >/dev/null; rc=\$?; ssh-agent -k >/dev/null; mv /home/user/key.away /home/user/key; exit \$rc" || fail "SSH with the agent"
ok "SSH with the agent (no key file)"
echo "git auth OK"
