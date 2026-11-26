---
title: Connect a reporting backend
order: 42
group: Administrators
previous: endpoint
---
# Connect a reporting backend

## Reports

furnish sends reports as OpenTelemetry logs over OTLP/HTTP with JSON. Any OTLP receiver can take them. The proxy forwards them to the service `receiver`, which is the [OpenTelemetry Collector](https://opentelemetry.io/docs/collector/) by default. It adds the user the proxy names to each record and prints each request as a JSON line (`docker compose logs receiver`). Replace the `file/stdout` exporter with the [exporter](https://opentelemetry.io/docs/collector/components/exporter/) for your log system, such as `otlp_http` for Grafana Cloud, Datadog or another OTLP backend, `elasticsearch`, `splunk_hec` or `loki`.

@include packaging/endpoint/collector/collector.yaml

To use a receiver you already run, such as Grafana Alloy, point the proxy at it instead of `receiver:4318`. It must add the `X-Furnish-User` header to each record and drop user attributes the client sent, like the processors above. Without that, a client could name any user.

### Vector instead of the Collector

`vector/compose.yml` replaces the Collector with [Vector](https://vector.dev). Vector's `opentelemetry` source takes only protobuf, so an `http_server` source reads furnish's JSON and makes one event per log record. Add the file after the proxy's: `docker compose -f docker-compose.yml -f caddy/compose.yml -f vector/compose.yml up -d`. Replace the `stdout` sink with the [sink](https://vector.dev/docs/reference/configuration/sinks/) for your log system. The Loki add-on below uses the Collector.

@include packaging/endpoint/vector/vector.yaml

## Look at the reports

Follow the [Loki and Grafana setup](https://github.com/furnish-self-service/furnish/tree/v0.2.0/packaging/endpoint/reports) for the Compose commands, dashboard and retention settings.
