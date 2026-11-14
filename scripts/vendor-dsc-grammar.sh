#!/bin/sh
# Regenerate vendor/tree-sitter-dscexpression for a DSC release tag.
# Upstream generates these sources at build time, so dsc-lib can't build as a plain git dependency.
# Usage: scripts/vendor-dsc-grammar.sh v3.3.0   (needs git and cargo)
set -eu
tag=${1:?usage: $0 <dsc-tag>}
ts_version=0.25.10  # tree-sitter-cli version DSC's build uses (helpers.build.psm1)
dest=$(cd "$(dirname "$0")/.." && pwd)/vendor/tree-sitter-dscexpression
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

git clone -q --depth 1 --branch "$tag" https://github.com/PowerShell/DSC.git "$tmp/dsc"
cargo install -q tree-sitter-cli --version "$ts_version" --locked --root "$tmp/ts"
cd "$tmp/dsc/grammars/tree-sitter-dscexpression"
"$tmp/ts/bin/tree-sitter" init --update >/dev/null
"$tmp/ts/bin/tree-sitter" generate
rm -rf "$dest/src" "$dest/bindings" "$dest/grammar.js"
mkdir -p "$dest/bindings"
cp -r src grammar.js "$dest/"
cp -r bindings/rust "$dest/bindings/"
echo "Vendored grammar from DSC $tag. Update the tag in Cargo.toml and vendor/tree-sitter-dscexpression/Cargo.toml."
