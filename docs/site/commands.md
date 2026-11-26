---
title: Command steps
order: 26
group: Collection authors
previous: collections
---
# Command steps

## Furnish/Command

A setting that a tool must apply itself. `test` runs a command and checks its result: the exit code (`exitCode`, default 0), its output (`output`, or `file` for a file's content, see below), or a place in its JSON output (`jsonPath`, and `value` to compare with). `set` runs only when the test fails. `remove`, if given, runs on uninstall whatever the test says, since a failing test can also mean a setting that is there but outdated. So `remove` must succeed when there is nothing to remove. `remove: keep` leaves the setting in place on uninstall (see below). Without `remove`, furnish doesn't offer to uninstall the app. Commands are lists of the program and its arguments, run without a shell. A program that isn't installed makes the app show "Needs" with its name.

On macOS and Linux, commands running as the signed-in user search the absolute directories on the inherited `PATH` first, then `~/.local/bin`, `/opt/homebrew/bin`, and `/usr/local/bin`. This also lets scripts find their interpreters and helper programs when Furnish starts from the Finder or a desktop menu. Elevated commands use their inherited `PATH`.

```yaml
- name: Vault server
  type: Furnish/Command
  properties:
    test:
      command: [bw, config, server]
      output: https://vault.acme.example
    set: [bw, config, server, https://vault.acme.example]
    remove: [bw, config, server, https://vault.bitwarden.com]
```

`jsonPath` takes the same paths as the `location` of `Furnish/Value`. A filter can select several values, from the elements of a list or the members of an object, and the test passes if one of them equals `value`. openvpn3 lists its profiles keyed by their D-Bus path, for example, and this test finds the profile named Acme:

```yaml
- name: VPN profile imported
  type: Furnish/Command
  properties:
    test:
      command: [openvpn3, configs-list, --json]
      jsonPath: "$[?@.name=='Acme'].name"
      value: Acme
    set: [openvpn3, config-import, --config, /etc/acme/acme.ovpn, --name, Acme, --persistent]
```

## Arguments and removal

An argument that starts with `~/`, or is just `~`, stands for the home folder, the same way as in paths, so one step covers every system. A `~` elsewhere in an argument, as in `--config=~/x`, is passed on as it is. For an argument that must start with `~/` itself, write `~~/`; furnish passes it on with one `~`. The steps preview shows the arguments with `~/`:

```yaml
- name: VPN connection
  type: Furnish/Command
  properties:
    test: { command: [acme-vpn, list], output: Acme }
    set: [acme-vpn, import, ~/.config/acme/acme.ovpn]
```

Some steps set up something that other apps or programs use too, which uninstalling must not take away. With `remove: keep`, uninstalling leaves the setting as it is, the app can still be uninstalled, and the uninstall preview says the step is kept. A plugin app for a coding assistant is an example: uninstalling removes the plugin and keeps the vendor's marketplace, which other plugins come from:

```yaml
- name: Acme marketplace
  type: Furnish/Command
  properties:
    test:
      command: [assistant, plugin, marketplace, list, --json]
      jsonPath: "$[?@.name=='acme-marketplace'].url"
      value: https://git.acme.example/marketplace.git
    set: [assistant, plugin, marketplace, add, https://git.acme.example/marketplace.git]
    remove: keep
- name: Acme plugin
  type: Furnish/Command
  properties:
    test: { command: [assistant, plugin, list, --json], jsonPath: "$[?@.id=='tools@acme-marketplace'].id", value: tools@acme-marketplace }
    set: [assistant, plugin, install, tools@acme-marketplace]
    remove: [assistant, plugin, uninstall, tools@acme-marketplace]
```

An app whose steps are all kept still tests as installed after uninstalling, so uninstalling it fails. Give such an app at least one step that uninstalling removes.

## Compare a command with a file

Many tools keep their own copy of a file the app writes, and the test is whether that copy still matches the file. `file` compares the output with the file's content instead of a fixed text. The path is absolute or starts with `~/`. `ignoreLines`, a regular expression, leaves out the lines it matches, in the output and in the file (or in `output`). furnish also ignores Windows line ends, spaces at the end of a line, and blank lines at the start and end, so this works the same on Windows, macOS and Linux, without a shell. The steps preview names the command and the file, and if they differ, it shows the differing lines as for a file change, or only how many if only the file's owner may read it. A file that the account checking the app's status may not read counts as matching, as with the file resources.

Set `ignoreLineOrder: true` on `test` or `present` when the tool reorders lines and their order is immaterial. With either `file` or `output`, Furnish filters `ignoreLines` first and then compares how often each remaining line occurs. The file diff shows only excess or missing occurrences. Leave this option off for order-sensitive input, including ordered rules and structured data where moving a line between blocks changes its meaning.

When a tool keeps its own copy, give the Command step a collection file with `source` or inline text with `content`. A command argument written as `'%s'` receives the path of a private temporary file. The file exists only while that command runs. A source file keeps its filename and extension. For `test.file` or `present.file`, `'%s'` compares the command's output directly with the supplied input. Only the tool's own copy determines whether the step is installed.

For this example, the profile uses an inline `<auth-user-pass>` block. Its separate bare `auth-user-pass` directive is redundant and disappears on export, so the filter below ignores that exact directive along with comments and blank lines. For profiles that rely on the bare directive, keep it in the comparison.

```yaml
- name: VPN profile in openvpn3
  type: Furnish/Command
  properties:
    source: files/acme.ovpn
    test:
      command: [openvpn3, config-dump, --config, Acme]
      file: '%s'
      ignoreLines: '^(#|$|auth-user-pass$)'
      ignoreLineOrder: true
    present: { command: [openvpn3, config-manage, --config, Acme, --show] }
    set: [openvpn3, config-import, --config, '%s', --name, Acme, --persistent]
```

Use `'%s'` as a whole argument in `test.command`, `present.command`, `set` or `remove`. If a shell is needed, pass the file as a positional argument to the script. By default, the preview shows how many input lines differ. To see the lines themselves, set `showDiff: true` alongside `source` or `content` and use `test.file: '%s'`. Both the desktop preview and CLI dry run will show lines from the command output and the supplied input after applying `ignoreLines`. Enable this for profiles whose contents can be displayed, including the currently imported copy. The temporary file stays private. The preview takes its diff from `test`. Furnish uses `present` only to decide whether the setting already exists.

When `test.file` or `present.file` names a file an earlier `Furnish/File` step will write, the preview compares the command output with the planned file contents. During installation the command checks the file on disk after the earlier step finishes.

Other tools that keep their own copy work the same way, for example `nmcli connection export`, `kubectl config view --minify` and `security find-certificate -p`.

A failing test can mean that the setting is missing or that it is there but outdated. `present`, with the same properties as `test`, tells the two apart: if `test` fails and `present` passes, the app shows an update instead of "not installed", and it can be uninstalled. Without `present`, a failing test counts as missing.

A VPN connection that NetworkManager imports from a profile is an example. When the collection brings a new profile, the connection is still there, but it was imported from the old one:

```yaml
- name: VPN profile
  type: Furnish/File
  condition: "[equals(context().os.family, 'Linux')]"
  properties:
    path: /etc/openvpn/acme-vpn.ovpn
    source: files/acme.ovpn
- name: VPN connection
  type: Furnish/Command
  condition: "[equals(context().os.family, 'Linux')]"
  properties:
    # Imported from this profile: the copy made at import time is the same.
    test: { command: [cmp, -s, /etc/openvpn/acme-vpn.ovpn, /var/lib/acme-vpn/imported.ovpn] }
    # The connection exists, from whichever profile.
    present: { command: [nmcli, connection, show, id, Acme VPN] }
    set: [sh, -c, "nmcli connection delete id 'Acme VPN'; nmcli connection import type openvpn file /etc/openvpn/acme-vpn.ovpn && nmcli connection modify acme-vpn connection.id 'Acme VPN' && mkdir -p /var/lib/acme-vpn && cp /etc/openvpn/acme-vpn.ovpn /var/lib/acme-vpn/imported.ovpn"]
    remove: [sh, -c, "nmcli connection delete id 'Acme VPN'; rm -f /var/lib/acme-vpn/imported.ovpn"]
```
