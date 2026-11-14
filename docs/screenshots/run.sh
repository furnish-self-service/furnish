#!/bin/sh
# Screenshots of the real furnish app for the website and docs: the example collection, a fake
# package manager with a few apps installed and one update, rendered at 2x in Docker.
#
#   docs/screenshots/run.sh <collection repo> <output dir>
#   e.g. docs/screenshots/run.sh ../self-service-example docs/site/images
set -eu
repo=$(cd "$(dirname "$0")/../.." && pwd)
collection=$(realpath "${1:?usage: $0 <collection repo> <output dir>}")
out=$(realpath -m "${2:?usage: $0 <collection repo> <output dir>}")
work=$(mktemp -d)
container=furnish-screenshots-$$
cleanup() { docker rm -f "$container" >/dev/null 2>&1 || true; rm -rf "$work"; }
trap cleanup EXIT
mkdir -p "$out"

docker build -q --target screenshots -t furnish-screenshots "$repo/tests/ui" >/dev/null
(cd "$repo/gui" && npm run -s build)
docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp -v "$repo:/src" -v "$HOME/.cargo/registry:/opt/cargo/registry" \
    -w /src -e CARGO_TARGET_DIR=/src/target/ui-docker furnish-screenshots \
    cargo build -q --release -p furnish-gui --features tauri/custom-protocol
binary="$repo/target/ui-docker/release/furnish-gui"

# A fake pacman: installed packages are files in $work/pkgs, an update a .new file next to one.
mkdir -p "$work/bin" "$work/pkgs"
cat > "$work/bin/pacman" <<SH
#!/bin/sh
case "\$1" in
  -Q) [ -f "$work/pkgs/\$2" ] && echo "\$2 \$(cat "$work/pkgs/\$2")" ;;
  -Qu) [ -f "$work/pkgs/\$2.new" ] && echo "\$2 \$(cat "$work/pkgs/\$2") -> \$(cat "$work/pkgs/\$2.new")" ;;
  -S) sleep 4; if [ -f "$work/pkgs/\$4.new" ]; then mv "$work/pkgs/\$4.new" "$work/pkgs/\$4"; else echo 1.0 > "$work/pkgs/\$4"; fi ;;
  -R) rm "$work/pkgs/\$3" ;;
esac
SH
printf '#!/bin/sh\nexec "$@"\n' > "$work/bin/pkexec"
chmod +x "$work/bin/"*
for p in git htop keepassxc libreoffice-fresh; do echo 24.2.1 > "$work/pkgs/$p"; done
echo 24.8.0 > "$work/pkgs/libreoffice-fresh.new"

# The collection as its own clone (uncommitted changes included).
cp -r "$collection" "$work/example-src" && rm -rf "$work/example-src/.git"
git -C "$work/example-src" init -q && git -C "$work/example-src" add -A
git -C "$work/example-src" -c user.name=t -c user.email=t@t commit -qm example
mkdir -p "$work/data/collections"
git clone -q "$work/example-src" "$work/data/collections/example"
# Only the policy can make the collection managed; a plain policy keeps furnish's own look.
echo '{"collections": {"preset": [{"name": "example", "url": "'"$work/example-src"'"}]}}' > "$work/policy.json"
echo '{"collections": {"preset": [{"name": "example", "url": "'"$work/example-src"'"}]},
  "branding": {"name": "Acme Apps", "logo": "example/icons/collection.svg", "accent": "#c2410c"}}' > "$work/policy-branded.json"

start() {  # start the app in a fresh container: $1 policy file, $2 extra env (e.g. GTK_THEME)
    docker rm -f "$container" >/dev/null 2>&1 || true
    docker run -d -t --name "$container" --network host --user "$(id -u):$(id -g)" -e HOME=/tmp \
        -e FURNISH_DATA_DIR="$work/data" -e FURNISH_POLICY_FILE="$1" -e GDK_SCALE=2 -e "${2:-NO_EXTRA=1}" \
        -e PATH="$work/bin:/usr/local/bin:/usr/bin:/bin" \
        -v "$work:$work" -v "$binary:/usr/local/bin/furnish-gui:ro" furnish-screenshots \
        sh -c 'Xvfb :99 -screen 0 2400x1600x24 >/dev/null 2>&1 & export DISPLAY=:99; exec tauri-driver --port 4444' >/dev/null
}
shoot() { node "$repo/docs/screenshots/shoot.mjs" "$1" "$work" "$out"; }
start "$work/policy.json" && shoot main
start "$work/policy-branded.json" && shoot branded
start "$work/policy.json" GTK_THEME=Adwaita:dark && shoot dark
# Phone crops: cut each window screenshot to its element (x.crop holds WxH+X+Y in pixels).
docker run --rm --user "$(id -u):$(id -g)" -v "$out:$out" furnish-screenshots sh -c \
    'for c in '"$out"'/*.crop; do convert "${c%.crop}.png" -crop "$(cat "$c")" +repage "${c%.crop}.png" && rm "$c"; done'
# The branded look for phones: the top left (logo, name, the first accent-colored buttons).
docker run --rm --user "$(id -u):$(id -g)" -v "$out:$out" furnish-screenshots \
    convert "$out/branded.png" -crop 1100x700+0+0 +repage "$out/branded-mobile.png"
# WebP for the web: a fraction of the PNG size at the same look.
docker run --rm --user "$(id -u):$(id -g)" -v "$out:$out" furnish-screenshots \
    sh -c 'for f in '"$out"'/*.png; do convert "$f" -quality 88 "${f%.png}.webp" && rm "$f"; done'
ls -l "$out"
