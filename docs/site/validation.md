---
title: Validate a collection
order: 28
group: Collection authors
previous: collections
---
# Validate a collection

## Check a collection

Run `furnish check` in the collection root before publishing. It evaluates definitions for Windows, macOS and Linux without installing anything.

- **Errors** include missing files or dependencies, invalid expressions, and incompatible installer versions. The command exits with status 1.
- **Warnings** include unknown DSC resources on the machine running the check and apps without a usable removal step.
- The translation summary shows coverage by language. Missing translations use the configured fallback.

To check every change before it reaches people, run it in the repository's CI. On GitHub:

```yaml
# .github/workflows/furnish.yml
on: [push, pull_request]
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: furnish-self-service/furnish/check@main
```

On GitLab:

```yaml
# .gitlab-ci.yml
furnish-check:
  image: alpine:3.24
  script:
    - apk add --no-cache curl git
    - curl -fsSL https://furnish-self-service.github.io/furnish/alpha/furnish-linux-x86_64 -o /usr/local/bin/furnish
    - chmod +x /usr/local/bin/furnish
    - furnish check
```

The preview evaluates `context().os.family` for each system. It marks a step `(depends on host)` when its result depends on the target computer's distribution, available package managers, or configured Nix destinations. Reads of other `context()` fields also depend on the target computer.

## Check target profiles in CI

Use `furnish check . --profiles targets.yaml` to check the same collection for specific computers. Each named profile supplies the OS family (`Linux`, `macOS`, or `Windows`) and the available package managers. Linux profiles can also supply the distribution ID and the ordered `ID_LIKE` list from `os-release`. When an expression reads `context().os.architecture`, set `architecture` to the exact DSC value for the target. Use `x86_64` for 64-bit Intel or AMD, `aarch64` for ARM64 Linux or Windows, and `arm64` for Apple silicon. Profiles that do not use this fact can omit it.

List the native managers available on the target in Furnish’s priority order. On Unix the order is `pacman`, `apt`, `dnf`, `zypper`, `brew`, `nix`, then `mise` and `0install`. On Windows the order is `winget`, `mise`, `0install`. Include a collection manager's ID when its script and prerequisites are available on that target.

For example, an admin can verify which package implements a step named `Editor` and which dependency applies to developers:

```yaml
profiles:
  developer-laptop:
    family: Linux
    architecture: x86_64
    distribution: cachyos
    distributionLike: [arch]
    packageManagers: [pacman, nix]
    options: { role: developer }   # collection.yaml parameters
    appOptions:
      editor: { plugins: true }    # DSC parameters declared by this app
    expect:
      editor:
        dependencies: [developer-tools]
        steps: [Editor]
        skipped: ["Install on Windows"]
        packages:
          Editor: { manager: pacman, name: code }
```

The report lists each active dependency, the install steps in execution order, the package selection and the condition behind each skip. Active dependencies include optional dependencies users can select. An applicable package step lists a missing prerequisite when its manager is absent from the profile. A preceding dependency may supply it during installation. Omit `expect` to inspect the decisions first. Each field you include checks the entire list or package map for that app. An empty list asserts that no entries apply. A mismatch prints the expected and actual values and exits with status 1. Defaults come from the collection and app documents. Supply option values for the choices you want to exercise.

The family preview reports a dependency cycle as an error when its conditions hold together on one system. A cycle that depends on unresolved host facts produces a warning. Profiles check the graph again with their concrete facts and options. Profile assertions cover install-time selection. A condition or package expression that needs a live machine fact outside the profile, such as an environment variable, produces a diagnostic.

Profiles that use `documentsDir` can supply a target path such as `documentsDir: /home/alice/Dokumente`. The check evaluates the app with that path independently of the computer running it.

A profile assumes a fresh target with no configured Furnish NixOS or Home Manager module. A `Furnish/NixOS` step with `onlyIfSetUp: true` appears in `skipped` by default, including when `nix` is present in `packageManagers`. To test a machine with a configured destination, include `nix` in `packageManagers` and declare the ready destinations:

```yaml
profiles:
  mac-home:
    family: macOS
    packageManagers: [brew, nix]
    nixSetup: { home: true }
  nixos-workstation:
    family: Linux
    packageManagers: [nix]
    nixSetup: { system: true, home: true }
```

Use `home` for Home Manager on Linux or macOS and `system` for the NixOS system configuration on Linux. These values also supply `variables('furnish.host').nixSetup.home` and `.nixSetup.system` in target expressions. Each defaults to `false`. Steps declaring only `homeOptions` require the home destination. Steps declaring only `options` require the system destination. For packages alone or options for both destinations, the profile uses home when configured and system otherwise. Windows profiles always skip guarded steps. A step without the guard keeps its normal condition.

Use `expectKits` to check what a kit offers on a fresh installation. Each optional assertion lists app IDs in kit order. `applicable` includes members with at least one active resource on the target, `required` includes the applicable required members, and `defaultSelection` also includes preselected optional members. An empty list asserts that no members qualify. These checks evaluate collection conditions offline. Package availability in external repositories and installed state still require a live check.

The [two-profile example](https://github.com/furnish-self-service/furnish/blob/main/docs/examples/target-profiles.yaml) runs against Furnish's test collection:

```sh
furnish check tests/fixtures/collection --profiles docs/examples/target-profiles.yaml
```

In the GitHub Action, pass the file with `with: { profiles: targets.yaml }`.
