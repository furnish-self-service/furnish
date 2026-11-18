#!/bin/sh
# Run from Settings → About or from Terminal after dragging the app into /Applications.
set -eu
cli=/Applications/furnish.app/Contents/MacOS/furnish
bin=/usr/local/bin
link=$bin/furnish
if [ ! -x "$cli" ]; then
    echo 'Move furnish.app to /Applications first.' >&2
    exit 1
fi
if [ -L "$link" ] && [ "$link" -ef "$cli" ]; then exit 0; fi
if [ -e "$link" ] || [ -L "$link" ]; then
    echo "$link already exists. Choose which furnish command you want to keep." >&2
    exit 1
fi
# Passing the directory to ln also refuses a destination created after the check.
install='/bin/mkdir -p /usr/local/bin && /bin/ln -s /Applications/furnish.app/Contents/MacOS/furnish /usr/local/bin/'
if [ -w "$bin" ] || { [ ! -e "$bin" ] && [ -w /usr/local ]; }; then
    /bin/sh -c "$install"
else
    /usr/bin/osascript -e 'on run argv' -e 'do shell script (item 1 of argv) with administrator privileges' -e 'end run' "$install"
fi
