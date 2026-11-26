---
title: Get started
order: 1
---
# Get started

furnish is an app catalog for an organization's computers. People pick an app and click Install; furnish installs it with the system's package manager on Windows, macOS or Linux, and keeps it updated.

The catalog is a git repository, called a collection. Each app in it is a short configuration file in [DSC](https://learn.microsoft.com/powershell/dsc/overview), Microsoft's open-source format for describing what a computer should have installed.

![The Discover page: apps recommended for the user's role, then the whole catalog.](images/discover.webp)

## Install furnish

Choose a channel and install scope. Stable is the published v0.1.0 release. Alpha builds include newer changes and the Windows and Linux current-user installers.

| System | Install scope | Channel and download | How to update |
|---|---|---|---|
| Windows x64, MSI | All users | [Stable MSI](https://furnish-self-service.github.io/furnish/stable/furnish-windows.msi) · [Alpha MSI](https://furnish-self-service.github.io/furnish/alpha/furnish-windows.msi) | Furnish updates itself on the installed channel |
| Windows x64, current-user installer | Your account | [Alpha releases](https://github.com/furnish-self-service/furnish/releases?q=prerelease%3Atrue), asset ending in `windows-x86_64_user-setup.exe` | Run the newer current-user installer |
| macOS, Apple silicon | Your account in `~/Applications` or all users in `/Applications` | [Stable disk image](https://furnish-self-service.github.io/furnish/stable/furnish-macos.dmg) · [Alpha disk image](https://furnish-self-service.github.io/furnish/alpha/furnish-macos.dmg) | Furnish updates itself on the installed channel |
| macOS, Apple silicon, Homebrew | Homebrew installation | Stable cask (command below) | `brew upgrade` |
| Linux x86-64, DEB or RPM | System-wide | Stable repository (commands below) or its `/alpha/` counterpart | System package manager |
| Linux x86-64, user archive | Your account | [Alpha releases](https://github.com/furnish-self-service/furnish/releases?q=prerelease%3Atrue), asset ending in `linux-x86_64_user.tar.gz` | Quit Furnish and replace the extracted folder |
| NixOS | NixOS configuration | Pin the stable tag or follow the main branch | Update the flake lock and rebuild |

On Intel Macs and other platforms, see the source build instructions below.

On macOS, Homebrew installs the desktop app and the `furnish` command, and `brew upgrade` brings stable updates:

```sh
brew install --cask furnish-self-service/tap/furnish
```

The Windows and macOS builds aren't signed with a publisher certificate. On Windows, choose More info → Run anyway when SmartScreen warns. On macOS, open furnish from Applications once, then allow it under System Settings → Privacy & Security → Open Anyway. Do this before you use the `furnish` command. macOS may ask again after an update.

When furnish installs a version itself, it checks the download's signature first. It restarts into the new version once nothing is installing and its window is closed. Settings → About shows how furnish was installed and whether a new version is out. See [New versions of furnish](using.md#new-versions-of-furnish).

`brew uninstall --cask furnish` keeps your settings and collections. Add `--zap` to remove them too.

Furnish requires Git and Git LFS for collection checkouts. The Linux DEB and RPM packages install both as dependencies, and the Nix packages provide both. When using the standalone Linux CLI or building from source, install both tools and check `git lfs version`. The Windows installer includes it ([MinGit](https://github.com/git-for-windows/git/wiki/MinGit) with Git LFS); Furnish uses an installed Git first when it also provides Git LFS. The macOS app includes Git LFS. Git itself comes with Apple's Command Line Tools: if they are missing, macOS offers to install them the first time furnish adds a collection, or run `xcode-select --install`.

On Linux, furnish comes from a signed package repository, so updates arrive with the system's other updates. The package contains the desktop app and the `furnish` command, and brings git along. On Debian, Ubuntu and their derivatives:

```sh
sudo install -d /etc/apt/keyrings
sudo curl -fsSLo /etc/apt/keyrings/furnish.asc https://furnish-self-service.github.io/furnish/gpg-key.asc
sudo curl -fsSLo /etc/apt/sources.list.d/furnish.list https://furnish-self-service.github.io/furnish/stable/furnish.list
sudo apt update && sudo apt install furnish
```

On Fedora and Enterprise Linux 10, save `https://furnish-self-service.github.io/furnish/stable/furnish.repo` in `/etc/yum.repos.d/` and run `sudo dnf install furnish` (on Enterprise Linux 10, enable EPEL and CRB first). On openSUSE, run `sudo zypper addrepo` with the same URL, then `sudo zypper install furnish`. On NixOS, use the flake's NixOS module and add its binary cache `furnish.cachix.org` to `nix.settings`, or Nix builds furnish from source on every update; see [For admins](admins.md#nixos).

To build from source you need git, Rust 1.91 or later and, for the desktop app, Node.js and on Linux WebKitGTK 4.1:

```sh
git clone https://github.com/furnish-self-service/furnish.git && cd furnish
cargo install --path .                    # the furnish command
cd gui && npm ci && npm run bundle        # the desktop app's installers, in target/release/bundle/
```

### Install Furnish without administrator rights

Choose the download for your operating system and CPU from the [GitHub release assets](https://github.com/furnish-self-service/furnish/releases). These options run Furnish as your account. On a managed computer, your organization must allow the app to run. A catalog step that changes the system still needs administrator approval or the [IT-managed Furnish service](admins.md#people-without-administrator-rights).

| System | User install |
|---|---|
| Windows x64 | Run the file ending in `windows-x86_64_user-setup.exe`. It installs the desktop app, CLI, DSC resources, Git and Git LFS for your account. WebView2 Runtime is needed for the desktop app. Use Installed apps to uninstall. |
| macOS, Apple silicon | Open the disk image and drag **furnish.app** into **Applications** inside your home folder (`~/Applications`). Create the folder if necessary. The release’s `arm64.app.tar.gz` contains the same app. Run the CLI as `"$HOME/Applications/furnish.app/Contents/MacOS/furnish"`. |
| Linux x86-64 | Extract the file ending in `linux-x86_64_user.tar.gz` into a folder you own, as shown below. It includes the static CLI, the desktop app, and DSC resources. Git and Git LFS must be available for collections. The desktop app also needs GTK 3, WebKitGTK 4.1 and Ayatana AppIndicator. |

From the folder containing the Linux download, substitute its release version for `VERSION`:

```sh
mkdir -p "$HOME/.local/opt"
tar -xzf furnish_VERSION_linux-x86_64_user.tar.gz -C "$HOME/.local/opt"
"$HOME/.local/opt/furnish/bin/furnish-gui"
"$HOME/.local/opt/furnish/bin/furnish" --version
```

Quit Furnish before replacing the Linux folder with a newer release. Removing that folder removes the programs. Collections and settings stay in your user data directory.

### Binary archives for packagers

Each release build produces an archive ending in `_binaries.zip` on Windows or `_binaries.tar.gz` on Linux and macOS. It contains the release CLI, desktop executable, MIT license and third-party notices. The filenames identify the operating system and architecture. Verify the archive against the same release’s `SHA256SUMS`.

When assembling your own package, provide Git and Git LFS for collections and include the DSC resources for apps using Microsoft’s resource types. Furnish looks for the `dsc` directory beside the executables, under `../lib/furnish` on Linux, or under `../Resources` in a macOS app. The user distributions above provide a working reference layout.

### Switching from alpha to stable

In builds with an **Update channel** selector, open **Settings → About** and choose **Stable** or **Alpha**. For earlier Windows MSI and macOS disk-image installations, install the download from the desired channel once to switch. If Windows reports a newer version is already installed, uninstall the alpha MSI through Installed apps first. On macOS quit Furnish before replacing the app in Applications. Keep your Furnish data folder to retain collections and settings.

For Homebrew, run `brew update` and `brew reinstall --cask furnish-self-service/tap/furnish`. For Linux, replace the alpha repository with the stable repository above. A package manager may rank `0.1.0+alpha.N` above `0.1.0`. On Debian or Ubuntu, run `sudo apt update` and `sudo apt install --allow-downgrades furnish=0.1.0`. On Fedora, refresh metadata and use `sudo dnf downgrade furnish-0.1.0` if an ordinary upgrade retains the alpha. Run `furnish --version` to confirm.

### Terminal command for the macOS download

Drag `furnish.app` from the disk image into Applications and open it there. In **Settings → About → Terminal command**, choose **Install**. Enter an administrator password if macOS asks, then run `furnish --version` in Terminal. The link at `/usr/local/bin/furnish` follows the app across updates. When uninstalling the app, you can remove its link with `sudo rm /usr/local/bin/furnish` after confirming that `readlink /usr/local/bin/furnish` still points into the app.

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
