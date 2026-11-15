# furnish endpoint with Caddy

Caddy (`caddy:2.11.9-alpine`) gets and renews a Let's Encrypt certificate by itself when
`FURNISH_TLS` is an e-mail address; with `/certs/tls.crt /certs/tls.key` it uses your own. It asks
oauth2-proxy about each token. The rest of the setup is in
[the endpoint setup](https://furnish-self-service.github.io/docs/alpha/endpoint.html).

## Start

In `packaging/endpoint/`:

```sh
docker compose -f docker-compose.yml -f caddy/compose.yml up -d
```

For a Let's Encrypt certificate, ports 80 and 443 must be reachable from the internet.

## Add a collection

1. Create a deploy token (`read_repository`) in the GitLab project.
2. Add a line to the site block in `caddy/Caddyfile` with the public name, the GitLab repository
   path and the name of the credentials variable:

   ```
   import collection apps /it/apps.git APPS_GITLAB_AUTH
   ```

3. Add `APPS_GITLAB_AUTH: ${APPS_GITLAB_AUTH:?set APPS_GITLAB_AUTH}` to the `proxy` service's
   `environment` in `caddy/compose.yml` and the value to `.env`.
4. Add `{"name": "apps", "url": "https://furnish.myorg.tld/git/apps.git"}` to `collections` in
   `furnish.json`.
5. Run the `up -d` command again.

If GitLab uses a certificate from a private CA, mount the CA into the container and add
`transport http { tls_trust_pool file /path/ca.pem }` to the `reverse_proxy` block.
