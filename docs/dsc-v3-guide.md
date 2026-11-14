# DSC v3 guide for furnish

Reference for two audiences: **admins** writing `apps/<app>.dsc.yaml` files, and **furnish
developers** writing DSC resources such as `Furnish/Package`. Covers Microsoft DSC v3.3.0, the
version furnish embeds as `dsc-lib`.

How to read this guide:

- Unless marked otherwise, every statement was checked against the DSC source at tag `v3.3.0`.
  Paths such as `lib/dsc-lib/src/...` point into the
  [PowerShell/DSC](https://github.com/PowerShell/DSC/tree/v3.3.0) repository.
- **Verified by experiment** means the claim was reproduced with furnish's own binary
  (`target/fast/furnish`), a scratch `FURNISH_DATA_DIR` and dummy command resources.
- Where the official docs and the v3.3.0 source disagree, this guide follows the source and says
  so.
- Section 6 is the only section that contains recommendations. Everything before it is fact.

---

## 1. Configuration documents

Sources: `lib/dsc-lib/src/configure/config_doc.rs`, `docs/concepts/configuration-documents/overview.md`,
`docs/reference/schemas/config/functions/*.md`.

### 1.1 Structure

```yaml
$schema: https://aka.ms/dsc/schemas/v3/bundled/config/document.json   # furnish injects this if missing
directives:                 # optional, added in v3.2
  securityContext: current  # current | elevated | restricted
metadata:
  furnish:                  # free-form; DSC ignores keys other than Microsoft.DSC
    description: Command-line benchmarking tool
    categories: [Development]
parameters:
  channel: {type: string, defaultValue: stable, allowedValues: [stable, beta]}
variables:
  pkg: "[concat('hyperfine-', parameters('channel'))]"
resources:
  - name: hyperfine          # must be unique per type within the document
    type: Furnish/Package
    condition: "[not(equals(context().os.family, 'Windows'))]"
    dependsOn: ["[resourceId('Furnish/Package', 'other')]"]
    properties:
      name: hyperfine
```

- Top-level keys are `$schema`, `contentVersion`, `directives`, `executionInformation`, `functions`,
  `metadata`, `outputs`, `parameters`, `resources` and `variables`. The document uses
  `deny_unknown_fields`, so a typo in a top-level key is an error.
- Resource instance keys are `name`, `type`, `properties`, `condition`, `dependsOn`, `directives`,
  `metadata`, `requireVersion` (alias `apiVersion`), `copy` (deprecated) and `resources`. These
  also use `deny_unknown_fields`.
- **`$schema`.** `dsc-lib`'s `Configurator` does not validate the URI. Even a bogus URI is
  accepted (verified by experiment). Editors and the `dsc` CLI do use it. Use the major-version
  URI `.../schemas/v3/...`: the recognized-version list generated at build time in v3.3.0 ends at
  `v3.2.3` and contains no `v3.3` entry.
- **Parameters.** The types are `string`, `secureString`, `int`, `bool`, `object`, `secureObject`
  and `array`, with the constraints `allowedValues`, `minValue`, `maxValue`, `minLength`,
  `maxLength`, `defaultValue`, `description` and `metadata`. furnish currently calls
  `set_context(None)`, so only defaults apply.
- **Secure values.** A `secureString` reaches the resource as the object
  `{"secureString": "<value>"}`, not as a plain string. Verified by experiment: a property with
  `type: string` failed schema validation with
  `{"secureString":"hunter2"} is not of type "string"`. DSC redacts secure values in results and
  skips them in the synthetic test.
- **`outputs`** (v3.2+) evaluates expressions after the run. **`functions`** (v3.2+) defines user
  functions.

### 1.2 `condition`

- An expression string. The instance runs only if it evaluates to exactly `true`; any other result
  skips it. Implemented as `skip_resource` in `configure/mod.rs`.
- **A skipped instance produces no result entry at all** in `get`, `test` or `set`. It isn't
  reported as "in desired state" or "skipped" either.
- Resource **manifests** can also carry `condition` (v3.2+). If the condition is false, discovery
  drops the manifest (`discovery/command_discovery.rs: evaluate_condition`).

### 1.3 `dependsOn` and ordering

- The syntax is `"[resourceId('<Type>', '<name>')]"`. The referenced instance must exist and have
  that exact type.
- Invocation order is **document order**, except that a dependency is pulled in before the first
  instance that depends on it (`configure/depends_on.rs`). DSC detects cycles.
- `set` aborts at the first resource error and returns `Err`, so later instances don't run
  (`configure/mod.rs: invoke_set`).
- DSC never reverses the order. Deleting things therefore still runs dependencies first. See 6.1.

### 1.4 Expression functions most useful to furnish

Expressions are strings wrapped in `[...]`. Escape a literal leading `[` as `[[`. String literals
use single quotes. All examples below were verified by experiment.

| Need | Expression |
|---|---|
| OS family | `[equals(context().os.family, 'Windows')]`. Values are `Windows`, `macOS` and `Linux`. |
| Other OS facts | `context().os.version`, `.edition`, `.codename`, `.architecture`, `.bitness`. There is no Linux distro name: every non-Windows, non-macOS system reports `Linux` (`lib/dsc-lib-osinfo/src/lib.rs`). |
| Elevation | `[equals(context().security, 'elevated')]`. Values are `elevated` and `restricted`. |
| A tool is present | `[not(equals(tryWhich('apt-get'), null()))]`. `tryWhich` searches `PATH` and returns `null` if the tool is missing. |
| Environment variable | `[envvar('HOME')]`. **Errors** if the variable is unset; there is no "try" variant. |
| Logic | `and()`, `or()`, `not()`, `if(cond, a, b)`, `coalesce()`, `equals()`, `less()`, `greater()`, ... |
| Strings and paths | `concat()`, `format()` (stable in v3.3), `path(a, b)`, `toLower()`, `startsWith()`, `contains()` |
| Values from other instances | `reference(resourceId(...))` (the actual state of an earlier instance), `stateChanged(resourceId(...))` (v3.3, during `set`) |
| Parameters and variables | `parameters('x')`, `variables('x')` |

The full list is in `docs/reference/schemas/config/functions/` and `lib/dsc-lib/src/functions/`.

Per-distro package names example: the condition selects by package manager because there is no
distro name in `context()`.

```yaml
- name: fd (apt)
  type: Furnish/Package
  condition: "[not(equals(tryWhich('apt-get'), null()))]"
  properties: {name: fd-find}
- name: fd
  type: Furnish/Package
  condition: "[and(equals(context().os.family, 'Linux'), equals(tryWhich('apt-get'), null()))]"
  properties: {name: fd}
```

### 1.5 Groups, adapters and built-in Microsoft resources

- `Microsoft.DSC/Group`, `Microsoft.DSC/Assertion` and `Microsoft.DSC/Include` are
  **command resources whose executable is `dsc`** (`dsc/group.dsc.resource.json` and siblings).
  The PowerShell and WMI adapters also need the `dsc` CLI or `pwsh`.
- furnish sets `DSC_RESOURCE_PATH` to its bundled resource directory. With that variable set, DSC
  searches only those directories for manifests and does not append `PATH`
  (`command_discovery.rs: get_resource_paths`).
- Consequence: **app files can use only resources that furnish bundles.** That rules out groups,
  assertions, includes, adapters and `Microsoft.Windows/Registry` unless furnish ships them.

### 1.6 Directives and security context

- Document-level: `directives.securityContext: elevated | restricted | current`. If the process
  doesn't match, DSC fails before running anything. Verified by experiment: running unelevated
  returned `Security context: Elevated security context required`.
- Instance-level: `directives.securityContext` and `directives.requireAdapter`.
- The older form `metadata.Microsoft.DSC.securityContext` still works but logs a deprecation
  warning (`configure/mod.rs: check_security_context`).

---

## 2. Canonical properties

Sources: `docs/reference/schemas/resource/properties/*.md`, `dscresources/dscresource.rs`,
`configure/mod.rs`, CHANGELOG v3.2.0.

| Property | Direction | Meaning |
|---|---|---|
| `_exist` | read/write, default `true` | Whether the instance should exist. `get` **must** return `_exist: false` for a missing instance and may omit it when the instance is present. The synthetic test fills in `true` for whichever side omits it (`get_well_known_properties`). |
| `_inDesiredState` | read-only | Returned by a resource that implements `test`. If present and boolean, DSC uses it in place of its own diff (`get_desired_state`). A non-boolean value is an error. |
| `_purge` | write-only | Remove list entries that aren't declared. Semantics only: DSC does no processing. Never return it. |
| `_name` / `_securityContext` | output of `export` only | Hoisted into the exported instance's `name` and its security context (v3.2). |
| `_restartRequired` | output | An array of `{system: ...}`, `{service: ...}` or `{process: {name, id}}`. DSC aggregates it into `executionInformation.restartRequired` (v3.2). |
| `_metadata` | in and out | Input: the instance's `metadata` (minus `Microsoft.DSC`) is passed as `_metadata` **only if the schema accepts it**; otherwise it is silently dropped. Output: keys are copied into the result metadata. `_metadata._refreshEnv: true` is described below. |
| `_ensure` | removed | Replaced by `_exist` since 2023-10 schemas. |

How DSC acts on `_exist: false` during `set` (`configure/mod.rs: invoke_set`):

1. The manifest has `set.handlesExist: true`: DSC calls `set`, and the resource must remove the
   instance.
2. Otherwise, if the manifest defines `delete`: DSC runs `get`, then `delete`, then `get` again.
   There is **no** test first, so `delete` must be idempotent.
3. Otherwise DSC returns the error
   `Resource '<type>' does not support delete and does not handle _exist as false`
   (verified by experiment).

How `_refreshEnv` works:

- It is recognized only in the output of `set`, and only acted on on Windows.
- When a resource returns `"_metadata": {"_refreshEnv": true}`, DSC rebuilds the process
  environment from the registry: HKLM, overlaid with HKCU, with the user `PATH` prepended to the
  system `PATH`. It does this with `set_var`.
- The value is read from the set result's after-state. That after-state comes from the resource's
  own stdout only when `set.return` is set; otherwise it comes from a fresh `get`. The schema must
  also allow `_metadata`.
- It exists so that a later instance can find a freshly installed tool.

---

## 3. Resource authoring (command resources)

Sources: `dscresources/resource_manifest.rs`, `dscresources/command_resource.rs`,
`discovery/command_discovery.rs`, `docs/reference/schemas/resource/manifest/*.md`, and the built-in
resources under `resources/` (`apt`, `brew`, `filecontent`, `registry`, `windows_service`).

### 3.1 Manifest

- Discovery matches files named `*.dsc.resource.{json,yaml,yml}`, and manifest lists named
  `*.dsc.manifests.{json,yaml,yml}` (v3.2+).
- The manifest uses `deny_unknown_fields`.

```json
{
  "$schema": "https://aka.ms/dsc/schemas/v3/bundled/resource/manifest.json",
  "type": "Furnish/Package",
  "version": "0.2.0",
  "description": "...",
  "tags": ["linux", "macos"],
  "condition": "[not(equals(context().os.family, 'Windows'))]",
  "get":    {"executable": "x.sh", "args": ["get"], "input": "env"},
  "set":    {"executable": "x.sh", "args": ["set", {"whatIfArg": "--what-if"}], "input": "env",
             "handlesExist": true, "implementsPretest": true, "return": "state"},
  "test":   {"executable": "x.sh", "args": ["test"], "input": "env", "return": "state"},
  "delete": {"executable": "x.sh", "args": ["delete"], "input": "env"},
  "export": {"executable": "x.sh", "args": ["export"]},
  "exitCodes": {"0": "Success", "10": "No supported package manager"},
  "schema": {"embedded": { "...": "JSON Schema" }}
}
```

**Identity and discovery fields**

- `type` must match `^\w+(\.\w+)*/\w+$`, that is `Owner[.Area...]/Name`
  (`types/fully_qualified_type_name.rs`). Microsoft uses forms like
  `Microsoft.Windows/Registry` and `DSC.PackageManagement/Apt`.
- `version` must be semver. A date-style version only triggers a warning.
- `kind` is one of `resource`, `adapter`, `group`, `importer` or `exporter`.
- `deprecationMessage` (v3.2+) marks the resource as deprecated.

**Operations**

- `get` (required in practice), `set`, `test`, `delete`, `export`, `whatIf`, `validate`,
  `resolve`, `schema`.
- Each operation also accepts `requireSecurityContext: elevated | restricted | current`. DSC
  checks it before invoking the operation.

**Argument kinds** (inside `args`)

| Kind | Effect | Valid in |
|---|---|---|
| plain string | passed as is | all operations |
| `{"jsonInputArg": "--input", "mandatory": true}` | adds the flag followed by the JSON | all operations |
| `{"resourceTypeArg": ...}` | passes the resource type | all operations |
| `{"resourceVersionArg": ...}` | passes the resource version | all operations |
| `{"resourcePathArg": ...}` | passes the manifest path | all operations |
| `{"whatIfArg": "-w"}` | added only in what-if mode | `set` and `delete` only (v3.2+) |

**Input**

- `input` is `env` or `stdin`. If you omit it and use no `jsonInputArg`, the resource gets no
  input at all.
- `get` with an empty filter receives no input.

**`set` fields**

- `implementsPretest: true`: DSC skips its own test before `set`.
- `handlesExist: true`: `set` handles `_exist: false` itself (section 2).
- `return: state | stateAndDiff` controls what `set` must print to stdout (section 3.4).
- `whatIfReturns` (v3.2+) lets what-if mode use a different output shape.

**`export` fields**

- `supportsFiltering: false` (v3.3, experimental): the engine filters exported instances itself.

**Exit codes**

- `exitCodes` maps codes to messages. Discussed in section 3.5.

**Schema**

- `schema` is `{"embedded": {...}}` or `{"command": {"executable", "args"}}`.
- **Remote `$ref` does not resolve**: dsc-lib builds `jsonschema` without `resolve-http`.
  Verified by experiment: `$ref: .../properties/exist.json` failed with `resolve-http feature or a
  custom resolver is required`. Inline the canonical properties (as `apt` does) or bundle them
  under `$defs` with an `$id` (as `PSScript` does).
- DSC validates **input and output** against the schema: the desired state before every
  operation, and the stdout of `get`, `test` and `set` with `return`.
- `readOnly` is documentation only; DSC does not enforce it.
- `writeOnly: true` properties are skipped by the synthetic test (v3.3 source, not in the docs).
- A property `default` in the schema is used when the actual state omits that property.

### 3.2 How the engine invokes a command

From `command_resource.rs: invoke_command` and `run_process_async`; `util.rs: canonicalize_which`.

- **Executable resolution.** DSC first looks the executable up on `PATH`. Only if that fails does
  it look in the manifest's directory. A same-named binary earlier on `PATH` therefore wins.
- **Environment.** The working directory is the manifest's directory. The child inherits the full
  parent environment; with `input: env`, the properties are **added** to it.
- **stdin.** stdin is `/dev/null` unless `input: stdin`, so a resource can't read interactive
  answers from stdin.
- **No timeout.** A hanging child blocks the whole operation.
- **Env conversion** (`json_to_hashmap`):
  - Strings are passed as is; booleans and numbers become `"true"`, `"false"` or `"42"`.
  - Arrays of strings or numbers are joined with `,`.
  - `null` is skipped, and nested objects are an **error**. That includes secure values, which are
    objects (section 1.1).
  - A property that is absent from the desired state is simply not set, so the child **inherits a
    same-named variable from the parent**. Verified by experiment: with `fail` unset in the
    document but exported in the parent shell, the resource saw it. Windows variable names are
    case-insensitive, so the risk of a clash is higher there.
  - Variable names equal the property names: `$name`, `$_exist`.

### 3.3 Operation semantics as implemented

**`get`**

- stdout is parsed as **one** JSON value (the whole of stdout) and validated against the schema.
- It must have no side effects, because DSC calls it many times. For a `set` with neither
  `implementsPretest` nor `return`, DSC runs `get` 3 times. Verified by experiment, the sequence
  is: test → get, pre-state get, `set`, post-state get.

**`test` with `test` defined**

- `return: state`: DSC uses `_inDesiredState` from stdout if present, and otherwise the diff
  between the desired and actual state.
- `return: stateAndDiff`: two JSON lines on stdout, the state and then an array of the names of
  properties out of desired state.
- **`return` omitted: DSC runs the test command, discards its stdout, then runs `get` and diffs.**
  The docs (`stdout/test.md`) claim the default is `state`; the v3.3.0 source does not do that.
  Always set `test.return`.
- The docs say `_inDesiredState` is mandatory when `test` is defined. In the source it is
  optional.

**`test` without `test` defined: synthetic test** (`invoke_synthetic_test`, `get_diff_with_schema`)

- DSC calls `get`, then compares every property of the desired state with the actual state.
- `_exist` defaults to `true` on both sides.
- Strings are compared case-sensitively.
- Arrays compare as sets of equal length.
- Objects are compared recursively, and the actual state may be a superset of the desired state.
- `$schema`, secure values and `writeOnly` properties are skipped.
- A property that is missing from the actual state counts as different, unless the schema
  `default` equals the desired value.

**`set`**

- Before the resource's `set`, DSC runs a test (unless `implementsPretest` is set) and **returns
  early if the instance is already in desired state**. It then always runs a pre-state `get`.
- After the command, the result depends on `return`:

  | `return` | What DSC does |
  |---|---|
  | omitted | Ignores stdout and runs `get` again. |
  | `state` | Expects one JSON line describing the after-state. |
  | `stateAndDiff` | Expects two JSON lines: the after-state and the array of changed property names. |

- `invoke_set(skip_test = true)` skips the test for **every** resource. Use it only when each
  resource's `set` is idempotent.

**What-if** (`ExecutionKind::WhatIf`, `context.execution_type`)

| Resource provides | What DSC does |
|---|---|
| `whatIfArg` in `set.args` | Calls `set` with that flag (native what-if). |
| a separate `whatIf` method | Calls it, and logs a warning that it prefers `whatIfArg`. |
| neither | Converts the test result into a set result (synthetic what-if). |

For `delete`, if there is no `whatIfArg`, DSC runs a synthetic test.

**`delete`**

- Takes the filter as input; stdout is ignored except in what-if mode.

**`export`**

- stdout is JSON Lines, one instance per line.
- If `export` is undefined, DSC calls `get` with the filter as input ("synthetic export", v3.2+).

### 3.4 stdout and stderr

- **stdout must contain only the JSON payload.** Anything else breaks parsing (`get`, `test`,
  `set` with `return`) or schema validation. Send tool output to stderr. With `set` and no
  `return`, DSC ignores stdout, which is the only reason `apt.dsc.resource.sh` gets away with
  printing `apt install` chatter there.
- **stderr.** DSC reads stderr line by line (`log_stderr_line`):

  | stderr line | Logged at | Kept for the error text |
  |---|---|---|
  | `{"error":"msg"}` | error | no |
  | `{"warn":"msg"}` | warn | no |
  | `{"info":"msg"}` | info | no |
  | `{"debug":"msg"}` | debug | no |
  | `{"trace":"msg"}` | trace | no |
  | tracing-crate JSON `{"timestamp":"...","level":"INFO","fields":{"message":"..."},"target":...,"line_number":...}` | its own `level` | no |
  | any other JSON | trace | yes |
  | plain text | trace | yes |

  The table was verified by experiment: the `{"info":...}` line was absent from the error text and
  the plain line was present.
- **Docs mismatch.** `docs/concepts/resources/anatomy.md` describes `{"message":..., "level":
  "error|warning|information"}`. v3.3.0 does **not** parse that shape; it is treated as plain
  text.
- All of these messages go to the `tracing` crate. With the `dsc` CLI they appear as CLI trace
  output. An embedder (furnish) sees them **only if it installs a `tracing` subscriber**.
- `ConfigurationTestResult.messages` and `hadErrors` are never populated in v3.3.0; a grep for
  pushes into them finds none. Failures surface as `Err`.

### 3.5 Exit codes

- 0 means success; any non-zero code is a failure (`run_process_async`).
- If the manifest defines `exitCodes` (anything other than the default `{0,1}`) **and** the code
  is in that map, the error becomes `CommandExitFromManifest`, whose message is **only the manifest
  description. stderr is dropped from the error.**
- If the code is not in the map, the error is `Command(exe, code, filtered_stderr)`, which includes
  the plain stderr lines.
- Verified by experiment: exit 1 with map `{"1":"Invalid arguments"}` produced
  `[exit code 1] manifest description: Invalid arguments`. Exit 7, which was unmapped, produced
  `[exit code 7] plain stderr line / failing on purpose: 7`.
- Implication: a script running under `set -e` passes the exit code of the failing tool straight
  through. If that code happens to be mapped, the tool's message is lost.

### 3.6 Discovery settings

- `DSC_RESTRICTED_PATH`: search only these directories, **and replace `PATH`** with them.
- `DSC_RESOURCE_PATH`: search only these directories for manifests. `PATH` itself is unchanged,
  apart from adding the directory of the running executable.
- Neither variable set: DSC searches the `resourcePath.directories` from `dsc.settings.json`
  (beside the executable, or the policy file under `/etc/dsc` or `%ProgramData%`), then `PATH`.

### 3.7 Cross-platform patterns seen in the DSC repo

- **One type, one manifest per OS.** Each manifest either carries a manifest `condition` or is
  shipped only on its OS. furnish does the latter at compile time.
- **One type, one native binary per OS**, with JSON in and out via `jsonInputArg`. Examples:
  `filecontent`, `registry`, `windows_service`.
- **Shell resources** use `input: env` (`apt`, `brew`). There is no JSON parser in POSIX `sh`, so
  env input is the pragmatic choice there. PowerShell resources can use `input: stdin` with
  `ConvertFrom-Json`.

---

## 4. Idioms and best practices

Docs-derived rules are cited; the rest follow from section 3.

**Idempotency and side effects**

- `set` must be idempotent and converge to the desired state. Never assume DSC tested first:
  `skip_test`, `implementsPretest` and the `delete` path all skip the test.
- `get` must be side-effect free and fast; it runs up to 3 times per `set`. Never install,
  refresh indexes (`apt update`), or prompt from `get`.

**`_exist` contract** (`docs/reference/schemas/resource/properties/exist.md`)

- Return `_exist: false` for a missing instance; omitting `_exist` means "exists".
- Always return a boolean, never `"false"`. The built-in `apt` resource returns a string, which is
  an example *not* to copy.

**Output and schema**

- Return the **full actual state** from `get`, including key properties, so that the synthetic
  test and `reference()` work.
- Keep the schema strict (`additionalProperties: false`, `required` keys) to catch typos in app
  files. Remember that this also drops `_metadata` unless you declare it.
- Mark computed properties such as an installed version as `readOnly`. Don't let admins set them
  in desired state: the synthetic test would compare them literally.
- Don't use the synthetic test for properties that need fuzzy comparison (version ranges,
  case-insensitive IDs). Implement `test` with `return: state` and `_inDesiredState` instead
  (`docs/concepts/resources/capabilities.md`).
