# furnish endpoint

Let furnish users clone selected repositories from your internal GitLab, signed in with your
organization's OIDC login, without exposing GitLab or handing out GitLab accounts.

```
furnish ──HTTPS, "Authorization: Bearer <ID token>"──▶ Caddy ──▶ oauth2-proxy: is the token valid?
                                                        │
                                                        └──▶ GitLab, one fixed repository path,
                                                             "Authorization: Basic <deploy token>"
```

The user adds `furnish.myorg.tld` in furnish. furnish reads
`https://furnish.myorg.tld/.well-known/furnish.json`, signs the user in at your IdP
(authorization code with PKCE, public client) and runs git against the endpoint with the user's
ID token. Caddy asks oauth2-proxy to verify the token, then forwards the request to GitLab with a
read-only deploy token. GitLab stays internal: only the Caddy host needs to reach it.

## What the endpoint lets through

Each collection gets a fixed public path that maps to exactly one GitLab repository. For the
collection `acme` mapped to `/it/catalog.git`, only these two requests are proxied:

| Client request | Forwarded to GitLab as |
| --- | --- |
| `GET /git/acme.git/info/refs?service=git-upload-pack` | `GET /it/catalog.git/info/refs?service=git-upload-pack` |
| `POST /git/acme.git/git-upload-pack` | `POST /it/catalog.git/git-upload-pack` |

These are the only requests git needs to clone and fetch, with protocol v2 as well as v0.

- **Exact match on the raw request URI.** `..`, `%2e`, `%2f`, `//`, extra segments, extra query
  parameters, other methods, pushes (`git-receive-pack`), the dumb HTTP protocol and every other
  path get 404. No client path or query is ever passed on to GitLab.
- **Authentication before proxying.** Requests without `Authorization: Bearer <JWT>` get 401.
  oauth2-proxy checks signature, issuer, expiry, audience (= client ID), `email_verified` and,
  if you set it, the e-mail domain; failures get 401 or 403.
- **The deploy token stays on the server.** Caddy replaces the client's `Authorization` header
  with the deploy token and drops `Cookie`, `Private-Token`, `Job-Token` and `Deploy-Token`.
  GitLab's `Set-Cookie` and `WWW-Authenticate` are removed from responses. If GitLab rejects the
  deploy token, the client gets 502.
- **`/.well-known/furnish.json` needs no sign-in.**

## Files

| File | Purpose |
| --- | --- |
| [`Caddyfile`](Caddyfile) | TLS, the allowlist, authentication and the deploy token |
| [`docker-compose.yml`](docker-compose.yml) | Caddy and oauth2-proxy, with pinned image versions |
| [`furnish.json`](furnish.json) | Example discovery document |
| [`.env.example`](.env.example) | Settings to copy into `.env` |

Image versions: `caddy:2.11.9-alpine`, `quay.io/oauth2-proxy/oauth2-proxy:v7.15.6`.

## Set it up

You need a host with Docker Compose that can reach GitLab, and a DNS name such as
`furnish.myorg.tld` pointing to it. Ports 80 and 443 must be reachable from your users, and from
Let's Encrypt if you use it.

1. **Register furnish at your IdP** as a *public* client (no client secret), with:
   - client ID `furnish` (or another; then set `OIDC_CLIENT_ID` and `clientId` to match),
   - grant type authorization code with PKCE (`S256`),
   - redirect URI `http://127.0.0.1/callback` on any port. furnish listens on a random loopback
     port (RFC 8252, section 7.3). Keycloak: valid redirect URI `http://127.0.0.1/*`. Microsoft
     Entra ID: platform "Mobile and desktop applications" with `http://127.0.0.1/callback`
     (Entra ignores the port for loopback addresses),
   - scopes `openid email`. oauth2-proxy rejects ID tokens with `email_verified: false`, and with
     `ALLOWED_EMAIL_DOMAINS` set, tokens without a matching `email`. The token's audience must be
     the client ID.

