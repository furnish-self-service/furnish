---
title: Files and directories
order: 25
group: Collection authors
previous: collections
---
# Files and directories

## Configuration

Six resources set up configuration: folders, files, parts of files and settings that a tool applies itself. They work the same on Windows, macOS and Linux, so one step covers every system. Each of them can remove what it set up, so an app made of them needs no removal steps.

Paths are absolute or start with `~/`, which resolves to the home folder of the account running the step. For the personal files in the examples below, set [`perUser: true`](app-steps.md#installing-for-one-person) on the app. Missing parent folders are created. A path that is a symbolic link is written through to its target. If the target can't be written, as with a file that Home Manager links from the Nix store, the step fails and names the file.

The resources that write files have these properties in common:

| Property | Meaning |
|---|---|
| `_exist` | `false` removes the file, or the resource's part of it |
| `requiresClosed` | Names of application executables that must be closed before this file changes |
| `backup` | `true` keeps a copy of the file before each change, named `<file>.<date and time in UTC>~` |
| `validate` | A command that checks the new file before it replaces the old one, with `'%s'` for its path, for example `[ssh, -G, -F, '%s', git.acme.example]`. If it exits with another code than 0, the step fails and the file stays as it was |
| `mode` | The file's mode on macOS and Linux, for example `'640'` |
| `private` | `true`: only the user may open the file. On Windows, its access list grants only the user access and inherits nothing; on macOS and Linux, its mode is 600 |

Some desktop apps keep their settings in memory and save them again on exit. Use `requiresClosed` on a `Furnish/File`, `Furnish/Line`, `Furnish/Block`, or `Furnish/Value` step that edits one of these files. For example, a Linux collection for an editor whose executable is named `notes-desktop` could include this step:

```yaml
- name: Disable the editor’s update reminders
  type: Furnish/Value
  properties:
    path: ~/.config/notes-desktop/settings.json
    format: json
    location: $.showUpdateReminders
    value: false
    requiresClosed: [notes-desktop]
```

Use executable names, such as `notes-desktop`, for processes visible to the account applying the change. The Furnish service can see applications in other users’ sessions. The desktop checks the pending operation before starting any steps and keeps it in Activity until the user closes the named applications and chooses **Recheck and continue**. The file resource checks again immediately before writing, including when invoked from the CLI. For an app in the system tray, use its Quit action. A step whose settings already match can finish while the app stays open.

On Linux, some packages start a shared `electron` or `electron<major>` executable. When its launch argument is an absolute path ending in `<app>/app.asar` or `<app>/resources/app.asar`, use the app directory name in `requiresClosed`. For example, `requiresClosed: [bitwarden]` covers both `/usr/lib/bitwarden/app.asar` on Arch and `…/opt/Bitwarden/resources/app.asar` on NixOS. Furnish ignores ASCII letter case for this directory name. Direct Linux executable names remain case-sensitive. Detection requires a readable launch argument and supports the first positional path or `--app=<path>`. An unrecognized standalone switch before that path leaves the package identity unknown. When Electron rewrites the visible command line as one title, Furnish can recognize package paths without spaces.

`mode` and `private` are part of the desired state: a file that someone made readable for others shows as an available update, and installing again restores them. A step without them leaves the mode as it is, so for a file that several `Furnish/Value`, `Furnish/Line` or `Furnish/Block` steps edit, `private: true` on one of them is enough. Steps that give the same file different modes would undo each other, so furnish refuses such an app when it loads it. Uninstalling never changes the mode, and these three resources never delete the file: it may hold the user's own settings.

A file that the account checking the app's status may not read, such as a root-only file that the furnish service wrote, counts as set up if it exists: furnish can't compare its contents as that account. The steps preview says so.

A file is replaced in one step, so programs never read half of it. It keeps its mode, owner and encoding, including a byte order mark and Windows line ends.

To tell whether an app is installed, each resource computes the change it would make to the file and reports the desired state when there is none. Installing makes that change. A setting someone changed by hand shows as an available update, and installing again sets it back.

A value that starts with `[` is a DSC expression. Write `[[` for a pattern or text that starts with `[`.

### Furnish/Directory

A folder. `_exist: false` removes it only when it is empty. `mode` and `private` work as for files, with mode 700 for a private folder.

| Property | Meaning |
|---|---|
| `path` | The folder |
| `source` | Optional folder in the collection to copy recursively |

```yaml
- name: VPN folder
  type: Furnish/Directory
  properties:
    path: ~/.config/acme-vpn
    private: true
```

To keep an offline copy of documentation current, give the folder a collection-relative source.

```yaml
- name: offline docs
  type: Furnish/Directory
  properties:
    path: ~/Documents/Team docs
    source: docs/offline/
```

Furnish manages the regular files at the paths supplied by the source. An update restores their contents and removes files dropped from the collection. Uninstall removes the files Furnish copied and any empty folders it created. A conflicting local file is reported so you can move it before installing. Other local files stay, including files that already matched when the copy was installed. The preview counts added, changed and removed files. Source and destination must be separate trees. Links inside the copied tree are refused.

### Furnish/File

A whole file, with the text in `content` or a copy of `source`, a file in the collection. furnish checks `source` like the `file` of `Furnish/Installer`: it must be in the collection, and furnish downloads it when the app is installed. Without either, the file only has to exist.

```yaml
- name: VPN profile
  type: Furnish/File
  properties:
    path: ~/.config/acme-vpn/acme.ovpn
    source: files/acme.ovpn
    private: true
```

### Furnish/Block

Lines between two marker comments that name the app, for example `# BEGIN furnish acme/ssh` and `# END furnish acme/ssh`. If the file has the markers, the step replaces what is between them. Otherwise it adds the block at `insert`: `top`, `bottom` (the default), `before: <pattern>` or `after: <pattern>`. The position counts only for that first time, so the block stays where it is, also when someone adds lines above it. Two apps get two blocks. `_exist: false` removes the block with its markers.

| Property | Meaning |
|---|---|
| `path` | The file |
| `content` | The lines between the markers |
| `comment` | What starts a comment in the file, default `#` |
| `insert` | Where the block goes the first time |

```yaml
- name: Git server host keys
  type: Furnish/Block
  properties:
    path: ~/.ssh/known_hosts
    content: |
      git.acme.example ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGq7…
```

### Furnish/Line

One line, for files where marker comments aren't allowed. Patterns are regular expressions ([syntax](https://docs.rs/regex/latest/regex/#syntax)) and match anywhere in a line unless they use `^` and `$`.

| Property | Meaning |
|---|---|
| `path` | The file |
| `line` | The line |
| `match` | A pattern for the line to replace |
| `before` | The line must come before the first line that matches this pattern |
| `after` | The line must come after the last line that matches this pattern |
| `accept` | A pattern for a line that counts as the line, where the line may be. Removal leaves such a line |

Without `match`, a line equal to `line` must exist. The step adds it only if it's missing, so it never adds a second one, and removal deletes every equal line. With `match`, a line that matches must exist and equal `line`: the step replaces the first match, deletes further matches, and adds `line` if nothing matches. `line` must match `match`, or the next install wouldn't find it and would add it again; furnish refuses such an app when it loads it.

With `accept`, a line that matches it counts as present, and the step adds `line` only if no such line is in place. This suits a line that a tool may have written in another form, such as an `Include` that names other files too. `line` must match `accept`, and an app can't use both `match` and `accept`.

`before` and `after` are part of the desired state. A line that someone moved past one of these lines shows as an available update, and installing again moves it back. A missing line goes right after the `after` match, else right before the `before` match, else at the end. ssh reads an `Include` only before the first `Host` or `Match` line:

```yaml
- name: SSH configuration
  type: Furnish/Line
  properties:
    path: ~/.ssh/config
    line: Include config.d/acme.conf
    before: '^\s*(Host|Match)\s'
```

### Furnish/Value

One value in a JSON, JSONC (JSON with comments), YAML or TOML file. The rest of the file keeps its keys, their order and its comments. A file that doesn't parse stops the step with an error, and furnish leaves it as it is.

| Property | Meaning |
|---|---|
| `path` | The file |
| `format` | `json`, `jsonc`, `yaml` or `toml`; by default from the file's extension |
| `location` | Where the value is: a JSONPath that names one place, such as `$.hosts['git.acme.example'].git_protocol`, or a JSON Pointer such as `/hosts/git.acme.example/git_protocol` |
| `value` | The value |
| `merge` | `replace` (the default), `deep` or `member` |
| `removeIf` | Instead of `value` and `merge`: a list of values. Installing leaves the value at the location as it is, and uninstalling removes it only if it is one of these |

Missing objects on the way to the location are created. Only the value at the location counts for the app's state. `[?@.name=='acme-prod']` selects the element of a list whose `name` is `acme-prod`: the step replaces that element, or adds it at the end if there is none, and `value` must have that `name`. In an object, as some tools write their entries keyed by an ID, the filter selects the member whose value has that `name`: the step replaces that member under its key, and fails if there is none, since a new member needs a key and a filter doesn't give one. To add a member, name its key in `location`. `merge: deep` sets the keys of `value` and keeps the other keys at the location. `merge: member` treats the location as a list that must contain `value` and keeps its other elements. `_exist: false` removes the value at the location, the keys of a deep merge, or the element, the whole element also with `merge: deep`.

A kubeconfig merge takes one step each for the cluster, the user and the context, and leaves the other entries as they are. kubeconfig holds credentials, and tools such as helm warn about one that others can read, so the first step has `private: true`. `merge: deep` keeps what others add to an entry: kubectl writes defaults into the user when it saves the file, such as `env: null`, and people set a context's `namespace`. With `replace`, either would show as a change, and installing would undo it.

Installing leaves `current-context` alone, so the user's choice stays. If it names the app's context when the app is uninstalled, kubectl fails on every command that doesn't name a context, since that context is gone. A step with `removeIf` removes it in that case and keeps any other value. It has nothing to do when the app is installed, so it never makes the app show as not installed or as having an update, and an uninstall that keeps the value says why in the steps preview. In [removal steps](app-steps.md#removal-steps), such a step needs `_exist: false`.

The whole app:

```yaml
metadata:
  furnish:
    name: Acme Kubernetes
    description: Access to the Acme production cluster with kubectl.
    categories: [Development]
resources:
  - name: Acme cluster
    type: Furnish/Value
    properties:
      path: ~/.kube/config
      format: yaml
      location: "$.clusters[?@.name=='acme-prod']"
      private: true
      merge: deep
      value:
        name: acme-prod
        cluster:
          server: https://k8s.acme.example
  - name: Acme user
    type: Furnish/Value
    properties:
      path: ~/.kube/config
      format: yaml
      location: "$.users[?@.name=='acme-prod']"
      merge: deep
      value:
        name: acme-prod
        user:
          exec: { apiVersion: client.authentication.k8s.io/v1beta1, command: kubelogin, args: [get-token] }
  - name: Acme context
    type: Furnish/Value
    properties:
      path: ~/.kube/config
      format: yaml
      location: "$.contexts[?@.name=='acme-prod']"
      merge: deep
      value:
        name: acme-prod
        context: { cluster: acme-prod, user: acme-prod }
  - name: Current context
    type: Furnish/Value
    properties:
      path: ~/.kube/config
      format: yaml
      location: $.current-context
      removeIf: [acme-prod]
```
