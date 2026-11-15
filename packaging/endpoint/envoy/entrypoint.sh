#!/bin/sh
# Renders envoy/envoy.yaml with the settings from the environment, then starts Envoy through the
# image's own entrypoint (which drops to the envoy user). Envoy cannot read environment variables
# in its configuration, hence this step.
set -eu
die() { echo "furnish endpoint: $*" >&2; exit 1; }

# GITLAB_URL: http(s)://host[:port], no path, no user info.
case ${GITLAB_URL:?set GITLAB_URL} in
    https://*) scheme=https port=443 rest=${GITLAB_URL#https://} ;;
    http://*) scheme=http port=80 rest=${GITLAB_URL#http://} ;;
    *) die "GITLAB_URL must start with http:// or https://" ;;
esac
rest=${rest%/}
case $rest in ''|*/*|*@*|*\[*) die "GITLAB_URL must be scheme://host[:port], without a path" ;; esac
host=${rest%%:*}
[ "$host" = "$rest" ] || port=${rest#*:}
case $port in ''|*[!0-9]*) die "GITLAB_URL has an invalid port" ;; esac
export GITLAB_HOST="$host" GITLAB_PORT="$port" GITLAB_AUTHORITY="$rest" GITLAB_SCHEME="$scheme"

# Replace ${NAME} outside comments with the variable's value; unset or empty is an error, so a
# missing deploy token cannot turn into "Authorization: Basic ". Values are restricted to
# characters that cannot break out of a YAML string.
out=/tmp/envoy.yaml
umask 077
awk '
    function fail(msg) { print "furnish endpoint: " msg > "/dev/stderr"; exit 1 }
    /^#https:/ { if (ENVIRON["GITLAB_SCHEME"] != "https") next; sub(/^#https:/, "") }
    /^[ \t]*#/ { print; next }
    {
        line = $0; res = ""
        while (match(line, /\$\{[A-Za-z_][A-Za-z0-9_]*\}/)) {
            name = substr(line, RSTART + 2, RLENGTH - 3)
            val = ENVIRON[name]
            if (val == "") fail("set " name)
            if (val ~ /[^A-Za-z0-9.:_+\/=-]/) fail(name " contains characters that are not allowed")
            res = res substr(line, 1, RSTART - 1) val
            line = substr(line, RSTART + RLENGTH)
        }
        print res line
    }
' /etc/envoy/envoy.yaml.in > "$out" || exit 1
# Envoy runs as the envoy user. It gets its own copy of the certificate and key, so the files in
# /certs can stay readable by root only; the rendered file holds the deploy tokens, so only the
# envoy user may read it.
mkdir -p /tmp/tls
cp /certs/tls.crt /certs/tls.key /tmp/tls/
if [ "$(id -u)" = 0 ]; then chown -R envoy "$out" /tmp/tls; fi

exec /docker-entrypoint.sh envoy -c "$out" "$@"
