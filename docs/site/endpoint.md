---
title: Endpoint setup
order: 6
---
# Endpoint setup

A furnish address such as `furnish.myorg.tld` lets people get your organization's collections from an internal GitLab with their organization account, without a VPN, and sends their [reports](admins.md#reports) to your log system. Behind the address runs a reverse proxy. This page sets one up with Docker Compose, with Caddy, nginx, Traefik, Envoy or HAProxy. The configurations on this page are the files CI tests.

```
furnish ── HTTPS, Bearer <ID token> ──▶ proxy: is the token valid?
                                          ├──▶ GitLab: one repository, read-only deploy token
                                          └──▶ OTLP receiver: POST /v1/logs, user from the token
```

furnish reads `https://furnish.myorg.tld/.well-known/furnish.json`, signs the person in at your identity provider and runs git against the address with their ID token. The proxy checks the token, then forwards the request to GitLab with a deploy token that only it knows. GitLab stays internal: only the proxy host reaches it.

## What the proxy lets through

Each collection gets a public path for exactly one GitLab repository. For the collection `acme`, kept in `/it/catalog.git`, the proxy passes on only these requests, which are all git needs to clone and fetch:

| Request | Forwarded to GitLab as |
|---|---|
| `GET /git/acme.git/info/refs?service=git-upload-pack` | `GET /it/catalog.git/info/refs?service=git-upload-pack` |
| `POST /git/acme.git/git-upload-pack` | `POST /it/catalog.git/git-upload-pack` |

Besides those, it serves `GET /.well-known/furnish.json` to everyone and accepts reports as OpenTelemetry logs, `POST /v1/logs`, from signed-in people.

