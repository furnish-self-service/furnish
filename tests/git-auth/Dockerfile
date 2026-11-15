# For tests/git-auth.sh and tests/checkout.sh. A git server: HTTPS-style smart HTTP behind Basic auth (Apache), and SSH.
FROM debian:trixie
RUN apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \
        apache2 apache2-utils openssh-server git git-lfs gnupg ca-certificates procps \
    && rm -rf /var/lib/apt/lists/*
RUN a2enmod -q cgi alias env auth_basic >/dev/null \
    && htpasswd -bc /etc/apache2/git.htpasswd alice s3cret-token \
    && printf '%s\n' \
      'SetEnv GIT_PROJECT_ROOT /srv/git' 'SetEnv GIT_HTTP_EXPORT_ALL' \
      'ScriptAlias /git/ /usr/lib/git-core/git-http-backend/' \
      '<Location /git/>' '  AuthType Basic' '  AuthName git' '  AuthUserFile /etc/apache2/git.htpasswd' '  Require valid-user' '</Location>' \
      > /etc/apache2/conf-enabled/git.conf \
    && useradd -m -s /usr/bin/git-shell git && mkdir -p /run/sshd /home/git/.ssh
