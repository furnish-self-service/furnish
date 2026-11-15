#!/bin/sh
set -eu
printf '%s:{PLAIN}%s\n' "$DEPLOY_USER" "$DEPLOY_TOKEN" > /etc/nginx/htpasswd
su -s /bin/sh nginx -c 'exec fcgiwrap -s unix:/run/fcgiwrap/sock' &
exec nginx -g 'daemon off;'
