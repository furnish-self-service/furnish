#!/usr/bin/env bash
# Exercise the public CLI and the hidden desktop against the CI-signed next release.
set -euo pipefail
archive=$(realpath "$1")
feed=$(realpath "$2")
version=$(jq -er .version "$feed/latest.json")
channel=stable
[[ $version != *-alpha.* ]] || channel=alpha
work=$(mktemp -d)
export FURNISH_DATA_DIR="$work/data" FURNISH_UPDATE_FEED=http://127.0.0.1:8765/latest.json
python3 -m http.server 8765 --bind 127.0.0.1 --directory "$feed" >"$work/http.log" 2>&1 &
server=$!
gui="$work/furnish/bin/furnish-gui"
cleanup() {
    kill "$server" || true
    pkill -f "^$gui( |$)" || true
    rm -rf "$work"
}
trap cleanup EXIT
curl -fsS --retry 10 --retry-connrefused "$FURNISH_UPDATE_FEED" >/dev/null
cli="$work/furnish/bin/furnish"
tar -xzf "$archive" -C "$work"
"$cli" settings --update-channel "$channel"
"$cli" self-update --check | grep 'is available'
original=$("$cli" --version)
artifact=$(find "$feed/linux" -name '*.tar.gz')
mv "$artifact" "$artifact.good"
printf damaged > "$artifact"
if "$cli" self-update >"$work/error" 2>&1; then exit 1; fi
grep 'signature' "$work/error"
test "$("$cli" --version)" = "$original"
mv "$artifact.good" "$artifact"
"$cli" self-update
test "$("$cli" --version | sed 's/+alpha\./-alpha./')" = "furnish $version"
test "$("$gui" --build-info | jq -r .version)" = "$version"
# Start the original desktop again with the same channel preference. Wait for
# both the directory exchange and the new desktop process at its installed path.
rm -rf "$work/furnish"
tar -xzf "$archive" -C "$work"
"$gui" --hidden >"$work/gui.log" 2>&1 &
old=$!
for ((i=0; i<180; i++)); do
    if ! kill -0 "$old" 2>/dev/null && pgrep -f "^$gui$" >/dev/null; then
        test "$("$gui" --build-info | jq -r .version)" = "$version"
        exit 0
    fi
    sleep 1
done
cat "$work/gui.log" "$work/http.log"
exit 1
