---
title: Endpoint access rules
order: 41
group: Administrators
previous: endpoint
---
# Endpoint access rules

## What the proxy lets through

Each collection gets a public path for exactly one GitLab repository. For the collection `acme`, kept in `/it/catalog.git`, the proxy passes on only these requests, for Git fetches and Git LFS download negotiation:

| Request | Forwarded to GitLab as |
|---|---|
| `GET /git/acme.git/info/refs?service=git-upload-pack` | `GET /it/catalog.git/info/refs?service=git-upload-pack` |
| `POST /git/acme.git/git-upload-pack` | `POST /it/catalog.git/git-upload-pack` |
| `POST /git/acme.git/info/lfs/objects/batch` | `POST /it/catalog.git/info/lfs/objects/batch` |

Besides those, it serves `GET /.well-known/furnish.json` to everyone and accepts reports as OpenTelemetry logs, `POST /v1/logs`, from signed-in people.

- The proxy compares the request exactly as sent. `..`, `%2e`, `%2f`, `//`, extra path segments or query parameters, other methods, pushes and every other path get 404, and nothing from the client's path reaches GitLab.
- Only `Authorization: Bearer <ID token>` signs in. The token must come from your issuer, carry your client ID as audience, be unexpired and not dated more than 5 minutes ahead (`nbf`), have an e-mail address in one of `ALLOWED_EMAIL_DOMAINS` (unless that is `*`) and, if you set `ALLOWED_GROUPS`, one of those groups in its `groups` claim. A token with `email_verified: false` is refused; see [Limits](#limits) for a token without the claim. Cookies and tokens sent as a Basic password don't count.
- The proxy checks the token before it reads a request body, so a client that isn't signed in can't make it hold one.
- The proxy replaces the person's token with the deploy token and removes other GitLab credentials the client sends. If GitLab rejects the deploy token, the client gets 502, so git doesn't ask the person for a password.
- GitLab gets the client's address and the endpoint's host name in `X-Forwarded-For`, `X-Real-IP`, `X-Forwarded-Host` and `X-Forwarded-Proto` from the proxy. What a client sends under these names, or in `Forwarded`, is replaced or dropped.
- Git requests and reports are limited per signed-in user: 2 requests a second on average, in bursts of up to 60, with 429 for the rest (Caddy has no limit, see [Pick a proxy](endpoint.md#pick-a-proxy)). A Git pack or LFS batch request may be at most 32 MiB, a report request at most 1 MiB.
- The proxy names the user of a report to the receiver in `X-Furnish-User`, from the checked token. A header of that name from the client is dropped. The receiver adds the user to each log record as `user.email` and drops `user.*` attributes the client sent.

## Git LFS downloads

For collections that track large files with Git LFS, configure GitLab’s LFS object storage with `proxy_download: false`. The batch response must give clients a presigned HTTPS download URL they can reach and whose certificate they trust. A download action that points back to an internal GitLab host will fail outside that network. Only batch metadata passes through the Furnish endpoint. The client downloads the object directly from storage. [GitLab object storage configuration](https://docs.gitlab.com/administration/object_storage/#proxy-download)

The Windows and macOS installers include Git LFS. Linux DEB, RPM and Nix packages provide it as a dependency. For standalone CLI and source installs, make `git lfs version` available to the account running Furnish.

## Proxy differences

| | Caddy | nginx | Traefik | Envoy | HAProxy |
|---|---|---|---|---|---|
| Token without `email_verified` | accepted | accepted | accepted | accepted | refused, unless `ALLOWED_GROUPS` is set |
| `email_verified: "true"` (a string) | accepted | accepted | accepted | accepted | refused |
| Rate limit on git requests and reports, per | none | token | token | user | user |
| 32 MiB limit on `git-upload-pack` | counted | counted | counted | counted | declared length only |
| 1 MiB limit on reports | counted | counted | counted | counted | declared length; no length: 411 |

"Counted" means the proxy counts the bytes it receives. HAProxy streams bodies and checks only `Content-Length`; git sends `git-upload-pack` requests over 1 MiB without one, and HAProxy lets those through without a limit. The limit per token in nginx and Traefik applies to one sign-in: a person who signs in again starts with a new allowance.

## Limits

- A token stays valid until it expires: the proxy checks it without asking the identity provider, so revoking a session doesn't stop it. Keep the ID token lifetime short.
- Everyone who passes the checks gets every collection of the address. `ALLOWED_EMAIL_DOMAINS` and `ALLOWED_GROUPS` decide who that is.
- `ALLOWED_EMAIL_DOMAINS` only restricts if the identity provider vouches for the e-mail address with `email_verified: true` in the ID token. Keycloak, Google and Okta do. oauth2-proxy refuses a token with `email_verified: false` but accepts one without the claim, and has no setting to require it; HAProxy requires it unless `ALLOWED_GROUPS` is set.
- Microsoft Entra ID sends no `email_verified`, and its `email` claim is an address that can be set without proof that the user owns it. With Entra ID, set `ALLOWED_EMAIL_DOMAINS=*`, add the `groups` claim to the ID token and put the IDs of the groups that may sign in into `ALLOWED_GROUPS`. Turning on "Assignment required" for the enterprise application and assigning those groups also keeps everyone else from getting a token.
- Any token from the issuer with your client ID as audience counts, also one issued to another client whose `aud` lists `furnish`. That includes a token furnish got for another address: someone who runs a lookalike address (`furnish-acme.tld`) can name your issuer and client ID in its `furnish.json`, and people who add it sign in at your identity provider and send their token to the lookalike, which can use it at your address. furnish shows where people sign in before they do, and your policy can allow only your addresses (`"usersCanAdd": false` with `endpoints`), but the proxy can't tell such a token apart. Where your identity provider supports it, give each address its own client ID or audience, so a token for one address doesn't count at another.
- The rate limit protects GitLab from one user, not the proxy from a flood of requests without sign-in; those cost the proxy a token check each. Put a firewall or your provider's flood protection in front for that.
