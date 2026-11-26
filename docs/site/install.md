---
title: Install Furnish
order: 2
group: Using Furnish
previous: index
---
# Install Furnish

## Install furnish

| Platform | Install for everyone on this computer |
|---|---|
| Windows x64 | Run the [Windows MSI](https://furnish-self-service.github.io/furnish/stable/furnish-windows.msi). It includes Git and Git LFS. If SmartScreen warns and your organization permits it, choose **More info → Run anyway**. |
| macOS on Apple silicon | Open the [disk image](https://furnish-self-service.github.io/furnish/stable/furnish-macos.dmg), drag **furnish.app** into `/Applications` and open it there. Allow it under **System Settings → Privacy & Security → Open Anyway** if prompted. The app includes Git LFS. For Git, run `xcode-select --install` if needed. |
| Linux x86-64 | Follow the [distribution-specific package instructions](https://furnish-self-service.github.io/#install). DEB and RPM packages install Git and Git LFS and update with your package manager. |
| NixOS | Enable the [Furnish NixOS module](nixos.md). |

## Install Furnish without administrator rights

Use the [current-user instructions](user-install.md) on Windows, macOS or Linux. Running Furnish as your account still requires approval for catalog steps that change the system.

## Homebrew

```sh
brew install --cask furnish-self-service/tap/furnish
```

Use `brew upgrade` to update. Uninstalling the cask keeps your settings and collections unless you add `--zap`.

## Terminal command for the macOS download

Drag `furnish.app` from the disk image into Applications and open it there. In **Settings → About → Terminal command**, choose **Install**. Enter an administrator password if macOS asks, then run `furnish --version` in Terminal. The link at `/usr/local/bin/furnish` follows the app across updates. When uninstalling the app, you can remove its link with `sudo rm /usr/local/bin/furnish` after confirming that `readlink /usr/local/bin/furnish` still points into the app.

## Build from source

To build from source you need git, Rust 1.91 or later and, for the desktop app, Node.js and on Linux WebKitGTK 4.1:

```sh
git clone https://github.com/furnish-self-service/furnish.git && cd furnish
cargo install --path .                    # the furnish command
cd gui && npm ci && npm run bundle        # the desktop app's installers, in target/release/bundle/
```

## Binary archives for packagers

Each release build produces an archive ending in `_binaries.zip` on Windows or `_binaries.tar.gz` on Linux and macOS. It contains the release CLI, desktop executable, MIT license and third-party notices. The filenames identify the operating system and architecture. Verify the archive against the same release’s `SHA256SUMS`.

When assembling your own package, provide Git and Git LFS for collections and include the DSC resources for apps using Microsoft’s resource types. Furnish looks for the `dsc` directory beside the executables, under `../lib/furnish` on Linux, or under `../Resources` in a macOS app. The user distributions above provide a working reference layout.
