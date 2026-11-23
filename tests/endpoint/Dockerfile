# Test-only images for tests/endpoint/run.sh.
FROM alpine:3.24.4 AS base
RUN apk add --no-cache git

# Stand-in for the internal GitLab: smart-HTTP git (nginx + fcgiwrap + git-http-backend) behind
# HTTP Basic auth with the deploy token. Pushes are enabled here on purpose, so only the endpoint
# proxy stands between a client and a successful push.
FROM base AS gitlab
RUN apk add --no-cache nginx fcgiwrap git-daemon jq
RUN set -e; \
    git config --system safe.directory '*'; \
    git config --system init.defaultBranch main; \
    git config --system http.receivepack true; \
    for repo in catalog secret; do \
        git init -q /tmp/$repo; \
        echo "content of $repo" > /tmp/$repo/$repo.txt; \
        [ $repo = secret ] && echo "TOP-SECRET-MARKER-c0ffee" > /tmp/$repo/secret.txt; \
        git -C /tmp/$repo add .; \
        git -C /tmp/$repo -c user.name=test -c user.email=test@test commit -q -m "initial $repo"; \
        git clone -q --bare /tmp/$repo /srv/git/it/$repo.git; \
        rm -rf /tmp/$repo; \
    done; \
    chown -R nginx:nginx /srv/git; \
    install -d -o nginx /run/fcgiwrap
COPY gitlab/nginx.conf /etc/nginx/nginx.conf
COPY --chmod=755 gitlab/lfs-batch.sh /usr/local/libexec/lfs-batch.sh
COPY gitlab/start.sh /start.sh
CMD ["/bin/sh", "/start.sh"]

# The test client: git, curl, openssl (for PKCE).
FROM base AS client
RUN apk add --no-cache curl openssl ca-certificates git-lfs busybox-extras
