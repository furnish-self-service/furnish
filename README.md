<img src="logo.svg" alt="furnish logo: a white armchair on a teal tile" width="96" align="right">

# furnish

A self-service app catalog for Linux, macOS and Windows. Your organization lists apps in git, and users install them with one command.

**Status:** early prototype. The CLI, the desktop app and the package resource work on Linux, macOS and Windows. Apps run with administrator rights, so only add collections you trust.

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

## Desktop app

`gui/` is a desktop app for the same features: browse and search the catalog, install, update and uninstall apps, and manage collections. It uses [Tauri 2](https://tauri.app) with a Svelte frontend and calls the furnish library directly, without going through the CLI.

```sh
cd gui
npm ci
npx tauri dev -- --profile fast   # development; the fast profile avoids DSC's pwsh build step
npx tauri build                   # release build with installers in target/release/bundle/
```

On Linux you need WebKitGTK 4.1, for example `webkit2gtk-4.1` on Arch or `libwebkit2gtk-4.1-dev` on Debian and Ubuntu. CI builds installers for Linux, macOS and Windows.

`tests/ui/run.sh` runs UI tests against the real app: Ubuntu 24.04 in Docker, Xvfb, and `tauri-driver`, with a fake package manager. Without arguments it also builds the app in that container, so it works on any Linux host with Docker and Node.

Each app's detail page has three collapsed sections for experts:

- **Steps** previews installing, updating or uninstalling, without changing anything: each step, the exact command and whether it needs administrator rights. While an operation runs, it follows those steps live: done, current and upcoming steps, with start time, duration and result.
- **Activity log** shows each DSC step and the package manager's output during install, update or uninstall.
- **DSC configuration** shows the app's file.

The CLI has the same features: `furnish install <app> --dry-run` and `furnish uninstall <app> --dry-run` preview the steps, and `--verbose` shows the log.

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

## Writing apps and resources

- [DSC v3 guide](docs/dsc-v3-guide.md): how DSC v3.3 behaves, verified against its source code, with the consequences for furnish.
- [authoring-dsc-v3 skill](.claude/skills/authoring-dsc-v3/SKILL.md): rules and a tested example for writing DSC v3 documents and resources. Claude Code loads it automatically in this repository. To use it in your own repository, copy `.claude/skills/authoring-dsc-v3/` there.

## Development

Use the cargo aliases `cargo b`, `cargo t` and `cargo r`. A plain `cargo build` fails without PowerShell, because one DSC build script calls `pwsh` in debug builds. The `fast` profile in `Cargo.toml` avoids that. `tests/e2e.sh` runs install and uninstall against a fake package manager.

`vendor/tree-sitter-dscexpression` holds grammar sources that DSC generates at build time. Regenerate them with `scripts/vendor-dsc-grammar.sh <tag>` when you update the pinned DSC tag.

## License

MIT
