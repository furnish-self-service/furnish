<img src="logo.svg" alt="furnish logo: a white armchair on a teal tile" width="96" align="right">

# furnish

A self-service app catalog for Linux, macOS and Windows. Your organization lists apps in git, and users install them with one command.

**Status:** early prototype. The CLI and the package resource work on Linux and macOS. There is no GUI and no Windows support yet. Apps run with administrator rights, so only add collections you trust.

## What you get

Admins publish apps in a git repository, called a *collection*. Each app is one [DSC v3](https://learn.microsoft.com/powershell/dsc/overview) configuration file. Users manage those apps with these commands:

| Command | What it does |
|---|---|
| `furnish collection add <git-url>` | Clones a collection and makes its apps available |
| `furnish list` | Shows every app with its state: installed, update available or not installed |
| `furnish install <app>` | Brings the app to the state its file describes, and does nothing if it's already there |
| `furnish uninstall <app>` | Removes what the app installed |
| `furnish collection update` | Pulls the latest app definitions |

## Why it is different

- **No server or agent.** furnish is one binary that reads collections straight from git.
- **App state comes from the definition.** furnish runs a DSC test: it compares the machine with the app's file instead of keeping its own records.
- **DSC is built in.** The [DSC](https://github.com/PowerShell/DSC) engine is compiled into furnish as a library, so users don't install it separately.
- **Admins only need YAML.** Common cases work with the bundled `Furnish/Package` resource, which uses pacman, apt, dnf, zypper or Homebrew.

## Start here

You need git and Rust 1.85 or later.

```sh
git clone git@github.com:furnish-self-service/furnish.git
cd furnish
cargo install --path .
furnish collection add git@github.com:furnish-self-service/self-service-example.git
furnish list
```

`furnish list` prints the catalog. Symbols show each app's state:

```
   App     Collection            Categories          Description
·  cowsay  self-service-example  Fun                 Let an ASCII cow say stuff in your terminal!
✓  htop    self-service-example  System, Monitoring  Interactive process viewer for the terminal.

✓ installed  ↑ update available  · not installed  ? unknown
```

## Example app definition

An app is a file in the collection's `apps/` directory. Its name, without `.dsc.yaml`, is the app's name, and it holds a standard DSC configuration. furnish reads only `metadata.furnish`:

```yaml
# apps/cowsay.dsc.yaml
metadata:
  furnish:
    description: Let an ASCII cow say stuff in your terminal!
    categories: [Fun]
resources:
  - name: cowsay package
    type: Furnish/Package
    properties:
      name: cowsay
```

See [self-service-example](https://github.com/furnish-self-service/self-service-example) for a complete collection.

## Development

Use the cargo aliases `cargo b`, `cargo t` and `cargo r`. A plain `cargo build` fails without PowerShell, because one DSC build script calls `pwsh` in debug builds. The `fast` profile in `Cargo.toml` avoids that. `tests/e2e.sh` runs install and uninstall against a fake package manager.

`vendor/tree-sitter-dscexpression` holds grammar sources that DSC generates at build time. Regenerate them with `scripts/vendor-dsc-grammar.sh <tag>` when you update the pinned DSC tag.

## License

MIT