2. **Create a GitLab project deploy token** in the project behind the collection: *Settings →
   Repository → Deploy tokens*, scope `read_repository` only. Note the username
   (`gitlab+deploy-token-<n>`) and the token.

3. **Configure.** Copy `.env.example` to `.env`, fill it in and make it readable by admins only:

   ```sh
   cp .env.example .env && chmod 600 .env
   printf '%s' 'gitlab+deploy-token-42:gldt-xxxxxxxxxxxxxxxxxxxx' | base64 -w0   # → ACME_GITLAB_AUTH
   openssl rand -base64 32 | tr -- '+/' '-_'                                    # → OAUTH2_PROXY_COOKIE_SECRET
   ```

   | Variable | Meaning |
   | --- | --- |
   | `FURNISH_HOST` | Public host name, such as `furnish.myorg.tld` |
   | `FURNISH_TLS` | ACME contact e-mail for a Let's Encrypt certificate, or `internal` for Caddy's own CA |
   | `GITLAB_URL` | Internal GitLab base URL, such as `https://gitlab.myorg.internal` |
   | `ACME_GITLAB_AUTH` | Base64 of `deploy-token-username:deploy-token` for the `acme` collection |
   | `OIDC_ISSUER_URL` | Issuer of the ID tokens, exactly as in the token's `iss` claim |
   | `OIDC_CLIENT_ID` | Client ID from step 1; default `furnish` |
   | `OAUTH2_PROXY_COOKIE_SECRET` | 32 random bytes. oauth2-proxy requires it; no cookie is ever issued |
   | `ALLOWED_EMAIL_DOMAINS` | Comma-separated e-mail domains to accept; default `*` (every user of the IdP) |

4. **Edit `furnish.json`**: your name, the issuer, the client ID and one entry per collection.

5. **Start** and check:

   ```sh
   docker compose up -d
   curl https://furnish.myorg.tld/.well-known/furnish.json
   curl -i https://furnish.myorg.tld/git/acme.git/info/refs?service=git-upload-pack   # 401
   ```

If GitLab uses a certificate from a private CA, mount the CA into the Caddy container and add
`transport http { tls_trust_pool file /path/ca.pem }` to the `reverse_proxy` block in the Caddyfile.

## Add a collection

1. Create a deploy token (`read_repository`) in the GitLab project.
2. Add one line to the site block in the `Caddyfile`, with the public name, the GitLab repository
   path and the name of the credentials variable:

   ```
   import collection apps /it/apps.git APPS_GITLAB_AUTH
   ```

3. Add `APPS_GITLAB_AUTH: ${APPS_GITLAB_AUTH:?}` to the `caddy` service's `environment` in
   `docker-compose.yml` and the value to `.env`.
4. Add `{"name": "apps", "url": "https://furnish.myorg.tld/git/apps.git"}` to `collections` in
   `furnish.json`.
5. `docker compose up -d` (recreates Caddy with the new variable).

Use one deploy token per project. A group deploy token also works, because the endpoint only ever
requests the one path in the `import collection` line.

## Limits to know about

- **Tokens cannot be revoked at the endpoint.** oauth2-proxy verifies the JWT locally; a stolen
  or revoked user's ID token keeps working until it expires. Keep the IdP's ID token lifetime short.
- **Every user who passes the checks sees every collection.** There is no per-collection access
  control. Restrict users with `ALLOWED_EMAIL_DOMAINS`, or with oauth2-proxy's
  `OAUTH2_PROXY_ALLOWED_GROUPS` if your IdP puts groups into the ID token.
- **Any JWT from the issuer with the right audience is accepted**, including tokens issued to
  other clients whose `aud` lists `furnish`; `azp` is not checked.

## Test

[`tests/endpoint/run.sh`](../../tests/endpoint/run.sh) runs this Caddyfile and compose file
against a fake IdP and a fake GitLab, and checks clone, fetch, rejected tokens, rejected pushes,
the allowlist, a rejected deploy token, and that the deploy token never reaches the client.
