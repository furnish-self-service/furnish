# Sourced by run.sh: report lines as furnish queues them. batch1 and batch2 print one batch each,
# per user (run.sh turns each user's lines into an OTLP request with otlp.jq and posts it with that
# user's X-Furnish-User header).
# report <device> <furnish version> <fields...>
report() {
    printf '{"time":"%s","device":"%s","os":"windows","osVersion":"10.0.26100","furnish":"%s",%s}\n' \
        "$(date -u +%Y-%m-%dT%H:%M:%S.%3NZ)" "$1" "$2" "$3"
}
op() { # op <event> <outcome> <app> <version> [error]
    err=${5:+,\"error\":\"$5\"}
    printf '"event":"%s","outcome":"%s","app":"%s","collection":"%s","version":"%s","trigger":"user","durationMs":%d%s' \
        "$1" "$2" "$3" "${3%%/*}" "$4" 4200 "$err"
}
inventory() { printf '"event":"inventory","outcome":"reported","apps":[{"app":"acme/firefox","status":"installed"},{"app":"acme/zoom","status":"needsUpdate"}]'; }

batch1() {
    case $1 in
    alice@myorg.tld)
        report 7f3a-alice 0.9.0 "$(op install succeeded acme/firefox c0ffee1)"
        report 7f3a-alice 0.9.0 "$(op install failed acme/zoom c0ffee1 'msiexec exited with 1603')"
        report 7f3a-alice 0.9.0 "$(op fallback succeeded acme/zoom 0dd0ld1)"
        report 7f3a-alice 0.9.0 "$(inventory)" ;;
    dave@myorg.tld)
        report 91bd-dave 0.9.0 "$(op install succeeded acme/firefox c0ffee1)"
        report 91bd-dave 0.9.0 "$(op update failed acme/vscode c0ffee1 'download: 404 Not Found')"
        report 91bd-dave 0.9.0 "$(inventory)" ;;
    erin@myorg.tld)
        report 5c02-erin 0.9.0 "$(op install succeeded design/gimp beef001)"
        report 5c02-erin 0.9.0 "$(op uninstall cancelled design/gimp beef001)"
        report 5c02-erin 0.9.0 '"event":"collectionUpdate","outcome":"failed","collection":"design","error":"git fetch: connection refused"'
        report 5c02-erin 0.9.0 "$(inventory)" ;;
    bob@myorg.tld)
        report e410-bob 0.10.0 "$(op install failed acme/zoom c0ffee1 'msiexec exited with 1603')"
        report e410-bob 0.10.0 "$(op install failed acme/zoom c0ffee1 'msiexec exited with 1603')"
        report e410-bob 0.10.0 "$(op fallback succeeded acme/zoom 0dd0ld1)" ;;
    esac
}

batch2() {
    case $1 in
    alice@myorg.tld)
        report 7f3a-alice 0.10.0 "$(op update succeeded acme/firefox c0ffee2)"
        report 7f3a-alice 0.10.0 '"event":"collectionUpdate","outcome":"succeeded","collection":"acme","version":"c0ffee2"'
        report 7f3a-alice 0.10.0 "$(inventory)" ;;
    bob@myorg.tld)
        report e410-bob 0.10.0 "$(op install succeeded acme/vscode c0ffee2)"
        report e410-bob 0.10.0 "$(inventory)" ;;
    carol@myorg.tld)
        report 2a77-carol 0.10.0 "$(op install succeeded acme/firefox c0ffee2)"
        report 2a77-carol 0.10.0 "$(op install failed acme/vscode c0ffee2 'checksum mismatch')"
        report 2a77-carol 0.10.0 "$(op reinstall succeeded acme/vscode c0ffee2)"
        report 2a77-carol 0.10.0 "$(inventory)" ;;
    esac
}
users="alice@myorg.tld bob@myorg.tld carol@myorg.tld dave@myorg.tld erin@myorg.tld"
