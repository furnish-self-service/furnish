---
title: Install the privileged service
order: 36
group: Administrators
previous: admins
---
# Install the privileged service

## People without administrator rights

furnish normally runs as the person using it and asks for an administrator password when a step needs one. Where people aren't administrators, the furnish service installs the apps of the preset collections for them. Turn it on in the policy, then install the service on each computer:

```json
{ "service": { "allowedGroup": "staff" } }
```

```sh
sudo furnish service install     # on Windows: furnish service install, in an administrator prompt
```

| System | The service | Who may install with it, besides root or SYSTEM |
|---|---|---|
| Windows | service `furnish`, running as SYSTEM | administrators in an elevated session, and members of `allowedGroup` |
| macOS | launchd daemon `io.github.furnish-self-service.furnish` | members of `allowedGroup`, including nested and directory groups |
| Linux | systemd unit `furnish` | members of `allowedGroup`; others need what polkit allows for `io.github.furnish.install` (by default an administrator password) |

`service install` registers the furnish binary it runs from, and refuses one that people without administrator rights could change: the file and the folders above it must belong to root (on Windows: SYSTEM, Administrators or TrustedInstaller) and must not be writable by others. A copy in `~/.cargo/bin`, an AppImage in a home folder, the Homebrew cask or another per-user install doesn't qualify; the furnish packages install to folders that do.

The service runs git from the system: `/usr/bin/git` on Linux and macOS (on NixOS, git from the service's `PATH`). On macOS that means Apple's Command Line Tools, which nobody is asked to install on the service's behalf: install them along with furnish, for example with `xcode-select --install` or from your software distribution. The launchd daemon's `PATH` has only system folders, not `/usr/local/bin` or `/opt/homebrew/bin`, since those belong to whoever installed Homebrew. For Homebrew itself, see [macOS: the service and Homebrew](#macos-the-service-and-homebrew).

Everyone can ask the service to set up and update the preset collections. When that fails, only people who may install see why; others get a short message, and the details go to the service's log: the journal of the `furnish` unit on Linux, `/var/log/furnish.log` on macOS, `C:\ProgramData\furnish\service.log` on Windows. The macOS and Windows logs are readable by administrators only.

The desktop app and CLI also record failed collection updates in `furnish.log` in the user's data folder. On macOS the default path is `~/Library/Application Support/furnish/furnish.log`, on Linux `~/.local/share/furnish/furnish.log`, and on Windows `%APPDATA%\furnish\furnish.log`. Each entry includes the UTC time, collection name and error. The file stays on the computer even when organization reporting is off. Furnish keeps up to 1 MiB in the current log and one previous file, `furnish.log.1`.

On Windows, the service manager restarts the service when it crashes or stops with an error. The service refuses to start when `C:\ProgramData\furnish` exists but wasn't created by it: when it is a link, or when anyone besides SYSTEM and Administrators can change it. Remove the folder and start the service again.

The service finds DSC resources only in folders that people without administrator rights can't change. A folder counts if it and every folder above it belong to root (on Windows: SYSTEM, Administrators or TrustedInstaller) and nobody else can write to it, the same rule as for the furnish binary. The service looks in the folder of the DSC resources furnish includes, which is inside furnish's installation, in the folders on its `PATH` and where DSC is installed, and drops the others. A resource in `/opt/homebrew/bin` or in a folder of a user's own `PATH` would otherwise run with full rights. The service also runs programs only from these folders, except `brew`, which it runs as Homebrew's owner.

For an install, the service reads the app from its own copy of a preset collection and checks the requested option values against the app's declarations and the policy. [Personal apps](app-steps.md#installing-for-one-person) run in the user's account. If a personal app needs a system rebuild, the service verifies the requested step against its preset collection before applying a NixOS declaration or adding an ordinary Nix package to the configuration. The app's file and command steps still run as the person installing it.

## macOS: the service and Homebrew

Homebrew refuses to run as root. So for `Furnish/Package`, the service runs `brew` as the account that owns Homebrew (`/opt/homebrew`, or `/usr/local/Homebrew` on Intel Macs). It never runs `brew` as root and needs no password for this.

Whoever can run programs as that account could take over the service's installs. So the service uses Homebrew only if:

- the account that owns it is an administrator (a member of `admin`),
- nobody else can change it: Homebrew's folder and the folders above it belong to that account or root, others can't write to them, and only the groups `admin` and `wheel` may.

Otherwise apps that need Homebrew show "Needs Homebrew that only administrators can change", with the reason. This happens, for example, when a person without administrator rights installed Homebrew and owns it. An administrator then has to give the folder to an administrator account, for example `sudo chown -R admin-account /opt/homebrew`, or remove Homebrew and install it again as an administrator.

Casks that run an installer package (`.pkg`), and the uninstall steps of many casks, call `sudo` themselves. The service has nobody to type a password, so allow it once:

```sh
sudo furnish service install --homebrew-sudo
```

This checks the same conditions and adds the file `/etc/sudoers.d/furnish-homebrew` (owner root, group wheel, mode 0440, checked with `visudo`):

```
admin-account ALL = (root) NOPASSWD: ALL
```

The rule names the account that owns Homebrew. It allows every command, because Homebrew runs its package installer, file removals and the casks' own scripts through `sudo`, and each of them already gives full control of the computer. A shorter list would protect nothing and would break casks. The rule gives nothing to people without administrator rights, since they can't run programs as that account. It does mean that the account can use `sudo` without its password. If the account belongs to a person, a program that person starts can then become root without asking. To avoid that, let a separate administrator account that nobody logs in to own Homebrew, and use `brew` through it (`sudo -u that-account -H /opt/homebrew/bin/brew …`).

`furnish service install` without `--homebrew-sudo` removes the file again, and so does `furnish service uninstall`. The service also removes it whenever it finds that Homebrew no longer meets the conditions. Without the file, the service still installs formulae and casks that need no `sudo`. Run `service install --homebrew-sudo` again after giving Homebrew to another account.