- The proxy compares the request exactly as sent. `..`, `%2e`, `%2f`, `//`, extra path segments or query parameters, other methods, pushes and every other path get 404, and nothing from the client's path reaches GitLab.
- Only `Authorization: Bearer <ID token>` signs in. The token must come from your issuer, carry your client ID as audience, be unexpired and not dated more than 5 minutes ahead (`nbf`), have an e-mail address in one of `ALLOWED_EMAIL_DOMAINS` (unless that is `*`) and, if you set `ALLOWED_GROUPS`, one of those groups in its `groups` claim. A token with `email_verified: false` is refused; see [Limits](#limits) for a token without the claim. Cookies and tokens sent as a Basic password don't count.
- The proxy checks the token before it reads a request body, so a client that isn't signed in can't make it hold one.
- The proxy replaces the person's token with the deploy token and removes other GitLab credentials the client sends. If GitLab rejects the deploy token, the client gets 502, so git doesn't ask the person for a password.
- GitLab gets the client's address and the endpoint's host name in `X-Forwarded-For`, `X-Real-IP`, `X-Forwarded-Host` and `X-Forwarded-Proto` from the proxy. What a client sends under these names, or in `Forwarded`, is replaced or dropped.
- Git requests and reports are limited per signed-in user: 2 requests a second on average, in bursts of up to 60, with 429 for the rest (Caddy has no limit, see [Pick a proxy](#pick-a-proxy)). A `git-upload-pack` request may be at most 32 MiB, a report request at most 1 MiB.
- The proxy names the user of a report to the receiver in `X-Furnish-User`, from the checked token. A header of that name from the client is dropped. The receiver adds the user to each log record as `user.email` and drops `user.*` attributes the client sent.

## Before you start

1. **Register furnish at your identity provider** as a public client (no secret): client ID `furnish`, authorization code with PKCE (`S256`), redirect URI `http://127.0.0.1/callback` on any port, scopes `openid email`. In Keycloak, the valid redirect URI is `http://127.0.0.1/*`. In Microsoft Entra ID, add the platform "Mobile and desktop applications" with `http://127.0.0.1/callback`, add the `groups` claim to the ID token (Token configuration → Add groups claim) and decide who may sign in by group (see [Limits](#limits)).
2. **Create a deploy token** in each GitLab project behind a collection: Settings → Repository → Deploy tokens, scope `read_repository`.
3. **Get a certificate** for the address: `tls.crt`, with intermediates, and `tls.key` in a folder `certs` next to the configuration. Caddy can get one from Let's Encrypt instead.

## Configure

Download the folder [`packaging/endpoint`](https://github.com/furnish-self-service/furnish/tree/main/packaging/endpoint). Copy `.env.example` to `.env`, fill it in and make it readable by admins only (`chmod 600 .env`). Create the folder `secrets` as the comments in the file say:

@include packaging/endpoint/.env.example

`ACME_GITLAB_AUTH` is base64 of `deploy-token-username:deploy-token`:

```sh
printf '%s' 'gitlab+deploy-token-42:gldt-xxxxxxxxxxxxxxxxxxxx' | base64 -w0
```

Edit `furnish.json`: your organization's name, the issuer, the client ID, one entry per collection, and the address of reports. `events` is the full OTLP logs URL on the address, like `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT`. The proxies take reports at the standard path `/v1/logs`:

@include packaging/endpoint/furnish.json

The fields and what furnish accepts in them are in [Sign-in without a VPN](admins.md#sign-in-without-a-vpn). furnish fetches `furnish.json` again whenever it updates all collections, so a change reaches everyone who added the address.

`docker-compose.yml` holds what every proxy uses: oauth2-proxy, which checks tokens for the proxies that ask it, and the OpenTelemetry Collector, which receives the reports. Each proxy has its own folder with its configuration and a second compose file.

@include packaging/endpoint/docker-compose.yml

Every container runs without capabilities (apart from what the proxy needs to start), with a read-only file system and `no-new-privileges`. The containers are on separate networks: only the proxy is on `edge`, the network with the published ports; the proxy reaches oauth2-proxy and the receiver on `backend`; the receiver reaches Loki and Grafana reaches Loki on two internal networks of their own. So a weakness in the proxy doesn't give access to Loki or Grafana, and Grafana can't reach the proxy, oauth2-proxy or the internet. With your own certificate, the files in `TLS_DIR` must be readable by root in the container without special rights: owned by root, or readable by everyone in a folder only admins can open.

## Reports

furnish sends reports as OpenTelemetry logs over OTLP/HTTP with JSON. Any OTLP receiver can take them. The proxy forwards them to the service `receiver`, which is the [OpenTelemetry Collector](https://opentelemetry.io/docs/collector/) by default. It adds the user the proxy names to each record and prints each request as a JSON line (`docker compose logs receiver`). Replace the `file/stdout` exporter with the [exporter](https://opentelemetry.io/docs/collector/components/exporter/) for your log system, such as `otlp_http` for Grafana Cloud, Datadog or another OTLP backend, `elasticsearch`, `splunk_hec` or `loki`.

@include packaging/endpoint/collector/collector.yaml

To use a receiver you already run, such as Grafana Alloy, point the proxy at it instead of `receiver:4318`. It must add the `X-Furnish-User` header to each record and drop user attributes the client sent, like the processors above. Without that, a client could name any user.

### Vector instead of the Collector

`vector/compose.yml` replaces the Collector with [Vector](https://vector.dev). Vector's `opentelemetry` source takes only protobuf, so an `http_server` source reads furnish's JSON and makes one event per log record. Add the file after the proxy's: `docker compose -f docker-compose.yml -f caddy/compose.yml -f vector/compose.yml up -d`. Replace the `stdout` sink with the [sink](https://vector.dev/docs/reference/configuration/sinks/) for your log system. The Loki add-on below uses the Collector.

@include packaging/endpoint/vector/vector.yaml

## Look at the reports

@include-md packaging/endpoint/reports/README.md

@include packaging/endpoint/reports/compose.yml

@include packaging/endpoint/reports/collector.yaml

@include packaging/endpoint/reports/loki.yaml

[`tests/reports/run.sh`](https://github.com/furnish-self-service/furnish/blob/main/tests/reports/run.sh) starts these files, sends reports from several devices to the Collector, and checks what Loki stored and that each panel of the dashboard shows data.

## Pick a proxy

All five do the same and pass the same tests. Pick the one your team already runs.

| Proxy | Checks the token | Certificate | Differences |
|---|---|---|---|
| [Caddy](#caddy) | asks oauth2-proxy | Let's Encrypt, or your own | the shortest configuration; no rate limit |
| [nginx](#nginx) | asks oauth2-proxy (`auth_request`) | your own | the allowlist is a list of regular expressions |
| [Traefik](#traefik) | asks oauth2-proxy (`forwardAuth`) | your own | needs a small extra container; treats `%65` in a path like `e` |
| [Envoy](#envoy) | asks oauth2-proxy (`ext_authz`) | your own | answers 502 for a rejected deploy token with a filter Envoy calls work in progress |
| [HAProxy](#haproxy) | itself (`jwt_verify`) | your own | needs the identity provider's signing key as a file, renewed when the provider changes it; requires `email_verified` |

Where they differ in the checks:

| | Caddy | nginx | Traefik | Envoy | HAProxy |
|---|---|---|---|---|---|
| Token without `email_verified` | accepted | accepted | accepted | accepted | refused, unless `ALLOWED_GROUPS` is set |
| `email_verified: "true"` (a string) | accepted | accepted | accepted | accepted | refused |
| Rate limit on git requests and reports, per | none | token | token | user | user |
| 32 MiB limit on `git-upload-pack` | counted | counted | counted | counted | declared length only |
| 1 MiB limit on reports | counted | counted | counted | counted | declared length; no length: 411 |

"Counted" means the proxy counts the bytes it receives. HAProxy streams bodies and checks only `Content-Length`; git sends `git-upload-pack` requests over 1 MiB without one, and HAProxy lets those through without a limit. The limit per token in nginx and Traefik applies to one sign-in: a person who signs in again starts with a new allowance.

## Caddy

@include-md packaging/endpoint/caddy/README.md

@include packaging/endpoint/caddy/Caddyfile

@include packaging/endpoint/caddy/compose.yml

## nginx

@include-md packaging/endpoint/nginx/README.md

@include packaging/endpoint/nginx/default.conf.template

@include packaging/endpoint/nginx/compose.yml

## Traefik

@include-md packaging/endpoint/traefik/README.md

@include packaging/endpoint/traefik/traefik.yml

@include packaging/endpoint/traefik/dynamic.yml

@include packaging/endpoint/traefik/compose.yml

The extra container serves `furnish.json` and names the user for reports:

@include packaging/endpoint/traefik/files/httpd.conf

@include packaging/endpoint/traefik/files/user.sh

## Envoy

@include-md packaging/endpoint/envoy/README.md

@include packaging/endpoint/envoy/envoy.yaml

@include packaging/endpoint/envoy/entrypoint.sh

@include packaging/endpoint/envoy/compose.yml

## HAProxy

@include-md packaging/endpoint/haproxy/README.md

@include packaging/endpoint/haproxy/haproxy.cfg

@include packaging/endpoint/haproxy/start.sh

@include packaging/endpoint/haproxy/jwks-to-pem.sh

@include packaging/endpoint/haproxy/compose.yml

## Check it

```sh
curl https://furnish.myorg.tld/.well-known/furnish.json
curl -i https://furnish.myorg.tld/git/acme.git/info/refs?service=git-upload-pack   # 401: no sign-in
```

Then add the address in furnish. [`tests/endpoint/run.sh`](https://github.com/furnish-self-service/furnish/blob/main/tests/endpoint/run.sh) runs these configurations against a test identity provider and a test GitLab, with about 150 checks: clone and fetch, refused tokens and pushes, the allowlist, the deploy token never reaching a client, forwarding headers, size and rate limits, request bodies of clients that aren't signed in, and reports arriving with the right user. `PROXY=nginx tests/endpoint/run.sh` tests one proxy.

## Limits

- A token stays valid until it expires: the proxy checks it without asking the identity provider, so revoking a session doesn't stop it. Keep the ID token lifetime short.
- Everyone who passes the checks gets every collection of the address. `ALLOWED_EMAIL_DOMAINS` and `ALLOWED_GROUPS` decide who that is.
- `ALLOWED_EMAIL_DOMAINS` only restricts if the identity provider vouches for the e-mail address with `email_verified: true` in the ID token. Keycloak, Google and Okta do. oauth2-proxy refuses a token with `email_verified: false` but accepts one without the claim, and has no setting to require it; HAProxy requires it unless `ALLOWED_GROUPS` is set.
- Microsoft Entra ID sends no `email_verified`, and its `email` claim is an address that can be set without proof that the user owns it. With Entra ID, set `ALLOWED_EMAIL_DOMAINS=*`, add the `groups` claim to the ID token and put the IDs of the groups that may sign in into `ALLOWED_GROUPS`. Turning on "Assignment required" for the enterprise application and assigning those groups also keeps everyone else from getting a token.
- Any token from the issuer with your client ID as audience counts, also one issued to another client whose `aud` lists `furnish`. That includes a token furnish got for another address: someone who runs a lookalike address (`furnish-acme.tld`) can name your issuer and client ID in its `furnish.json`, and people who add it sign in at your identity provider and send their token to the lookalike, which can use it at your address. furnish shows where people sign in before they do, and your policy can allow only your addresses (`"usersCanAdd": false` with `endpoints`), but the proxy can't tell such a token apart. Where your identity provider supports it, give each address its own client ID or audience, so a token for one address doesn't count at another.
- The rate limit protects GitLab from one user, not the proxy from a flood of requests without sign-in; those cost the proxy a token check each. Put a firewall or your provider's flood protection in front for that.
