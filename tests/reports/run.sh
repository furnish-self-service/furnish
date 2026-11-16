#!/bin/sh
# Test of the reports add-on (packaging/endpoint/reports): the OpenTelemetry Collector, Loki and
# Grafana.
#
#   tests/reports/run.sh
#
# Runs packaging/endpoint/docker-compose.yml with reports/compose.yml, the files the docs show,
# plus tests/reports/compose.yml (free host ports) under a fresh compose project. Sends the reports
# of reports.sh as OTLP logs (otlp.jq) to the Collector as the proxy would, then checks what Loki stored, with the queries of
# the dashboard, and that every panel of the dashboard gets data through Grafana. Removes
# everything on exit. Prints one line per check; exits non-zero on any failure.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
endpoint=$root/packaging/endpoint
dashboard=$endpoint/reports/grafana/dashboards/furnish-reports.json
project=furnish-reports-test-$(date +%s)-$$

# docker-compose.yml requires these for oauth2-proxy, which this test doesn't start.
export OIDC_ISSUER_URL=http://unused ALLOWED_EMAIL_DOMAINS=unused
SECRETS_DIR=$(mktemp -d); export SECRETS_DIR
GRAFANA_ADMIN_PASSWORD=$(od -An -N12 -tx1 /dev/urandom | tr -d ' \n')
printf %s "$GRAFANA_ADMIN_PASSWORD" > "$SECRETS_DIR/grafana_admin_password"
printf unused > "$SECRETS_DIR/oauth2_proxy_cookie_secret"
chmod 644 "$SECRETS_DIR"/*

dc() { docker compose -p "$project" --project-directory "$endpoint" -f "$endpoint/docker-compose.yml" \
    -f "$endpoint/reports/compose.yml" -f "$here/compose.yml" "$@"; }
trap 'dc down -v -t 2 --remove-orphans >/dev/null 2>&1 || true; rm -rf "$SECRETS_DIR"' EXIT
trap 'exit 130' INT TERM

failed=0
# ok/bad always succeed, so `cond && ok || bad` is a safe if/else here.
ok()  { echo "ok   $*"; }
bad() { echo "FAIL $*"; failed=1; }
# wait_for <seconds> <command...>: retry until the command succeeds.
wait_for() { n=$1; shift; while ! "$@" >/dev/null 2>&1; do n=$((n - 1)); [ $n -gt 0 ] || return 1; sleep 1; done; }

dc up -d --quiet-pull receiver loki grafana 2>/dev/null
receiver=http://$(dc port receiver 4318) loki=http://$(dc port loki 3100) grafana=http://$(dc port grafana 3000)
# otlp <user>: post the OTLP request on stdin to the Collector as the proxy forwards it.
otlp() { curl -sf -o /dev/null -H "X-Furnish-User: $1" -H 'Content-Type: application/json' --data-binary @- "$receiver/v1/logs"; }
gf() { curl -s -u "admin:$GRAFANA_ADMIN_PASSWORD" -H 'Content-Type: application/json' "$@"; }

wait_for 60 curl -sf "$loki/ready" && ok "Loki is ready" || bad "Loki isn't ready"
wait_for 60 curl -sf -H 'Content-Type: application/json' --data-binary '{}' "$receiver/v1/logs" && ok "the Collector accepts reports" || bad "the Collector doesn't accept reports"

# Each user's reports with that user's header, as the proxy forwards them. The second batch comes
# a few seconds later: alice's device updates furnish in between, and dave's and erin's devices
# send no inventory in it.
. "$here/reports.sh"
send() {
    for u in $users; do
        lines=$($1 "$u")
        [ -z "$lines" ] || printf '%s\n' "$lines" | jq -sc -f "$here/otlp.jq" | otlp "$u" || bad "the Collector rejected $1 of $u"
    done
}
send batch1
sleep 5
send batch2
batch2_at=$(date +%s)
expected=$( (for u in $users; do batch1 "$u"; batch2 "$u"; done) | wc -l | tr -d ' ')

# loki <LogQL>: the instant query's result as sorted "labels=value" words.
loki() {
    curl -s -G "$loki/loki/api/v1/query" --data-urlencode "query=$1" \
        | jq -r '[.data.result[]? | "\(.metric | [to_entries[].value] | join(","))=\(.value[1])"] | sort | join(" ")'
}
stored() { [ "$(loki 'sum(count_over_time({service_name="furnish"}[10m]))')" = "=$expected" ]; }
wait_for 30 stored && ok "Loki stored all $expected reports" || bad "Loki has $(loki 'sum(count_over_time({service_name="furnish"}[10m]))') of $expected reports"

labels=$(curl -s "$loki/loki/api/v1/labels" | jq -r '.data | sort | join(" ")')
[ "$labels" = "event outcome service_name" ] && ok "Loki labels are only: $labels" || bad "unexpected Loki labels: $labels"
[ "$(loki 'count(sum by (user_email) (count_over_time({service_name="furnish"} | user_email=~".+@myorg.tld" [10m])))')" = "=5" ] \
    && ok "reports carry the user from X-Furnish-User" || bad "reports without the user from X-Furnish-User"

# The dashboard's own queries: expr <panel title> [refId], with the time range and $stale set.
expr() {
    jq -r --arg t "$1" --arg r "${2:-A}" '.panels[] | select(.title == $t) | .targets[] | select(.refId == $r) | .expr' "$dashboard" \
        | sed -e 's/ offset -\$__auto//' -e 's/\$__range/10m/g' -e 's/\$__auto/10m/g' -e "s/\\\$stale/${stale:-7d}/g"
}
# check <description> <expected> <panel title> [refId]
check() {
    got=$(loki "$(expr "$3" "${4:-A}")")
    [ "$got" = "$2" ] && ok "$1: $got" || bad "$1: expected '$2', got '$got' from: $(expr "$3" "${4:-A}")"
}
check "failures by app" "acme/vscode=2 acme/zoom=3" "Failed operations by app"
check "failure rate per app" "acme/firefox=0 acme/vscode=0.5 acme/zoom=1 design/gimp=0" "Failure rate per app"
check "operations per app" "acme/firefox=4 acme/vscode=4 acme/zoom=3 design/gimp=1" "Failure rate per app" C
check "fallbacks (devices per app and commit)" "acme/zoom,0dd0ld1=2" "Fallbacks: apps installed at an earlier version"
check "furnish versions from each device's latest report" "0.10.0=3 0.9.0=2" "furnish versions in use"
# Long enough to include the second batch, too short for the first.
stale=$(( $(date +%s) - batch2_at + 2 ))s
got=$(loki "$(expr "Devices without inventory for \$stale")" | sed 's/=[0-9]*//g')
[ "$got" = "5c02-erin,erin@myorg.tld 91bd-dave,dave@myorg.tld" ] \
    && ok "devices without inventory for $stale: $got" || bad "devices without inventory for $stale: expected dave's and erin's, got '$got'"
