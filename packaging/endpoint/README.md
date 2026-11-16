# furnish endpoint

The reverse proxy behind a furnish address such as `furnish.myorg.tld`: people get your
organization's collections from an internal GitLab with their organization account, without a
VPN, and send reports as OpenTelemetry logs to your log system. The setup guide is
[docs/site/endpoint.md](../../docs/site/endpoint.md), published at
<https://furnish-self-service.github.io/docs/alpha/endpoint.html>; it shows the files in this
folder as they are.

| Path | What it is |
| --- | --- |
| [`docker-compose.yml`](docker-compose.yml) | oauth2-proxy and the OpenTelemetry Collector, used with every proxy |
| [`collector/collector.yaml`](collector/collector.yaml) | Where reports go |
| [`vector/`](vector) | Optional: Vector instead of the Collector |
| [`furnish.json`](furnish.json) | Example discovery document |
| [`.env.example`](.env.example) | Settings to copy into `.env` |
| [`reports/`](reports) | Optional: Loki and Grafana with a dashboard of the reports |
| [`caddy/`](caddy), [`nginx/`](nginx), [`traefik/`](traefik), [`envoy/`](envoy), [`haproxy/`](haproxy) | One proxy each: its configuration and `compose.yml` |

Start with one proxy:

```sh
docker compose -f docker-compose.yml -f caddy/compose.yml up -d
```

[`tests/endpoint/run.sh`](../../tests/endpoint/run.sh) tests these files (`PROXY=nginx` for
another proxy); CI runs it for all five.
