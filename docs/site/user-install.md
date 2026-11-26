---
title: Install for your account
order: 3
group: Using Furnish
previous: index
---
# Install for your account

On Windows, run the [current-user setup](https://github.com/furnish-self-service/furnish/releases/download/v0.2.0/furnish_0.2.0_windows-x86_64_user-setup.exe). It includes Git and Git LFS and needs WebView2 for the desktop. Run a newer user setup to update, or use Installed apps to uninstall.

On Apple silicon, drag the downloaded app into `~/Applications`. Create that folder in your home directory if necessary. The CLI is at `~/Applications/furnish.app/Contents/MacOS/furnish`.

## Linux current-user install

On an x86-64 machine, download the [stable Linux user archive](https://github.com/furnish-self-service/furnish/releases/download/v0.2.0/furnish_0.2.0_linux-x86_64_user.tar.gz). From the directory containing the download, extract it into your home directory:

```sh
mkdir -p -m 700 "$HOME/.local/opt"
tar -xzf furnish_0.2.0_linux-x86_64_user.tar.gz -C "$HOME/.local/opt"
```

Start the desktop with `"$HOME/.local/opt/furnish/bin/furnish-gui"`, or run the CLI directly:

```sh
"$HOME/.local/opt/furnish/bin/furnish" --version
"$HOME/.local/opt/furnish/bin/furnish" --help
```

The desktop uses GTK 3, WebKitGTK 4.1 and Ayatana AppIndicator from the host. If it reports a missing shared library on a managed machine, use the static CLI or ask IT to provide the desktop libraries. Both interfaces use Git and Git LFS for collections. Check that `git --version` and `git lfs version` work in your account.

**Update:** Starting with v0.2.0, the desktop verifies signed archives and updates automatically when idle in the tray. Select Stable or Alpha in **Settings → About**, or run `"$HOME/.local/opt/furnish/bin/furnish" self-update` to update immediately. Keep the extracted directory and its parent owned by your account and writable only by you. The updater replaces the entire extracted directory in one filesystem operation, so keep personal files outside it.

**Migrate from an older archive or recover manually:** Quit Furnish, including its tray process. Move the extracted `$HOME/.local/opt/furnish` folder aside and extract the new user archive into the same parent directory. Launch the new copy and check its version before deleting the old folder.

**Remove:** Quit Furnish and delete the extracted `furnish` folder. Your settings and collections stay in the data directory shown under **Settings → About → Data folder**.
