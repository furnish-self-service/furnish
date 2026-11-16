---
title: Get started
order: 1
---
# Get started

furnish is an app catalog for an organization's computers. People pick an app and click Install; furnish installs it with the system's package manager on Windows, macOS or Linux, and keeps it updated.

The catalog is a git repository, called a collection. Each app in it is a short configuration file in [DSC](https://learn.microsoft.com/powershell/dsc/overview), Microsoft's open-source format for describing what a computer should have installed.

![The Discover page: apps recommended for the user's role, then the whole catalog.](images/discover.webp)

## Install furnish

furnish is in alpha. Each change to the main branch is built and published; there are no releases yet.

| System | How |
|---|---|
| Windows, 64-bit | [download](https://furnish-self-service.github.io/furnish/alpha/furnish-windows.msi) |
| macOS with Apple silicon | [download](https://furnish-self-service.github.io/furnish/alpha/furnish-macos.dmg) |
| Linux | the signed package repository, below |
| Intel Macs, other systems | build from source, below |

The Windows and macOS builds aren't signed yet. On Windows, choose More info → Run anyway when SmartScreen warns. On macOS, open furnish once, then allow it under System Settings → Privacy & Security → Open Anyway. Download a new alpha to update; furnish doesn't update itself on these systems yet.

furnish uses git. The Windows installer includes it ([MinGit](https://github.com/git-for-windows/git/wiki/MinGit) with Git LFS); furnish uses a git you installed yourself first. On macOS, git comes with Apple's Command Line Tools: if they are missing, macOS offers to install them the first time furnish adds a collection, or run `xcode-select --install`.

On Linux, furnish comes from a signed package repository, so updates arrive with the system's other updates. The package contains the desktop app and the `furnish` command, and brings git along. On Debian, Ubuntu and their derivatives:

```sh
sudo install -d /etc/apt/keyrings
sudo curl -fsSLo /etc/apt/keyrings/furnish.asc https://furnish-self-service.github.io/furnish/gpg-key.asc
sudo curl -fsSLo /etc/apt/sources.list.d/furnish.list https://furnish-self-service.github.io/furnish/alpha/furnish.list
sudo apt update && sudo apt install furnish
```

On Fedora and Enterprise Linux 10, save `https://furnish-self-service.github.io/furnish/alpha/furnish.repo` in `/etc/yum.repos.d/` and run `sudo dnf install furnish` (on Enterprise Linux 10, enable EPEL and CRB first). On openSUSE, run `sudo zypper addrepo` with the same URL, then `sudo zypper install furnish`. On NixOS, use the flake's NixOS module and its binary cache `furnish.cachix.org` (Nix asks once whether to trust it; without it, Nix builds from source), see [For admins](admins.md#nixos).

To build from source you need git, Rust 1.91 or later and, for the desktop app, Node.js and on Linux WebKitGTK 4.1:

```sh
git clone https://github.com/furnish-self-service/furnish.git && cd furnish
cargo install --path .                    # the furnish command
cd gui && npm ci && npm run bundle        # the desktop app's installers, in target/release/bundle/
```

## Add a collection

In the app, open Collections and enter one of these:

- your organization's furnish address, such as `furnish.myorg.tld`, if it has one. You sign in with your organization's account in the browser, and furnish adds its collections.
- a collection's git URL, for example the example collection with a dozen real apps: `https://github.com/furnish-self-service/self-service-example.git`

The command line takes the same input:

```sh
furnish collection add https://github.com/furnish-self-service/self-service-example.git
```

The example collection asks what you work on and recommends apps for the answer. Pick an app on Discover and click Install.

## Next

- [Using the app](using.md): installing, the queue, updates and options.
- [Writing a collection](collections.md): a catalog for your organization.
- [For admins](admins.md): presetting collections, locking settings, branding, sign-in without VPN, and installs for people without administrator rights.
