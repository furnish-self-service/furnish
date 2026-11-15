---
title: Endpoint setup
order: 6
---
# Endpoint setup

A furnish address such as `furnish.myorg.tld` lets people get your organization's collections from an internal GitLab with their organization account, without a VPN, and sends their [reports](admins.md#reports) to your log system. Behind the address runs a reverse proxy. This page sets one up with Docker Compose, with Caddy, nginx, Traefik, Envoy or HAProxy. The configurations on this page are the files CI tests.

```
furnish ── HTTPS, Bearer <ID token> ──▶ proxy: is the token valid?
                                          ├──▶ GitLab: one repository, read-only deploy token
                                          └──▶ Vector: POST /events, user from the token
```

furnish reads `https://furnish.myorg.tld/.well-known/furnish.json`, signs the person in at your identity provider and runs git against the address with their ID token. The proxy checks the token, then forwards the request to GitLab with a deploy token that only it knows. GitLab stays internal: only the proxy host reaches it.

## What the proxy lets through

Each collection gets a public path for exactly one GitLab repository. For the collection `acme`, kept in `/it/catalog.git`, the proxy passes on only these requests, which are all git needs to clone and fetch:

| Request | Forwarded to GitLab as |
|---|---|
| `GET /git/acme.git/info/refs?service=git-upload-pack` | `GET /it/catalog.git/info/refs?service=git-upload-pack` |
| `POST /git/acme.git/git-upload-pack` | `POST /it/catalog.git/git-upload-pack` |

Besides those, it serves `GET /.well-known/furnish.json` to everyone and accepts `POST /events` from signed-in people.

- The proxy compares the request exactly as sent. `..`, `%2e`, `%2f`, `//`, extra path segments or query parameters, other methods, pushes and every other path get 404, and nothing from the client's path reaches GitLab.
- Only `Authorization: Bearer <ID token>` signs in. The token must come from your issuer, carry your client ID as audience, be unexpired, have `email_verified` set and, if you set `ALLOWED_EMAIL_DOMAINS`, an e-mail address in one of them. Cookies and tokens sent as a Basic password don't count.
- The proxy replaces the person's token with the deploy token and removes other GitLab credentials the client sends. If GitLab rejects the deploy token, the client gets 502, so git doesn't ask the person for a password.
- Reports may be at most 1 MiB per request. The proxy names the user in `X-Furnish-User` from the checked token; a name the client sends is dropped.

## Before you start

1. **Register furnish at your identity provider** as a public client (no secret): client ID `furnish`, authorization code with PKCE (`S256`), redirect URI `http://127.0.0.1/callback` on any port, scopes `openid email`. In Keycloak, the valid redirect URI is `http://127.0.0.1/*`. In Microsoft Entra ID, add the platform "Mobile and desktop applications" with `http://127.0.0.1/callback`.
2. **Create a deploy token** in each GitLab project behind a collection: Settings → Repository → Deploy tokens, scope `read_repository`.
3. **Get a certificate** for the address: `tls.crt`, with intermediates, and `tls.key` in a folder `certs` next to the configuration. Caddy can get one from Let's Encrypt instead.

## Configure

Download the folder [`packaging/endpoint`](https://github.com/furnish-self-service/furnish/tree/main/packaging/endpoint). Copy `.env.example` to `.env`, fill it in and make it readable by admins only (`chmod 600 .env`):

@include packaging/endpoint/.env.example

`ACME_GITLAB_AUTH` is base64 of `deploy-token-username:deploy-token`:

```sh
printf '%s' 'gitlab+deploy-token-42:gldt-xxxxxxxxxxxxxxxxxxxx' | base64 -w0
```

Edit `furnish.json`: your organization's name, the issuer, the client ID, one entry per collection, and the address of reports:

@include packaging/endpoint/furnish.json

`docker-compose.yml` holds what every proxy uses: oauth2-proxy, which checks tokens for the proxies that ask it, and Vector, which receives the reports. Each proxy has its own folder with its configuration and a second compose file.

@include packaging/endpoint/docker-compose.yml

## Reports with Vector

The proxy forwards reports to Vector, which by default prints each one as a JSON line (`docker compose logs vector`). Replace the `stdout` sink with the [sink](https://vector.dev/docs/reference/configuration/sinks/) for your log system, such as `file`, `loki`, `elasticsearch` or `splunk_hec`.

@include packaging/endpoint/vector/vector.yaml

## Look at the reports

@include-md packaging/endpoint/reports/README.md

@include packaging/endpoint/reports/compose.yml

@include packaging/endpoint/reports/vector.yaml

@include packaging/endpoint/reports/loki.yaml

[`tests/reports/run.sh`](https://github.com/furnish-self-service/furnish/blob/main/tests/reports/run.sh) starts these files, sends reports from several devices to Vector, and checks what Loki stored and that each panel of the dashboard shows data.

## Pick a proxy

All five do the same and pass the same tests. Pick the one your team already runs.

| Proxy | Checks the token | Certificate | Differences |
|---|---|---|---|
| [Caddy](#caddy) | asks oauth2-proxy | Let's Encrypt, or your own | the shortest configuration |
| [nginx](#nginx) | asks oauth2-proxy (`auth_request`) | your own | the allowlist is a list of regular expressions |
| [Traefik](#traefik) | asks oauth2-proxy (`forwardAuth`) | your own | needs a small extra container; treats `%65` in a path like `e` |
| [Envoy](#envoy) | asks oauth2-proxy (`ext_authz`) | your own | answers 502 for a rejected deploy token with a filter Envoy calls work in progress |
| [HAProxy](#haproxy) | itself (`jwt_verify`) | your own | needs the identity provider's signing key as a file, renewed when the provider changes it |

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

Then add the address in furnish. [`tests/endpoint/run.sh`](https://github.com/furnish-self-service/furnish/blob/main/tests/endpoint/run.sh) runs these configurations against a test identity provider and a test GitLab, with about 120 checks: clone and fetch, refused tokens and pushes, the allowlist, the deploy token never reaching a client, and reports arriving with the right user. `PROXY=nginx tests/endpoint/run.sh` tests one proxy.

## Limits

- A token stays valid until it expires: the proxy checks it without asking the identity provider, so revoking a session doesn't stop it. Keep the ID token lifetime short.
- Everyone who passes the checks gets every collection of the address. Restrict who signs in with `ALLOWED_EMAIL_DOMAINS`, or with oauth2-proxy's `OAUTH2_PROXY_ALLOWED_GROUPS` if your identity provider puts groups into the ID token.
- Any token from the issuer with your client ID as audience counts, also one issued to another client whose `aud` lists `furnish`.
