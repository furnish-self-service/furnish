# furnish endpoint with Envoy

This variant runs the endpoint behind Envoy instead of Caddy. It lets through the same requests,
checks tokens with the same oauth2-proxy and sends reports to the same OTLP receiver, so the
[endpoint setup](https://furnish-self-service.github.io/docs/alpha/endpoint.html) applies, apart from the points below.

## Start

Envoy has no ACME client: put a certificate and key for `FURNISH_HOST` into `certs/` next to
`docker-compose.yml` as `tls.crt` (with the intermediate certificates) and `tls.key`, or set
`TLS_DIR` to another directory. Envoy reads them at start, so restart the proxy after renewing
them. Then, in `packaging/endpoint/`:

```sh
docker compose -f docker-compose.yml -f envoy/compose.yml up -d
```

`FURNISH_TLS` is not used. The other settings in `.env` are the same as for Caddy.

## Add a collection

1. Create a deploy token (`read_repository`) in the GitLab project.
2. In `envoy/envoy.yaml`, copy the three routes under `Collection acme` and change the public name
   in all `:path` matches (`/git/apps.git/...`), the repository path in all `substitution`
   lines (`/it/apps.git/...`) and the credentials variable (`${APPS_GITLAB_AUTH}`). Give the
   copied `request_headers_to_add` anchor a new name (`&apps_auth`) and refer to it in the
   POST routes (`*apps_auth`). The other anchors (`*gitlab_drop`, `*gitlab_drop_down`,
   `*gitlab_filters`) are shared. Reuse the existing `*git_post_filters` on each copied POST route.
3. Add `APPS_GITLAB_AUTH: ${APPS_GITLAB_AUTH:?set APPS_GITLAB_AUTH}` to `environment` in
   `envoy/compose.yml` and the value to `.env`.
4. Add the collection to `furnish.json`.
5. `docker compose -f docker-compose.yml -f envoy/compose.yml up -d`.

## How it differs from Caddy

Envoy cannot read environment variables in its configuration. `envoy/entrypoint.sh` replaces
each `${NAME}` in `envoy.yaml` with the variable's value when the container starts and refuses to
start if one is unset or contains characters other than letters, digits and `.:_+/=-`. The
rendered file, which holds the deploy tokens, is readable only by the envoy user. There is no
admin listener, because its `/config_dump` would show the tokens.

Envoy's `ext_authz` filter accepts only status 200 from the authorization service, while
oauth2-proxy's `/oauth2/auth` answers 202. The check therefore goes through a second listener on
`127.0.0.1:10000` inside the container. It checks the `Authorization: Bearer` format, asks
oauth2-proxy, and turns its 202 into 200 with a local reply mapper. The user's e-mail
(`X-Auth-Request-Email`) is kept on the way.

The 502 for a deploy token that GitLab rejects comes from Envoy's `custom_response` filter, which
Envoy marks as work in progress and logs a warning about it at start. Local replies pass through that
filter too, so it only acts on responses that carry Envoy's `x-envoy-upstream-service-time`
header, which only upstream responses have; the header reaches the client.

Envoy checks the token before any filter reads the body. The `buffer` filter, which enforces
the 1 MiB limit on reports and the 32 MiB limit on `git-upload-pack`, comes after `ext_authz`.
While the check runs, Envoy reads at most 32 KiB per connection
(`per_connection_buffer_limit_bytes`) and, over HTTP/2, at most 64 KiB per stream and 1 MiB per
connection, with at most 100 streams per connection. A body over the limit therefore always
reaches the buffer filter in pieces and gets 413; a report that declares more than 1 MiB gets
413 at once. The overload manager stops accepting connections beyond 5000 and stops taking
requests when Envoy's heap reaches 98% of 1 GiB.

Git requests are limited per user with the `local_ratelimit` filter: it keys on the e-mail
address from the token check and allows 2 requests a second on average, in bursts of up to 60.

`furnish.json` is read when Envoy starts. Restart the proxy after changing it:
`docker compose -f docker-compose.yml -f envoy/compose.yml restart proxy`.

Port 80 redirects to HTTPS on `FURNISH_HOST`.

If GitLab uses a certificate from a private CA, mount the CA into the container and change
`trusted_ca` in the `gitlab` cluster in `envoy.yaml`.

Image version: `envoyproxy/envoy:v1.40.2`.
