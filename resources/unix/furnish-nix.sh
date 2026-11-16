# Nix packages declared in a NixOS or Home Manager configuration. Sourced by
# furnish-package.dsc.resource.sh; pure sh (nixos/nix has no sed).
#
# Declared packages land in buildEnv symlink farms: /run/current-system/sw
# (environment.systemPackages), /etc/profiles/per-user/<user> (users.users.<user>.packages, and
# Home Manager's packages with useUserPackages), Home Manager's home-path. A farm's store
# references are exactly the packages that went in, not their dependencies. Store paths name a
# package "<pname>-<version>[-<output>]"; furnish knows it by its nixpkgs attribute (jq,
# kdePackages.kdenlive, python3Packages.requests), so it matches the attribute's last part first
# and else asks nixpkgs for the attribute's name (cached in $FURNISH_CACHE_DIR/nix).

# "<source>=<path>" per line; source: nixos or home-manager. FURNISH_NIX_ROOTS replaces them, for
# tests only.
nix_roots() {
    if [ -n "${FURNISH_NIX_ROOTS+set}" ]; then printf '%s\n' "$FURNISH_NIX_ROOTS"; return; fi
    u=${USER:-$(id -un 2>/dev/null || echo root)}
    printf '%s\n' "nixos=/run/current-system/sw" "nixos=/etc/profiles/per-user/$u" \
        "home-manager=${HOME:-/nonexistent}/.local/state/nix/profiles/home-manager/home-path" \
        "home-manager=/nix/var/nix/profiles/per-user/$u/home-manager/home-path"
}

# The packages in the roots: "<source> <name>" per line, name without the store hash. Farms inside
# a farm (Home Manager's in the per-user one) are opened once more.
nix_declared() {
    command -v nix-store >/dev/null 2>&1 || return 0
    nix_roots | while IFS= read -r root; do
        src=${root%%=*} dir=${root#*=}
        [ -n "$dir" ] && [ -e "$dir" ] || continue
        nix-store -q --references "$dir" 2>/dev/null | while IFS= read -r p; do
            n=${p##*/} n=${n#*-}
            case $n in
            home-manager-path) inner=home-manager ;;
            user-environment | system-path) inner=$src ;;
            *) echo "$src $n"; continue ;;
            esac
            nix-store -q --references "$p" 2>/dev/null | while IFS= read -r q; do
                q=${q##*/}; echo "$inner ${q#*-}"
            done
        done
    done
}

# Prints the version if store name $1 is package $2: "<stem>-<version>" (the version starts with a
# digit) or "<stem>", each with an optional "-<output>". "?" without a version.
nix_name_version() {
    n=$1 s=$2
    for o in bin man dev lib out doc info data; do
        case $n in *-"$o") n=${n%-"$o"}; break ;; esac
    done
    case $n in
    "$s") echo '?' ;;
    "$s"-[0-9]*) echo "${n#"$s"-}" ;;
    *) return 1 ;;
    esac
}

# The stems attribute $1 may have in store names: its last part, and for python3Packages.x the
# interpreter's prefix ("python3.12-x").
nix_guess() {
    a=$1 last=${1##*.}
    echo "$last"
    case $a in python*Packages.*) echo "python*-$last" ;; esac
}

# The stem nixpkgs gives attribute $1 ($ref): its name without "-<version>". Not its outPath:
# unfree packages fail there. Cached per reference and attribute; nothing on failure (offline).
nix_stem() {
    a=$1 file=
    if [ -n "${FURNISH_CACHE_DIR:-}" ]; then
        file="$FURNISH_CACHE_DIR/nix/$(printf '%s' "$ref#$a" | tr '/:?=&#' '______')"
        [ -f "$file" ] && { cat "$file"; return 0; }
    fi
    # shellcheck disable=SC2016
    s=$(nix eval --raw "$ref#$a" --apply 'p: let n = p.name; v = p.version or ""; s = builtins.substring 0 (builtins.stringLength n - builtins.stringLength v - 1) n; in if v != "" && n == "${s}-${v}" then s else p.pname or n' 2>/dev/null) || return 0
    [ -n "$s" ] || return 0
    if [ -n "$file" ] && mkdir -p "${file%/*}" 2>/dev/null; then
        printf '%s\n' "$s" >"$file.$$" && mv -f "$file.$$" "$file"
    fi
    echo "$s"
}

# Prints "<source> <version>" of the first line "<source> <name>" on stdin whose name matches one
# of the stems given; "python*-<stem>" stands for any interpreter prefix.
nix_find() {
    while read -r src n; do
        for s in "$@"; do
            case $s in
            python\*-*)
                case $n in python[0-9]*-*) ;; *) continue ;; esac
                # The prefix ends at the first '-': python3.12-requests-2.32.3.
                v=$(nix_name_version "${n#*-}" "${s#python\*-}") && { echo "$src $v"; return 0; }
                ;;
            *) v=$(nix_name_version "$n" "$s") && { echo "$src $v"; return 0; } ;;
            esac
        done
    done
    return 1
}

# Sets declared_by (nixos, home-manager or empty) and declared_version for attribute $1.
nix_lookup() {
    declared_by= declared_version=
    list=$(nix_declared)
    [ -n "$list" ] || return 0
    # The guesses split into words, but aren't file patterns (python*-x).
    set -f
    # shellcheck disable=SC2046
    found=$(printf '%s\n' "$list" | nix_find $(nix_guess "$1")) || found=
    set +f
    if [ -z "$found" ]; then
        stem=$(nix_stem "$1")
        [ -n "$stem" ] || return 0
        found=$(printf '%s\n' "$list" | nix_find "$stem") || return 0
    fi
    declared_by=${found%% *} declared_version=${found#* }
}
