#!/bin/sh
# Furnish/NixOS: an app's packages and NixOS options in the computer's NixOS configuration, through
# furnish's local flake (src/nixos_apps.rs). DSC passes the properties as JSON on stdin; furnish
# sets FURNISH_EXE to its own program.
[ -n "${FURNISH_EXE:-}" ] || { echo "furnish: FURNISH_EXE isn't set" >&2; exit 10; }
exec "$FURNISH_EXE" nixos-resource "$@"
