#!/bin/sh
# Entrypoint of the proxy container: HAProxy expands environment variables in its configuration
# but cannot take a URL or a list apart, so this derives the variables haproxy.cfg uses, then
# starts HAProxy in master-worker mode (SIGHUP reloads the configuration and the IdP key).
set -eu

# GITLAB_URL=https://gitlab.myorg.internal[:port] → server address, Host header, TLS or not.
case $GITLAB_URL in
    https://*) GITLAB_TLS=1 hostport=${GITLAB_URL#https://} port=443 ;;
    http://*)  GITLAB_TLS=0 hostport=${GITLAB_URL#http://} port=80 ;;
    *) echo "GITLAB_URL must start with http:// or https://: $GITLAB_URL" >&2; exit 1 ;;
esac
hostport=${hostport%/}
case $hostport in */*|"") echo "GITLAB_URL must be a base URL without a path: $GITLAB_URL" >&2; exit 1 ;; esac
case $hostport in
    *:*) GITLAB_ADDR=$hostport GITLAB_SNI=${hostport%:*} ;;
    *)   GITLAB_ADDR=$hostport:$port GITLAB_SNI=$hostport ;;
esac
GITLAB_HOST=$hostport

# ALLOWED_EMAIL_DOMAINS=a.tld,b.tld → "@a.tld @b.tld" for an ends-with match; "*" → any.
set -f
domains=$(printf '%s' "${ALLOWED_EMAIL_DOMAINS:-*}" | tr 'A-Z,' 'a-z ')
EMAIL_ANY_DOMAIN=0 EMAIL_SUFFIXES=
for d in $domains; do
    [ "$d" = "*" ] && EMAIL_ANY_DOMAIN=1
    EMAIL_SUFFIXES="$EMAIL_SUFFIXES @$d"
done
EMAIL_SUFFIXES=${EMAIL_SUFFIXES# }
[ -n "$EMAIL_SUFFIXES" ] || EMAIL_ANY_DOMAIN=1
set +f

# ALLOWED_GROUPS=a,b → patterns for the "groups" claim, as a string (groups.str) or in the JSON
# text of an array (groups.sub): ["a"], ["a",...], [...,"a",...], [...,"a"].
mkdir -p /tmp/haproxy
: > /tmp/haproxy/groups.str
: > /tmp/haproxy/groups.sub
GROUPS_REQUIRED=0
printf '%s\n' "${ALLOWED_GROUPS:-}" | tr ',' '\n' | while IFS= read -r g; do
    [ -n "$g" ] || continue
    printf '%s\n' "$g" >> /tmp/haproxy/groups.str
    printf '["%s"]\n["%s",\n,"%s",\n,"%s"]\n' "$g" "$g" "$g" "$g" >> /tmp/haproxy/groups.sub
done
[ -s /tmp/haproxy/groups.str ] && GROUPS_REQUIRED=1

export GITLAB_TLS GITLAB_ADDR GITLAB_SNI GITLAB_HOST EMAIL_ANY_DOMAIN EMAIL_SUFFIXES GROUPS_REQUIRED
exec haproxy -W -db -f /usr/local/etc/haproxy/haproxy.cfg "$@"
