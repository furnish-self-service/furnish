#!/bin/sh
# Test double for GitLab's response for the single object served by the client-side storage fixture.
set -eu
oid=$(printf 'Furnish LFS fixture\n' | sha256sum | cut -d' ' -f1)
if ! jq -e --arg oid "$oid" '.operation == "download" and .objects == [{oid: $oid, size: 20}]' >/dev/null; then
    printf 'Status: 422 Unprocessable Entity\r\n\r\n'
    exit 0
fi
printf 'Content-Type: application/vnd.git-lfs+json\r\n\r\n'
printf '{"transfer":"basic","objects":[{"oid":"%s","size":20,"actions":{"download":{"href":"http://127.0.0.1:8971/object?signature=test"}}}]}' "$oid"
