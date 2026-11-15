# Sourced by run.sh for PROXY=haproxy, before the stack starts. HAProxy verifies ID tokens with
# the IdP's public key from a file, so start the mock IdP first and export its acme key (created
# at container start) with the same script admins use, run in the client container.
dc up -d --quiet-pull mock-oidc 2>/dev/null
JWT_KEY_FILE=$TLS_DIR/idp.pem; export JWT_KEY_FILE
i=0
until dc run --rm -T client sh -s -- http://mock-oidc:8080/acme \
    < "$root/packaging/endpoint/haproxy/jwks-to-pem.sh" > "$JWT_KEY_FILE" 2>/dev/null && [ -s "$JWT_KEY_FILE" ]; do
    i=$((i + 1)); [ $i -gt 30 ] && { echo "could not export the mock IdP's key" >&2; exit 1; }; sleep 1
done
chmod 644 "$JWT_KEY_FILE"
# HAProxy runs as uid 99, not root; mktemp -d made the certificate directory private to us.
chmod 755 "$TLS_DIR"