lines=$(curl -s -G "$loki/loki/api/v1/query_range" --data-urlencode "query=$(expr "Recent failures")" --data-urlencode since=10m \
    | jq -r '.data.result[].values[][1]')
[ "$(printf '%s\n' "$lines" | grep -c .)" = 6 ] && ok "recent failures: 6 lines" || bad "recent failures: expected 6 lines, got: $lines"
printf '%s\n' "$lines" | grep -qx 'carol@myorg.tld on 2a77-carol: install acme/vscode failed: checksum mismatch' \
    && printf '%s\n' "$lines" | grep -qx 'erin@myorg.tld on 5c02-erin: collectionUpdate design failed: git fetch: connection refused' \
    && ok "recent failures name user, device, app or collection, and error" || bad "recent failures lines: $lines"

# Grafana: login required, the provisioned dashboard and data source, each panel's queries.
wait_for 60 curl -sf "$grafana/api/health" && ok "Grafana is ready" || bad "Grafana isn't ready"
code=$(curl -s -o /dev/null -w '%{http_code}' "$grafana/api/search")
[ "$code" = 401 ] && ok "Grafana refuses requests without login" || bad "Grafana answered $code without login"
code=$(curl -s -o /dev/null -w '%{http_code}' -u admin:admin "$grafana/api/search")
[ "$code" = 401 ] && ok "Grafana refuses the default admin password" || bad "Grafana answered $code to admin:admin"
listed() { gf "$grafana/api/search?query=furnish%20reports" | jq -e '.[] | select(.title == "furnish reports" and .type == "dash-db")'; }
wait_for 30 listed && ok "Grafana lists the dashboard \"furnish reports\"" || bad "Grafana doesn't list \"furnish reports\": $(gf "$grafana/api/search")"
health=$(gf "$grafana/api/datasources/uid/furnish-loki/health" | jq -r .status)
[ "$health" = OK ] && ok "Grafana's Loki data source works" || bad "Loki data source health: $(gf "$grafana/api/datasources/uid/furnish-loki/health")"

