# CGI script, asked by Traefik's events-user forwardAuth (see dynamic.yml) after oauth2-proxy has
# verified the token: answers with the verified e-mail under the name Vector reads.
# The value comes from an HTTP header, so it can't contain a line break.
printf 'Content-Type: text/plain\r\nX-Furnish-User: %s\r\n\r\n' "$HTTP_X_AUTH_REQUEST_EMAIL"
