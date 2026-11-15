# furnish endpoint with nginx

This variant runs `nginx:1.30.6-alpine` as the proxy instead of Caddy. It behaves like the Caddy
variant described in [the endpoint setup](https://furnish-self-service.github.io/docs/alpha/endpoint.html): same allowlist, same token check through
oauth2-proxy, same deploy token handling and the same `POST /events` forwarding to Vector.
The configuration is [`default.conf.template`](default.conf.template).

## Start

Put your certificate and key into `certs/` next to `docker-compose.yml` as `tls.crt` (with the
intermediate chain) and `tls.key`, or set `TLS_DIR` to another directory. nginx has no ACME
client, so `FURNISH_TLS` is not used. Then, from `packaging/endpoint/`:

```sh
docker compose -f docker-compose.yml -f nginx/compose.yml up -d
```

Plain HTTP requests for `FURNISH_HOST` are redirected to HTTPS. Requests for any other host
name get 404.

## Add a collection

A collection needs four lines in `default.conf.template` and one environment variable. For a
collection `tools` mapped to the GitLab repository `/it/tools.git`, add to the
`$furnish_gitlab_uri` map:

```nginx
"~^GET/git/tools\.git/info/refs\?service=git-upload-pack$" "/it/tools.git/info/refs?service=git-upload-pack";
"~^POST/git/tools\.git/git-upload-pack$"                   "/it/tools.git/git-upload-pack";
```

and to the `$furnish_gitlab_auth` map:

```nginx
"~^GET/git/tools\.git/info/refs\?service=git-upload-pack$" "Basic ${TOOLS_GITLAB_AUTH}";
"~^POST/git/tools\.git/git-upload-pack$"                   "Basic ${TOOLS_GITLAB_AUTH}";
```

Add `TOOLS_GITLAB_AUTH: ${TOOLS_GITLAB_AUTH:?set TOOLS_GITLAB_AUTH}` to `environment` in
`nginx/compose.yml`, set it in `.env`, add the collection to `furnish.json` and recreate the
container (`docker compose ... up -d`). The template is rendered when the container starts, so
a reload inside a running container does not pick up changes to it.

## Notes

The image's start script runs `envsubst` on the template and replaces every `${NAME}` for which
an environment variable `NAME` exists. Do not give the container environment variables whose
names match nginx variables used in the template.

If a credentials variable is unset inside the container, its `${NAME}` stays in the rendered
file and GitLab rejects the request, which the client sees as 502. `compose.yml` refuses to
start without the variables to prevent that.

For an `https` `GITLAB_URL`, nginx verifies GitLab's certificate against the image's CA bundle.
If GitLab uses a certificate from an internal CA, mount that CA and point
`proxy_ssl_trusted_certificate` at it.

The proxy resolves `oauth2-proxy`, `vector` and the GitLab host through Docker's DNS server
(`resolver 127.0.0.11`) at request time. If GitLab's name is only resolvable through another
DNS server, change the `resolver` line.

Git requests have no body size limit, as with Caddy. Reports are limited to 1 MiB.

Git requests pass through a second server inside the container on `127.0.0.1:8081`. That
server talks to GitLab and turns GitLab's 401 or 403 into 502. It exists because nginx keeps
GitLab's `WWW-Authenticate` header on an intercepted 401, and only the outer hop can remove it.
Nothing outside the container can reach port 8081.

The allowlist keys are regular expressions because plain `map` keys compare case-insensitively.
Escape `.` and `?` in new keys and keep the `^` and `$` anchors.

nginx itself rejects a request with two `Authorization` headers with 400. The HTTPS server
block for `FURNISH_HOST` turns that into 401. The other server blocks keep the 400. Either way,
nothing is proxied.