# Each panel's queries as the browser sends them for the default range (30 days, about 1500
# pixels wide: 30-minute steps). Points of range queries must fall inside the range, or Grafana
# shows "Data outside time range" (Loki aligns points to the step, which can lie after "to").
stale=$(( $(date +%s) - batch2_at + 2 ))s
to=$(( $(date +%s) * 1000 )); from=$(( to - 30 * 86400000 ))
queries=$(jq -c --arg stale "$stale" '.panels[] | .title as $t | .targets[] | {title: $t, query: (.expr |= gsub("\\$stale"; $stale))}' "$dashboard")
while read -r q; do
    title=$(printf '%s\n' "$q" | jq -r '"\(.title) (\(.query.refId))"')
    response=$(printf '%s\n' "$q" | jq -c --arg from "$from" --arg to "$to" \
        '{from: $from, to: $to, queries: [.query + {intervalMs: 1800000, maxDataPoints: 1500}]}' | gf -d @- "$grafana/api/ds/query")
    result=$(printf '%s\n' "$response" | jq -c --argjson from "$from" --argjson to "$to" '.results[] | {status, error,
        rows: ([.frames[]?.data.values[0] // [] | length] | add // 0),
        outside: [.frames[]? | select(.schema.fields[0].type == "time") | .data.values[0][] | select(. < $from or . > $to)] | length}')
    printf '%s\n' "$result" | jq -e '.status == 200 and .error == null and .rows > 0 and .outside == 0' >/dev/null \
        && ok "panel $title: $(printf '%s\n' "$result" | jq .rows) rows in the time range" || bad "panel $title: $result"
    case $title in
    "furnish versions in use (A)")
        got=$(printf '%s\n' "$response" | jq -r '[.results.A.frames[] | "\(.schema.fields[1].labels.service_version)=\(.data.values[1][0])"] | sort | join(" ")')
        [ "$got" = "0.10.0=3 0.9.0=2" ] && ok "panel $title: one series per version: $got" || bad "panel $title: expected 0.10.0=3 0.9.0=2, got $got" ;;
    esac
done <<EOF
$queries
EOF

# How the panels show results. The time series must ask for a range; the versions bar gauge must
# show one bar per row, as Grafana's browser turns an instant query into one table (Time, labels,
# "Value #<refId>"); tables must rename those value columns.
jq -e '.panels[] | select(.type == "timeseries") | .targets[] | .queryType == "range" and .range == true and .instant == false' "$dashboard" >/dev/null \
    && ok "the time series panel runs range queries" || bad "the time series panel doesn't run a range query"
jq -e '.panels[] | select(.title == "furnish versions in use") | .options.reduceOptions.values == true' "$dashboard" >/dev/null \
    && ok "the versions panel shows one bar per version" || bad "the versions panel reduces all versions to one value"
unnamed=$(jq -r '.panels[] | select(.type == "table") | .title as $t
    | ([.transformations[]? | select(.id == "organize") | .options.renameByName | keys[]]) as $renamed
    | .targets[] | "Value #\(.refId)" | select(. as $v | $renamed | index($v) | not) | "\($t): \(.)"' "$dashboard")
[ -z "$unnamed" ] && ok "every table names its value columns" || bad "table columns without a name: $unnamed"

# Made-up values in the fields Loki uses as labels: each new value would be a new stream, and Loki
# refuses reports once there are too many. They are stored, with "other" as label.
made_up=$(i=0; while [ $i -lt 200 ]; do
    printf '{"time":"%s","device":"ffff-mallory","os":"linux","osVersion":"1","furnish":"0.10.0","event":"ev%d","outcome":"out%d","collection":"c%d"}\n' \
        "$(date -u +%Y-%m-%dT%H:%M:%SZ)" $i $i $i
    i=$((i + 1)); done)
printf '%s\n' "$made_up" | jq -sc -f "$here/otlp.jq" | otlp mallory@myorg.tld || bad "the Collector rejected the made-up reports"
others() { [ "$(loki 'sum(count_over_time({service_name="furnish", event="other", outcome="other"}[10m]))')" = "=200" ]; }
wait_for 30 others && ok "reports with made-up event and outcome are stored under \"other\"" \
    || bad "made-up reports: $(loki 'sum by (event, outcome) (count_over_time({service_name="furnish"}[10m]))')"
events=$(curl -s "$loki/loki/api/v1/label/event/values" | jq -r '.data | sort | join(" ")')
outcomes=$(curl -s "$loki/loki/api/v1/label/outcome/values" | jq -r '.data | sort | join(" ")')
[ "$events" = "collectionUpdate fallback install inventory other reinstall uninstall update" ] && ok "event labels: $events" || bad "event labels: $events"
[ "$outcomes" = "cancelled failed other reported succeeded" ] && ok "outcome labels: $outcomes" || bad "outcome labels: $outcomes"
streams=$(curl -s -G "$loki/loki/api/v1/series" --data-urlencode 'match[]={service_name="furnish"}' | jq '.data | length')
[ "$streams" -le 40 ] && ok "Loki streams: $streams" || bad "Loki streams: $streams, want at most 40"

# A client that names a user itself, at every level of the request, with a time a day ahead: the
# Collector replaces the user with the proxy's and Loki gets the time the report arrived.
s() { printf '{"key":"%s","value":{"stringValue":"%s"}}' "$1" "$2"; }
ahead=$(( ($(date +%s) + 86400) * 1000000000 ))
printf '{"resourceLogs":[{"resource":{"attributes":[%s,%s,%s]},"scopeLogs":[{"scope":{"name":"furnish","attributes":[%s]},"logRecords":[{"timeUnixNano":"%s","body":{"stringValue":"forged"},"attributes":[%s,%s,%s,%s,%s]}]}]}]}' \
    "$(s service.name furnish)" "$(s device.id ffff-forger)" "$(s user.email mallory@myorg.tld)" "$(s user.name mallory)" "$ahead" \
    "$(s furnish.event install)" "$(s furnish.outcome failed)" "$(s user.email mallory@myorg.tld)" "$(s user.id mallory)" "$(s furnish.user mallory@myorg.tld)" \
    | otlp frank@myorg.tld || bad "the Collector rejected the forged report"
forged() { [ "$(loki 'sum by (user_email, user_name, user_id, furnish_user) (count_over_time({service_name="furnish"} | device_id="ffff-forger" [10m]))')" = "frank@myorg.tld=1" ]; }
wait_for 30 forged && ok "a user the client names is replaced by the proxy's, and a time ahead by the arrival" \
    || bad "forged report: $(loki 'sum by (user_email, user_name, user_id, furnish_user) (count_over_time({service_name="furnish"} | device_id="ffff-forger" [10m]))')"

if [ $failed -ne 0 ]; then
    echo "--- container logs ---"
    dc logs --no-color --tail 50
    echo "FAILED"
else
    echo "PASSED"
fi
exit $failed