- Inline the canonical-property schemas; remote `$ref` fails (3.1).

**Messages and exit codes**

- Write nothing but the JSON to stdout. Use JSON lines on stderr for user-relevant messages:
  `{"info":"Installing hyperfine via pacman"}`.
- Map only exit codes that the script itself emits on purpose, and choose codes that underlying
  tools are unlikely to use (for example 10 and up). Let tool failures surface with stderr
  attached.

**Manifest metadata**

- `type`: use `Owner.Area/Name` with the owner first, for example `Furnish/Package` or
  `Furnish.Desktop/Shortcut`. Bump the manifest `version` (semver) when the schema changes; app
  files can then pin it with `requireVersion: "^0.2"`.
- `$schema` URIs: use major-version pins (`/v3/`) for both manifests and documents.

**Elevation**

- Declare `requireSecurityContext` only if the resource itself cannot elevate. DSC refuses to run
  the operation otherwise.

**App files**

- Keep each instance self-contained and use `condition` per OS or per tool.
- Don't rely on `envvar()` for variables that may be missing: it errors and there is no "try"
  variant.

---

## 5. Version notes (v3.0 → v3.3)

From `CHANGELOG.md` and the
[v3.3.0 announcement](https://devblogs.microsoft.com/powershell/announcing-dsc-v3-3-0/).

- **v3.1**
  - `_exist` and `handlesExist` existed already.
  - Resource instances could not use `condition`.
- **v3.2.0**
  - Resource instances gained `condition`, plus `copy` (now deprecated).
  - Manifests gained `condition`, `deprecationMessage` and `metadata`.
  - Documents gained top-level `directives`, `outputs` and `functions`.
  - Functions added: `and`, `or`, `not`, `bool`, `true`, `false`, `less`, and more.
  - Execution features: `whatIfArg`, `set.whatIfReturns`, `_metadata._refreshEnv`.
  - Discovery features: manifest lists (`*.dsc.manifests.json`), synthetic export.
  - New canonical properties: `_name`, `_securityContext`, `_metadata`, `_restartRequired`.
  - `executionInformation` replaced the `Microsoft.DSC` metadata namespace.
  - `adapter.config` was renamed `inputKind`.
- **v3.3.0**
  - `format()` is now stable.
  - New functions: `stateChanged()` and `restartRequired()`.
  - `export.supportsFiltering` (experimental).
  - `--required-version` on `dsc resource` commands.
  - New Windows resources: `RegistryList`, `WindowsFeatureList`, `Personalization`, and a
    Registry adapter.
  - `dsc mcp` was renamed `dsc server`.
  - The synthetic test now ignores `writeOnly` properties and uses schema defaults (source).
  - The repo's own CHANGELOG at the tag stops at v3.2.2; the release notes are on GitHub.

---

## 6. Implications for furnish

### 6.1 Defects in the current code

Each of these was observed with the current furnish build.

**1. An app whose every resource is conditioned out shows as "installed".**

- An app with only a Windows resource, run on Linux, gives `invoke_test` no results;
  `classify(&[])` then returns `Installed`.
- `furnish install` prints "already installed".
- *Fix:* if no resource ran, report "not available on this OS".

**2. Uninstall runs in the wrong order when `dependsOn` is used.** *(Fixed: furnish now runs one resource at a time in its own computed order, reversed for uninstall; see `engine::run`.)*

- `engine::remove` reverses the resource array. DSC still pulls dependencies in first (1.3), so
  for `b dependsOn a` the uninstall removed `a` before `b`.
- *Fix:* compute the order yourself. Run `get_resource_invocation_order` equivalent ordering,
  reverse it, and strip `dependsOn`. Alternatively, invert the edges.

**3. Uninstall fails on any resource without `_exist` support.**

- Flipping `_exist` gives the error "does not support delete and does not handle _exist as false".
  If the resource's schema lacks `_exist`, the result is a schema error instead.
- Uninstall also removes shared prerequisites, such as a repo source or a runtime other apps
  need.

**4. Exit-code mapping hides failure details.**

- Unix manifest maps `1` to "Invalid arguments". `pacman`, `apt-get` and `dnf` exit 1 on most
  failures, and `set -eu` passes that through. A failed install is reported as "Invalid arguments"
  and the package manager's message is dropped.
- Also, `: "${name:?}"` exits with 2 in dash, which is mapped to "No supported package manager".
- Windows maps `3` to "winget failed", so the winget output never reaches the user.

**5. Resource messages are invisible.**

- furnish installs no `tracing` subscriber, so `{"info":...}` lines and all trace-level tool
  output are discarded.
- `fail_on_errors` checks `had_errors` and `messages`, which v3.3.0 never sets. That code is dead;
  errors arrive as `Err`.

**6. Every install spawns needless processes.**

- `furnish install` first runs `status` (one `get` per resource).
- `invoke_set(false)` then runs test → `get`, pre-state `get`, `set`, and post-state `get`.
- That is four `get` calls per resource, and on Windows each one starts a `powershell.exe`.

**7. `NeedsUpdate` can never occur for `Furnish/Package`.**

- The synthetic test compares only `name` and `_exist`.
- The Windows `get` returns no version.

### 6.2 Recommendations for `Furnish/Package`

1. **Add `implementsPretest: true` and `return: state` to `set`.**
   - `pacman --needed`, `apt-get install`, `brew install` and `winget install` are already
     idempotent.
   - Have `set` print the post-state JSON, the same as `get`.
   - This removes two of the four `get` calls.
   - On Windows, also return `"_metadata":{"_refreshEnv":true}` and allow `_metadata` in the
     schema, so that a following resource sees the updated `PATH`.
   - `_refreshEnv` resets `PATH` from the registry, dropping furnish's prepended directory. That
     is harmless, because DSC falls back to the manifest directory (3.2).
2. **Fix exit codes.**
   - Drop the `"1"` mapping.
   - Use 10 and above for the script's own conditions (missing manager, bad input).
   - Let tool failures exit with their own unmapped code so that stderr is attached.
   - Emit a short `{"error":"pacman -S hyperfine failed"}` line before exiting.
3. **Report progress.**
   - Emit `{"info":"..."}` lines from the scripts.
   - In furnish, install a `tracing_subscriber` layer that forwards INFO and above, plus resource
     ERROR lines, to the UI.
   - `ProgressFormat::Json` writes per-resource progress to stderr via `eprintln!`. That suits a
     CLI but cannot be captured in-process; the `tracing` layer is the in-process hook.
4. **`test`.**
   - Not needed while equality on `name` and `_exist` suffices.
   - Implement it (with `return: state` and `_inDesiredState`) when "update available" should mean
     "newer version in the repo". `get` must stay offline-cheap. Note that
     `pacman -Qu`, `apt list --upgradable` and `winget upgrade` depend on refreshed indexes.
5. **`delete`.** Not needed: `handlesExist` covers removal. Add it only if furnish wants to call
   `dsc resource delete`-style removal directly.
6. **What-if.**
   - The synthetic what-if (test → set result) already works for "would install / would remove".
   - Set `configurator.context.execution_type = ExecutionKind::WhatIf` to preview an install.
   - Add `whatIfArg` only if the scripts can do real dry runs (`apt-get -s`, `dnf --assumeno`).
7. **`export`.**
   - Useful only to list installed packages. The synthetic export already covers single
     instances. Low priority.
8. **Input mode.**
   - Keep `env` on Unix: there is no JSON parser in `sh`.
   - Guard against inherited variables by always emitting every property from furnish's side, or
     by unsetting stale names. The cheapest fix is to rename the property to something unlikely in
     a user environment. `name` is fine because it is required and always set.
   - On Windows, move to `input: stdin` with `ConvertFrom-Json`: the env namespace there is
     case-insensitive and shared.
9. **Security context.**
   - Don't set `requireSecurityContext: elevated` or `directives.securityContext`. furnish runs as
     the user and elevates per command (`sudo`/`pkexec`), and `brew` refuses root.
   - Consider `requireSecurityContext: restricted` on the macOS `brew` manifest's `set`, so that a
     root run fails clearly.
10. **Schema.**
    - Keep `version` as `readOnly`.
    - Escape `name` when hand-building JSON in `sh` (the current `printf` breaks on `"` or `\`), or
      validate it with a `pattern` in the schema, such as `^[A-Za-z0-9@._+-]+$`.
11. **No timeout.**
    - DSC waits forever on a hung child, for example winget waiting on a UAC prompt or a network
      stall. Add a timeout in the scripts, or run the configurator on a thread that can be
      abandoned.

### 6.3 Recommendations for the furnish engine and the app-file format

- **Uninstall model.** Flipping `_exist` is sound for resources that are "the app" (a package)
  and unsound for prerequisites and for configuration resources. Options, cheapest first:
  - Flip only resources whose type is in an allow-list (`Furnish/Package`), and leave the others
    alone.
  - Let authors mark instances with `metadata.furnish.uninstall: false`. Instance `metadata`
    passes through untouched; `Furnish/Package`'s strict schema drops `_metadata`, so the resource
    ignores it.
  - Support an optional explicit `apps/<app>.uninstall.dsc.yaml` for complex apps.

  In every case, compute the reverse dependency order yourself (6.1, item 2).
- **Status.**
  - Treat "no resource ran" as "unavailable" (6.1, item 1).
  - A `Group` result can't occur, because furnish can't run `Microsoft.DSC/Group` (1.5). The
    recursive `flatten` is harmless but unnecessary.
- **Allowed resource set.** Document for admins that only bundled resources work. Groups,
  assertions, includes, adapters and the Windows built-ins all need the `dsc` executable or
  separate binaries (1.5).
- **Parameters.** Not exposed today. If added, pass them through `set_context(Some(...))`. Avoid
  `secureString` for anything a resource reads through env input (1.1, 3.2).
- **Manifest `condition`.** furnish could ship both OS variants as one manifest list using a
  manifest `condition` instead of `cfg`. That is optional; the current compile-time split is
  equivalent.
- **Tracing.** One subscriber gives progress (`info`), warnings, and the trace-level tool output
  for a `--verbose` flag.
