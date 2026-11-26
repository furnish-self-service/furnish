---
title: Deploy an endpoint
order: 40
group: Administrators
---
# Deploy an endpoint

A furnish address such as `furnish.myorg.tld` lets people get your organization's collections from an internal GitLab with their organization account, without a VPN, and sends their [reports](reports.md#reports) to your log system. Behind the address runs a reverse proxy. This page sets one up with Docker Compose, with Caddy, nginx, Traefik, Envoy or HAProxy.

```
furnish ── HTTPS, Bearer <ID token> ──▶ proxy: is the token valid?
                                          ├──▶ GitLab: one repository, read-only deploy token
                                          └──▶ OTLP receiver: POST /v1/logs, user from the token
```

furnish reads `https://furnish.myorg.tld/.well-known/furnish.json`, signs the person in at your identity provider and runs git against the address with their ID token. The proxy checks the token, then forwards the request to GitLab with a deploy token that only it knows. GitLab stays internal: only the proxy host reaches it.

## Before you start

1. **Register furnish at your identity provider** as a public client (no secret): client ID `furnish`, authorization code with PKCE (`S256`), redirect URI `http://127.0.0.1/callback` on any port, scopes `openid email`. In Keycloak, the valid redirect URI is `http://127.0.0.1/*`. In Microsoft Entra ID, add the platform "Mobile and desktop applications" with `http://127.0.0.1/callback`, add the `groups` claim to the ID token (Token configuration → Add groups claim) and decide who may sign in by group (see [Limits](endpoint-access.md#limits)).
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

For a collection that should update only on a release, add `"tracking": { "mode": "tag", "pattern": "^v[0-9]+[.][0-9]+[.][0-9]+$" }` to its `collections` entry. A team following a release branch can use `"tracking": { "mode": "branch", "name": "release" }`. [Release branches and tags](collection-updates.md#release-branches-and-tags) describes selection and policy precedence.

The fields and what furnish accepts in them are in [Sign-in without a VPN](sign-in.md#sign-in-without-a-vpn), which also says what happens when you add a collection or take one out. furnish fetches `furnish.json` again whenever it updates all collections, so a change reaches everyone who added the address.

`docker-compose.yml` holds what every proxy uses: oauth2-proxy, which checks tokens for the proxies that ask it, and the OpenTelemetry Collector, which receives the reports. Each proxy has its own folder with its configuration and a second compose file.

@include packaging/endpoint/docker-compose.yml

Every container runs without capabilities (apart from what the proxy needs to start), with a read-only file system and `no-new-privileges`. The containers are on separate networks: only the proxy is on `edge`, the network with the published ports; the proxy reaches oauth2-proxy and the receiver on `backend`; the receiver reaches Loki and Grafana reaches Loki on two internal networks of their own. So a weakness in the proxy doesn't give access to Loki or Grafana, and Grafana can't reach the proxy, oauth2-proxy or the internet. With your own certificate, the files in `TLS_DIR` must be readable by root in the container without special rights: owned by root, or readable by everyone in a folder only admins can open.

## Pick a proxy

Choose the proxy your team already operates.

| Proxy | Checks the token | Certificate | Differences |
|---|---|---|---|
| [Caddy](https://github.com/furnish-self-service/furnish/tree/v0.2.0/packaging/endpoint/caddy) | asks oauth2-proxy | Let's Encrypt, or your own | the shortest configuration; no rate limit |
| [nginx](https://github.com/furnish-self-service/furnish/tree/v0.2.0/packaging/endpoint/nginx) | asks oauth2-proxy (`auth_request`) | your own | the allowlist is a list of regular expressions |
| [Traefik](https://github.com/furnish-self-service/furnish/tree/v0.2.0/packaging/endpoint/traefik) | asks oauth2-proxy (`forwardAuth`) | your own | needs a small extra container; treats `%65` in a path like `e` |
| [Envoy](https://github.com/furnish-self-service/furnish/tree/v0.2.0/packaging/endpoint/envoy) | asks oauth2-proxy (`ext_authz`) | your own | answers 502 for a rejected deploy token with a filter Envoy calls work in progress |
| [HAProxy](https://github.com/furnish-self-service/furnish/tree/v0.2.0/packaging/endpoint/haproxy) | itself (`jwt_verify`) | your own | needs the identity provider's signing key as a file, renewed when the provider changes it; requires `email_verified` |

Follow the linked proxy guide to start its Compose services. Compare [authentication, rate and request-size limits](endpoint-access.md#proxy-differences) before choosing a proxy.

## Check it

```sh
curl https://furnish.myorg.tld/.well-known/furnish.json
curl -i https://furnish.myorg.tld/git/acme.git/info/refs?service=git-upload-pack   # 401: no sign-in
```

Then add the address in furnish. [`tests/endpoint/run.sh`](https://github.com/furnish-self-service/furnish/blob/main/tests/endpoint/run.sh) runs these configurations against a test identity provider and a test GitLab, with about 150 checks: clone and fetch, refused tokens and pushes, the allowlist, the deploy token never reaching a client, forwarding headers, size and rate limits, request bodies of clients that aren't signed in, and reports arriving with the right user. `PROXY=nginx tests/endpoint/run.sh` tests one proxy.

See [endpoint access rules and limits](endpoint-access.md) and [reporting backends](endpoint-reports.md).
