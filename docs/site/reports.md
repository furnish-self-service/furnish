---
title: Installation reports
order: 34
group: Administrators
previous: admins
---
# Installation reports

## Reports

furnish can report to your organization what it installed, updated or removed on each computer and whether that worked, through your furnish address. Reports are OpenTelemetry logs, sent over OTLP/HTTP. Any OTLP receiver takes them: the OpenTelemetry Collector, Grafana Alloy, Vector, Datadog, Grafana Cloud and others. The address's proxy accepts them from signed-in users only and names the user to the receiver. The receiver adds that user to each record. furnish itself never sends a user name, and one a client puts in a record is dropped. The [endpoint setup](endpoint.md) contains the configuration, with the OpenTelemetry Collector as the receiver.

To look at reports without a log system of your own, add Loki and Grafana to the endpoint: the dashboard "furnish reports" shows failed operations per app, fallbacks to earlier versions, the devices whose installed version of an app isn't acceptable or can't be checked, the furnish versions in use and devices that stopped reporting. See [Look at the reports](endpoint-reports.md#look-at-the-reports).

The setting `reporting` decides what furnish sends:

| Value | Reports |
|---|---|
| `off` (default) | none |
| `errors` | failed installs, updates and uninstalls, failed collection updates, installs of an earlier version after a failure, and installed versions that [required versions](versions.md#required-versions) don't accept |
| `all` | also successful ones, and once a day the list of installed apps |

People see the setting under Reports to your organization on the Settings page, and `furnish reports` shows what is waiting to be sent.

![The Reports to your organization section, set to send failures and fallbacks.](images/reports-setting.webp)

To turn reports on for everyone, set it in the policy, and lock it if people mustn't turn it off:

```json
{ "settings": { "reporting": { "value": "errors", "locked": true }, "reportHostname": { "value": false } } }
```

The address's `furnish.json` names where reports go, on the address itself: `"events": "https://furnish.myorg.tld/v1/logs"`. It is the full OTLP logs URL, like `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT`. Each address gets only the reports about its own collections, and the daily list of installed apps only with its apps; with several addresses, each sees its part. Error messages go out without passwords or tokens: furnish replaces credentials in URLs (`https://user:password@host`), `Bearer` and `Basic` credentials and JWTs with `***`. furnish keeps reports in `reports/queue.ndjson` in its data folder, readable only by the person, until the address accepts them, at most 5 MB, so computers that are offline report later. If the address answers 429, 502, 503 or 504, furnish waits as long as `Retry-After` says, or from 1 minute up to 1 hour, and tries again. Reports the address refuses with 400 or 413 are dropped, so they don't hold up the rest.

Each report is one log record. furnish sets the time it happened, the severity (`ERROR` for failures and for `noneOffered`, `WARN` for fallbacks and for `notAcceptable` and `unknown`, `INFO` otherwise), a short text such as `install acme/vpn failed` and these attributes:

| Attribute | Meaning |
|---|---|
| `service.name`, `service.version` | `furnish` and its version (resource) |
| `os.type`, `os.version` | `windows`, `darwin` or `linux`, and the system's version (resource) |
| `device.id` | A random ID furnish created for this user on this computer (resource) |
| `host.name` | The computer's name, only with `reportHostname: true` (resource) |
| `user.email` | The signed-in user's e-mail address, added by the receiver from the proxy's header |
| `furnish.event` | `install`, `update`, `uninstall`, `reinstall`, `fallback`, `collectionUpdate`, `requirement` or `inventory` |
| `furnish.outcome` | `succeeded`, `failed`, `cancelled`; `reported` for an inventory; for a requirement `notAcceptable` (the installed version isn't acceptable, an acceptable one is offered), `noneOffered` (no acceptable version is offered), `unknown` (furnish can't read the installed version, for example of a program installed outside furnish) or `acceptable` (acceptable again) |
| `furnish.app`, `furnish.collection` | The app (`collection/app`) and its collection |
| `furnish.collection.commit` | The collection's commit the app was installed from, or a collection update went to |
| `furnish.trigger` | `user`, `update` (automatic updates) or `autoInstall` |
| `error.message`, `furnish.duration_ms` | The error message of a failure, and how long it took |
| `furnish.apps` | For an inventory: each installed app as `app` and `status` (`installed` or `needsUpdate`) |
| `furnish.installed_version`, `furnish.offered_version` | For `notAcceptable` and `noneOffered`: the installed version and the newest one offered; for `unknown`, the newest one offered |

furnish reports a requirement only when it changes, so a computer that stays on a version that isn't acceptable reports it once. It keeps what it reported last in `reports/requirements.json`. The dashboard's panel of devices on a version that isn't acceptable therefore only lists devices that reported within its time range.

Loki, in the endpoint's reports add-on, makes labels of `furnish.event` and `furnish.outcome` and files values other than these under `other`, because a client could otherwise create so many streams that Loki stops taking reports.

Reports about the software people install are personal data, and in Germany a works council has a say before they are collected (§ 87 (1) 6 BetrVG). `errors` without host names collects the least that helps fix broken apps. To send reports without user names, remove the line that sets `X-Furnish-User` from the proxy's configuration.
