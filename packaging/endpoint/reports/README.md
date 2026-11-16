# furnish reports in Grafana

This folder adds Loki (`grafana/loki:3.7.9`), which stores the reports, and Grafana
(`grafana/grafana:13.3.0`) with the dashboard "furnish reports" to the endpoint. The
OpenTelemetry Collector keeps printing each report to its log and also sends it to Loki's OTLP
endpoint.

## Start

Put Grafana's admin password into the secrets folder (`SECRETS_DIR` in `.env`), and optionally
set how long Loki keeps reports in `.env`:

```sh
openssl rand -base64 24 | tr -d '\n' > secrets/grafana_admin_password && chmod 644 secrets/grafana_admin_password
echo REPORTS_RETENTION=90d >> .env
```

In `packaging/endpoint/`, add `reports/compose.yml` after the proxy's file:

```sh
docker compose -f docker-compose.yml -f caddy/compose.yml -f reports/compose.yml up -d
```

Grafana listens on port 3000 of this host only. Open <http://127.0.0.1:3000> on the host, or
from your computer through an SSH tunnel (`ssh -L 3000:127.0.0.1:3000 <host>`), and sign in as
`admin` and that password. The dashboard opens as the home page. Grafana stores the
password in its database on first start; change it later with
`docker compose ... exec grafana grafana cli admin reset-admin-password <new>`.

To make Grafana reachable from other computers, publish it behind TLS (for example as another
site of your proxy) and sign people in with your identity provider through Grafana's generic
OAuth. The comment in `reports/compose.yml` lists the settings.

## The dashboard

The time range at the top applies to every panel. The default is the last 30 days.

| Panel | What it shows |
| --- | --- |
| Failed operations by app | Failed installs, updates, uninstalls, reinstalls and fallbacks per app over time. |
| Failure rate per app | For each app, failed operations divided by all finished ones (succeeded or failed), with both counts. Cancelled operations don't count. |
| Fallbacks: apps installed at an earlier version | After a failed install or update, furnish can install the app from an earlier commit of its collection. The table lists the app, that commit and the number of devices. |
| furnish versions in use | Devices per furnish version, from each device's latest report. |
| Devices without inventory for 7d | Devices that sent an inventory in the time range but none in the last 7 days, with the time of their last one. Choose 1 to 30 days at the top. |
| Recent failures | Every failed operation and collection update with user, device, app (or collection) and error message, newest first. Open a line for all fields of the report. |

With `reporting: errors` furnish sends only failures and fallbacks, so the failure rate is 100%
for every app that appears, and devices send no inventory. A device in the fallback table stays
at the earlier version until a later update succeeds, which the table doesn't show. A device is
one user on one computer; its host name appears only with `reportHostname: true`.

Loki indexes reports by the labels `service_name`, `event` and `outcome` only. The other
attributes are structured metadata, with `_` for `.`: `furnish_app`, `furnish_collection`,
`device_id`, `user_email`, `service_version` and so on. Queries filter and group by them without a
parser. Edit queries in Grafana's Explore view with the data source "Loki", for example
`{service_name="furnish", event="install"} | furnish_app="acme/zoom"`.

Each combination of labels is a stream in Loki, and Loki refuses all reports once there are more
than 5000. Since the attributes of a report come from the client, `reports/collector.yaml` labels
only the values furnish sends and files everything else under `event="other"` or
`outcome="other"`; the attributes stay as they were. `reports/loki.yaml` turns off Loki's default
list of resource attributes it makes labels of, so a client can't add labels. The proxies,
apart from Caddy, limit requests per user, so one user can't use up what Loki takes in. Reports
keep the time they happened, also when a computer sends them weeks later; a time in the future
becomes the time the report arrived. The provisioned dashboard can't be saved from Grafana: save
a copy, or change `grafana/dashboards/furnish-reports.json`.

## Retention

Loki deletes reports older than `REPORTS_RETENTION` (default `90d`); its compactor removes them
within a few hours after they expire.
Set it in `.env` and run the `up -d` command again. Reports are personal data: keep them only as
long as you need them to fix problems. The data lives in the volumes `loki-data` and
`grafana-data`.

## Another log system

Loki receives reports through the pipeline `logs/loki` in `reports/collector.yaml`, which the
Collector loads in addition to `collector/collector.yaml`. To send reports elsewhere, start the
endpoint without `reports/compose.yml` and replace the `file/stdout` exporter in
`collector/collector.yaml` with the
[exporter](https://opentelemetry.io/docs/collector/components/exporter/) for your system. Any
OTLP backend, such as Grafana Cloud or Datadog, takes them through `otlp_http`.
