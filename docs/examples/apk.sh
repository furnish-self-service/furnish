#!/bin/sh
# Alpine example for an admin-preset Furnish collection. Install jq on the host.
# Use concrete package names and untagged repositories. Status and previews read the
# installed database and cached indexes. Keep those indexes current with apk update.
# Without a version requirement this example ensures presence. An explicit target
# uses apk's name=version world constraint. Furnish compares ranges generically.
# Run writes with the Furnish service or another account already authorized to use apk.
set -eu

fail() { printf '%s\n' "$*" >&2; exit 1; }
case ${1:-} in get|test|what-if|set) operation=$1 ;; *) fail 'expected get, test, what-if or set' ;; esac
request=$(jq -ces '
    if length == 1 then .[0] else error("expected one package request") end
    | select(.packageManager == "apk" and (.name | type == "string" and test("^[a-z0-9][a-z0-9+_.-]*\\z")))
    | select((has("_exist") | not) or (._exist | type == "boolean"))
    | select((has("targetVersion") | not) or (.targetVersion | type == "string" and test("^[0-9][0-9a-zA-Z._+-]*\\z")))
    | select((has("reportVersions") | not) or .reportVersions == "latest" or .reportVersions == "all")
') || fail 'invalid apk package request'
name=$(printf '%s' "$request" | jq -r .name)
want=$(printf '%s' "$request" | jq -r 'if has("_exist") then ._exist else true end')
target=$(printf '%s' "$request" | jq -r '.targetVersion // ""')
report=$(printf '%s' "$request" | jq -r '.reportVersions // ""')

read_installed() {
    packages=$(apk --no-network list --installed -- "$name")
    version=$(printf '%s\n' "$packages" | awk -v prefix="$name-" 'index($1, prefix) == 1 { print substr($1, length(prefix)+1); exit }')
    if [ -n "$version" ]; then exists=true; else exists=false; fi
}
in_desired_state() {
    [ "$want" = "$exists" ] && { [ "$want" = false ] || [ -z "$target" ] || [ "$version" = "$target" ]; }
}

read_installed
step=
if ! in_desired_state; then
    if [ "$want" = true ]; then
        package=$name${target:+=$target}
        step="apk add -- $package"
        [ "$operation" != set ] || apk add -- "$package" >&2
    else
        step="apk del -- $name"
        [ "$operation" != set ] || apk del -- "$name" >&2
    fi
    case $operation in
        what-if) exists=$want; version=$target; [ "$want" = true ] || version= ;;
        set) read_installed; in_desired_state || fail "apk did not reach the requested state for $name" ;;
    esac
fi

latest= available=
if [ -n "$report" ]; then
    policy=$(apk --no-network policy -- "$name")
    available=$(printf '%s\n' "$policy" | awk '
        /^  [^ ]+:$/ { version=$1; sub(/:$/, "", version); next }
        /^    / && $1 != "lib/apk/db/installed" && $1 !~ /^@/ && version { print version; version="" }
    ')
    for candidate in $available; do
        if [ -z "$latest" ] || [ "$(apk version --test "$candidate" "$latest")" = '>' ]; then latest=$candidate; fi
    done
fi
if in_desired_state; then desired=true; else desired=false; fi
jq -n --arg name "$name" --argjson exists "$exists" --arg version "$version" --arg op "$operation" \
    --argjson desired "$desired" --arg step "$step" --arg report "$report" --arg latest "$latest" --arg available "$available" '
    {name: $name, _exist: $exists}
    + (if $version != "" then {version: $version} else {} end)
    + (if $op == "test" then {_inDesiredState: $desired} else {} end)
    + (if $op == "what-if" then {_metadata: {whatIf: [$step | select(. != "")]}} else {} end)
    + (if $report != "" then {via: "apk", versionScheme: "generic", supportsVersionPin: true} else {} end)
    + (if $latest != "" then {latestVersion: $latest} else {} end)
    + (if $report == "all" then {availableVersions: ($available | split("\n") | map(select(. != "")))} else {} end)'
