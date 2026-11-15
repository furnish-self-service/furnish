# furnish endpoint with Traefik

This variant runs the endpoint described in [the endpoint setup](https://furnish-self-service.github.io/docs/alpha/endpoint.html) with Traefik
(`traefik:v3.7.17`) instead of Caddy. oauth2-proxy and Vector come from the shared
`docker-compose.yml`. Traefik cannot serve files or copy a header under another name, so a small
busybox httpd container (`busybox:1.37.0-musl`, service `files`) serves `furnish.json` and
answers Traefik with the user name it passes to Vector. That container is not published.

| File | Purpose |
| --- | --- |
| `compose.yml` | The `proxy` (Traefik) and `files` (busybox httpd) services |
| `traefik.yml` | Static configuration: entry points, path handling, file provider |
| `dynamic.yml` | Routers, middlewares, services and the TLS certificate; one line per collection |
| `files/httpd.conf`, `files/user.sh` | Configuration and CGI script of the `files` container |

## Start

Set up `.env` as described in the endpoint setup. This variant reads `FURNISH_HOST`, `GITLAB_URL`
and one credentials variable per collection. It uses your own certificate: put `tls.crt` (with
the intermediate certificates) and `tls.key` into `./certs`, or set `TLS_DIR` to another
directory. `FURNISH_TLS` is not used. Then, in `packaging/endpoint/`:

```sh
docker compose -f docker-compose.yml -f traefik/compose.yml up -d
curl https://furnish.myorg.tld/.well-known/furnish.json
curl -i https://furnish.myorg.tld/git/acme.git/info/refs?service=git-upload-pack   # 401
```

Traefik renders `dynamic.yml` as a Go template when it starts. After changing it or a variable,
run the same `up -d` again, or `docker compose -f docker-compose.yml -f traefik/compose.yml
restart proxy` if only `dynamic.yml` changed.

## Add a collection

1. Create a deploy token (`read_repository`) in the GitLab project.
2. Add one line next to the `acme` line at the top of `dynamic.yml`, with the public name, the
   GitLab repository path and the name of the credentials variable:

   ```
   {{- $collections = append $collections (list "apps" "/it/apps.git" "APPS_GITLAB_AUTH") }}
   ```

3. Add `APPS_GITLAB_AUTH: ${APPS_GITLAB_AUTH:?set APPS_GITLAB_AUTH}` to the `proxy` service's
   `environment` in `traefik/compose.yml` and the value to `.env`.
4. Add `{"name": "apps", "url": "https://furnish.myorg.tld/git/apps.git"}` to `collections` in
   `furnish.json`.
5. Run `docker compose -f docker-compose.yml -f traefik/compose.yml up -d`.

## How the checks map to Traefik

Routers match the host, method, decoded path and the `service` query parameter. A request with
`Authorization: Bearer <token>` goes to a router that asks oauth2-proxy and then GitLab; the same
request without it goes to a router that answers 401. Requests that match no router get 404.

Traefik's routers do not see the request URI as sent, so every proxied request also passes a
`redirectRegex` middleware that matches on the request URI and flags any `%`, and any query
beyond `service=git-upload-pack` (none at all on the other requests). An `errors` middleware
turns that flag into a 404. The entry point keeps `..`, `.` and `//` in the path
(`sanitizePath: false`), so those never match a router.

Traefik has no handler that answers with a fixed status. The 401 and 502 answers come from the
`errors` middleware rewriting the status of the built-in `noop@internal` service or of GitLab's
answer; they have an empty body.

## Limits of this variant

- Traefik decodes percent-encoded unreserved characters (letters, digits, `-`, `.`, `_`, `~`) in
  the path before any router or middleware sees the request, and this cannot be switched off. So
  `GET /git/acme.git/info/r%65fs?service=git-upload-pack` is treated like
  `.../info/refs?service=git-upload-pack` and proxied, unlike in the Caddy variant. RFC 3986
  treats both forms as the same URI, and GitLab still only receives the fixed upstream URI. An
  encoded `.` cannot reach another repository either: `%2e%2e` becomes `..`, which matches no
  router.
- A redirect from GitLab (301 or 308) is answered with 404, because the same `errors` middleware
  catches the URI check's flag. Git does not need redirects from a fixed repository path.
- ACME (Let's Encrypt) is not configured. To use it, add a `certificatesResolvers` section to
  `traefik.yml`, set `http.tls.certResolver` on the `websecure` entry point, mount a volume for
  `acme.json`, and remove the `tls` section from `dynamic.yml`.
