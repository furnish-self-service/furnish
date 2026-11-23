# furnish endpoint with HAProxy

This variant runs `haproxy:3.2.26-alpine` as the proxy instead of Caddy. It lets through the same
requests as the Caddy variant described in [the endpoint setup](https://furnish-self-service.github.io/docs/alpha/endpoint.html), with the same deploy
token handling and the same `POST /v1/logs` forwarding to the OTLP receiver. The configuration is
[`haproxy.cfg`](haproxy.cfg).

The difference is the token check. HAProxy cannot ask oauth2-proxy about a request, so it
verifies the ID token itself with its built-in JWT converters: RS256 signature with the IdP's
public key, `iss` equal to `OIDC_ISSUER_URL`, `aud` equal to (or, for an array, containing)
`OIDC_CLIENT_ID`, `exp` in the future, `nbf` (if present) at most 5 minutes ahead, the `email`
domain in `ALLOWED_EMAIL_DOMAINS` unless that is `*`, and either `email_verified` true or, if
`ALLOWED_GROUPS` is set, one of those groups in the `groups` claim.

HAProxy is stricter about `email_verified` than oauth2-proxy, which only refuses
`email_verified: false`. Without `ALLOWED_GROUPS`, HAProxy refuses a token without the claim or
with the string `"true"`, because the e-mail domain is then all that decides who signs in, and an
address the identity provider doesn't vouch for proves nothing. With `ALLOWED_GROUPS`, the group
decides, and a token without the claim passes, as with Microsoft Entra ID, which never sends
it. oauth2-proxy from `docker-compose.yml` still starts
but receives no requests. It still needs `OAUTH2_PROXY_COOKIE_SECRET` set, because Compose
checks the whole file.

## Start

Put your certificate and key into `certs/` next to `docker-compose.yml` as `tls.crt` (with the
intermediate chain) and `tls.key`, or set `TLS_DIR` to another directory. HAProxy runs as uid 99,
so the directory and both files must be readable by that user. `FURNISH_TLS` is not used: HAProxy
3.2's ACME client is experimental, so this variant takes your own certificate.

Export the IdP's signing key (next section), then, from `packaging/endpoint/`:

```sh
docker compose -f docker-compose.yml -f haproxy/compose.yml up -d
```

Plain HTTP requests are redirected to HTTPS. Requests for any host name other than
`FURNISH_HOST` get 404. [`start.sh`](start.sh) is the container's entrypoint: it splits
`GITLAB_URL` and `ALLOWED_EMAIL_DOMAINS` into the variables `haproxy.cfg` uses, because HAProxy
can expand environment variables but not take them apart.

If GitLab uses `https://`, HAProxy checks its certificate against the system CAs in the image. For
a private CA, mount the CA file into the container and replace `ca-file @system-ca` in the
`server gitlab` line with its path.

## The IdP's signing key

HAProxy reads the public key from `haproxy/idp.pem` (or the file named by `JWT_KEY_FILE`) when it
starts and when it reloads. It cannot fetch the IdP's JWKS itself. Export the key with
[`jwks-to-pem.sh`](jwks-to-pem.sh), which needs `curl` and `openssl`:

```sh
haproxy/jwks-to-pem.sh "$OIDC_ISSUER_URL" > haproxy/idp.pem
```

The script reads `jwks_uri` from the issuer's `/.well-known/openid-configuration` and converts
the RSA signing key to PEM. If the JWKS holds more than one RSA signing key, it lists their key
IDs and stops; pass the one your IdP signs ID tokens with as second argument. That is the `kid`
in the header of a current ID token (the first dot-separated part, base64url-encoded JSON).

When the IdP rotates its signing key, every token signed with the new key gets 401 until you
export the new key and reload HAProxy:

```sh
haproxy/jwks-to-pem.sh "$OIDC_ISSUER_URL" <new kid> > haproxy/idp.pem
docker compose -f docker-compose.yml -f haproxy/compose.yml kill -s HUP proxy
```

The reload keeps open connections. If the new file is not a valid key, the new configuration
fails to load and the running process keeps the old key; `docker compose logs proxy` shows the
error. Keycloak rotates only when an admin adds a key, so plan the export for that moment. IdPs
that rotate on their own schedule, such as Microsoft Entra ID, publish the next key in the JWKS
ahead of time; check the JWKS regularly (for example from cron) and switch when ID tokens start
using the new `kid`.

## Add a collection

A collection needs three lines in `haproxy.cfg` and one environment variable. For a collection
`tools` mapped to the GitLab repository `/it/tools.git`, add next to the `acme` lines:

```
http-request set-var(txn.upstream) str(/it/tools.git/info/refs?service=git-upload-pack) if { method GET } { pathq -m str /git/tools.git/info/refs?service=git-upload-pack }
http-request set-var(txn.upstream) str(/it/tools.git/git-upload-pack) if { method POST } { pathq -m str /git/tools.git/git-upload-pack }
http-request set-var(txn.upstream) str(/it/tools.git/info/lfs/objects/batch) if { method POST } { pathq -m str /git/tools.git/info/lfs/objects/batch }
http-request set-var(txn.gitlab_auth) str("${TOOLS_GITLAB_AUTH}") if { var(txn.upstream) -m found } { pathq -m beg /git/tools.git/ }
```

Add `TOOLS_GITLAB_AUTH: ${TOOLS_GITLAB_AUTH:?set TOOLS_GITLAB_AUTH}` to `environment` in
`haproxy/compose.yml` and the value (base64 of `deploy-token-username:deploy-token`) to `.env`,
add the collection to `furnish.json`, and run the `up -d` command again. HAProxy reads
`furnish.json` only at start and reload, so after editing only that file, send `HUP` as above.

## Limits

These come on top of the limits in [the endpoint setup](https://furnish-self-service.github.io/docs/alpha/endpoint.html).

HAProxy knows one signing key at a time and does not follow the JWKS, so an unannounced key
rotation at the IdP locks everyone out until you export the new key. There is no fallback to a
second key during the switch.

Tokens cannot be revoked at the endpoint, as with oauth2-proxy: a valid ID token works until its
`exp`. Keep the IdP's ID token lifetime short. `iat` and `azp` are not checked. Like oauth2-proxy,
HAProxy allows no clock skew on `exp` and 5 minutes on `nbf`.

An `aud` array is matched in the JSON text HAProxy's `json_query` returns, which keeps the
token's spacing. If an IdP wrote `["other", "furnish"]` with a space after the comma, the client
ID would only be found as the first element; the IdPs we know write compact JSON. HAProxy also
treats `1` and `"1"` like `true` for `email_verified`. The `groups` claim is matched the same
way: as a string, or as an element of the array's JSON text. Tokens that fail any check get 401
(signature, issuer, audience, expiry, `nbf`) or 403 (e-mail, groups).

The 1 MiB limit on reports is checked against `Content-Length`, because HAProxy streams request
bodies. `POST /v1/logs` without exactly one `Content-Length` header gets 411; furnish always sends
one. HAProxy itself rejects a body longer than its declared length. The 32 MiB limit on
`git-upload-pack` is checked the same way, but git sends requests over 1 MiB without a length
(chunked), and HAProxy has no native way to count those bytes, so they pass without a limit.

Git requests are limited per user: a stick table counts each e-mail address's requests, and more
than 80 in 10 seconds get 429.
