#!/usr/bin/env bash
# Production-container smoke test: no real platform connections or user data.
set -euo pipefail
cleanup() { docker compose down --volumes --remove-orphans >/dev/null 2>&1 || true; }
trap cleanup EXIT
docker compose --profile tools run --rm init
docker compose --profile tools run --rm init
docker compose up -d --wait --wait-timeout 90 server
test "$(docker compose exec -T server id -u)" = 10001
test "$(docker compose exec -T server sh -c 'command -v python || command -v python3 || true')" = ""
curl -fsS http://localhost:8080/healthz | grep -q '"ok":true'
test "$(curl -s -o /dev/null -w '%{http_code}' -X POST -H 'content-type: application/json' -d '{}' http://localhost:8080/api/v1/state)" = 401
curl -fsS http://localhost:8080/ | grep -q '<html'
# The token remains in shell memory, never printed, archived or put in a URL.
token=$(docker compose --profile tools run -T --rm --entrypoint cat init /run/h-live-secrets/admin-token)
request() { printf 'header = "Authorization: Bearer %s"\n' "$token" | curl -fsS -K - -H 'content-type: application/json' -X POST -d "$2" "http://localhost:8080/api/v1/$1"; }
request state '{}' > /tmp/hlive-initial.json
node -e 'const s=require("/tmp/hlive-initial.json");if(s.anchors.length!==0)process.exit(1)'
request anchors/import '{"values":"1"}' > /dev/null
docker compose stop server
test "$(docker inspect "$(docker compose ps -aq server)" --format '{{.State.ExitCode}}')" = 0
docker compose logs --no-color server | grep -q 'All monitoring workers stopped and archives flushed'
docker compose up -d --wait --wait-timeout 90 server
request state '{}' > /tmp/hlive-restored.json
node -e 'const s=require("/tmp/hlive-restored.json");if(s.anchors.length!==1||s.anchors[0].active)process.exit(1)'
unset token
echo 'PASS: non-root, fresh empty data, authentication, persistence, SIGTERM flush, restart, no Python executable'
