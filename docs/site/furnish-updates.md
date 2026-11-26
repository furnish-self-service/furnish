---
title: Update Furnish
order: 6
group: Using Furnish
previous: using
---
# Update Furnish

## New versions of furnish

Open **Settings → About** to see your version, install method and update channel. Choose **Update now** to download an available update, then **Restart to update** when ready.

| How Furnish was installed | How it updates |
|---|---|
| Windows MSI, macOS disk image, Linux current-user archive from v0.2.0 | Furnish verifies the signed download. With automatic updates enabled it restarts when idle in the tray. Use `furnish self-update` to update from the CLI. |
| Windows current-user setup | Run the newer current-user installer. |
| Homebrew, DEB or RPM | Update the package through Homebrew or the system package manager. |
| Nix | Update the Furnish input and rebuild. |

Choose **Only tell me** for notifications or turn off checks under **New versions of Furnish**. Your organization can lock this setting. When a package manager replaces the running desktop, **Restart** opens the installed build once your queue and paused work are cleared.

For the running version, source revision and build target, open **Settings → About → Build details**. **Data folder** opens the directory containing local settings and collections.

## Switch update channels

For self-updating installations, choose **Stable** or **Alpha** in About, or run `furnish settings --update-channel stable`. A channel switch may offer an older version. Other installations follow their package source.

## Switching from alpha to stable

In builds with an **Update channel** selector, open **Settings → About** and choose **Stable** or **Alpha**. For earlier Windows MSI and macOS disk-image installations, install the download from the desired channel once to switch. If Windows reports a newer version is already installed, uninstall the alpha MSI through Installed apps first. On macOS quit Furnish before replacing the app in Applications. Keep your Furnish data folder to retain collections and settings.

For Homebrew, run `brew update` and `brew reinstall --cask furnish-self-service/tap/furnish`. For DEB and RPM installs, switch to the [stable repository for your distribution](https://furnish-self-service.github.io/#install). A package manager may rank `0.2.0+alpha.N` above `0.2.0`. On Debian or Ubuntu, run `sudo apt update` and `sudo apt install --allow-downgrades furnish=0.2.0`. On Fedora, refresh metadata and use `sudo dnf downgrade furnish-0.2.0` if an ordinary upgrade retains the alpha. Run `furnish --version` to confirm.
