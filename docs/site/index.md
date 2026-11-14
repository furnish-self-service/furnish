---
title: Get started
order: 1
---
# Get started

furnish is an app catalog for your organization. People pick apps from it and install, update or remove them with one click. The catalog is a git repository, called a **collection**: one [DSC v3](https://learn.microsoft.com/powershell/dsc/overview) configuration file per app. furnish runs those files with the system's own package manager on Linux, macOS and Windows.

![The Discover page: apps recommended for the user's role above the whole catalog.](images/discover.webp)

## Install furnish

furnish is in alpha: builds of the main branch are published, there are no releases yet. Linux gets packages from a signed package repository, macOS and Windows a download.

| System | How |
|---|---|
| Debian, Ubuntu, Mint | APT repository (below) |
| Fedora, Enterprise Linux 10 | DNF repository; on EL 10 enable EPEL and CRB first |
| openSUSE Leap and Tumbleweed | zypper repository |
| NixOS | the flake's NixOS module, see [For admins](admins.md#nixos) |
| macOS (Apple silicon) | [download](https://furnish-self-service.github.io/furnish/alpha/furnish-macos.dmg); not signed yet: open it once, then allow it under System Settings → Privacy & Security → Open Anyway |
| Windows (64-bit) | [download](https://furnish-self-service.github.io/furnish/alpha/furnish-windows.msi); not signed yet: in SmartScreen choose More info → Run anyway |
| Intel Macs, other systems | build from source (below) |

On Debian or Ubuntu:

```sh
sudo install -d /etc/apt/keyrings
sudo curl -fsSLo /etc/apt/keyrings/furnish.asc https://furnish-self-service.github.io/furnish/gpg-key.asc
sudo curl -fsSLo /etc/apt/sources.list.d/furnish.list https://furnish-self-service.github.io/furnish/alpha/furnish.list
sudo apt update && sudo apt install furnish
```

On Fedora and openSUSE, add `https://furnish-self-service.github.io/furnish/alpha/furnish.repo` (`dnf` reads it from `/etc/yum.repos.d/`, `zypper addrepo` takes the URL), then install `furnish`. The package contains the desktop app and the `furnish` command.

To build from source, you need git, Rust 1.91 or later and, for the desktop app, Node.js and WebKitGTK 4.1 on Linux:

```sh
git clone https://github.com/furnish-self-service/furnish.git && cd furnish
cargo install --path .                      # the furnish command
cd gui && npm ci && npm run bundle          # the desktop app's installers, in target/release/bundle/
```

## Add a collection

Start the app and add a collection's git URL on the **Collections** page, or use the command line. The example collection has a dozen real apps:

```sh
furnish collection add https://github.com/furnish-self-service/self-service-example.git
```

It asks one question, what you work on, and recommends apps for the answer. Then pick an app on **Discover** and click **Install**.

## Next

- [Using the app](using.md): installs, the queue, updates and options.
- [Writing a collection](collections.md): your own catalog.
- [For admins](admins.md): presetting collections, locking settings, branding, and installs for users without administrator rights.
