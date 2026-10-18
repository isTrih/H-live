#!/usr/bin/env bash
# Isolated CI only. Persistent application volumes are never pruned by run.sh.
set -euo pipefail
[[ ${H_LIVE_EPHEMERAL_CI:-} == 1 ]] || exit 1
export H_LIVE_CONTAINER_NAME="h-live-single-ci-$RANDOM"
export H_LIVE_DATA_VOLUME="$H_LIVE_CONTAINER_NAME-data"
export H_LIVE_SECRET_VOLUME="$H_LIVE_CONTAINER_NAME-secrets"
name=$H_LIVE_CONTAINER_NAME
cleanup() {
  result=$?
  if ((result != 0)); then docker logs --tail 80 "$name" >&2 || true; fi
  docker rm -f "$name" > /dev/null 2>&1 || true
  docker volume rm "$H_LIVE_DATA_VOLUME" "$H_LIVE_SECRET_VOLUME" > /dev/null 2>&1 || true
}
trap cleanup EXIT
bash packaging/server/docker-single/run.sh > /dev/null
ready() {
  for ((i=0;i<60;i++)); do
    if curl -fsS http://localhost:8080/healthz > /dev/null 2>&1; then return; fi
    sleep 1
  done
  return 1
}
ready
[[ $(docker ps --format '{{.Names}}') == "$name" ]]
[[ $(docker exec "$name" id -u) == 10001 ]]
docker exec "$name" sh -c 'test "$(cat /proc/1/comm)" = h-live-server'
docker exec "$name" sh -c '! touch /opt/h-live/write-test' 2>/dev/null
[[ $(docker exec "$name" sh -c 'command -v python || command -v python3 || true') == '' ]]
[[ $(curl -s -o /dev/null -w '%{http_code}' -X POST -H 'content-type: application/json' -d '{}' http://localhost:8080/api/v1/state) == 401 ]]
curl -fsS http://localhost:8080/ | grep -q '<html'
keys_before=$(docker exec "$name" sh -c 'sha256sum /run/h-live-secrets/master.key /run/h-live-secrets/admin-token')
token=$(docker exec "$name" cat /run/h-live-secrets/admin-token)
request() { printf 'header = "Authorization: Bearer %s"\n' "$token" | curl -fsS -K - -H 'content-type: application/json' -X POST -d "$2" "http://localhost:8080/api/v1/$1"; }
request state '{}' > /tmp/hlive-docker-initial.json
jq -e '.anchors | length == 0' /tmp/hlive-docker-initial.json > /dev/null
request anchors/import '{"values":"1"}' > /dev/null
docker exec "$name" sh -c 'test -s /var/lib/h-live/messages.sqlite3 && printf persistent > /var/lib/h-live/.docker-volume-probe'
docker stop --time 180 "$name" > /dev/null
[[ $(docker inspect "$name" --format '{{.State.ExitCode}}') == 0 ]]
docker logs "$name" | grep -q 'All monitoring workers stopped and archives flushed'
# Recreate the same single app container over the two existing volumes.
docker rm "$name" > /dev/null
bash packaging/server/docker-single/run.sh > /dev/null
ready
[[ $(docker exec "$name" cat /var/lib/h-live/.docker-volume-probe) == persistent ]]
docker exec "$name" test -s /var/lib/h-live/messages.sqlite3
[[ $keys_before == "$(docker exec "$name" sh -c 'sha256sum /run/h-live-secrets/master.key /run/h-live-secrets/admin-token')" ]]
request state '{}' > /tmp/hlive-docker-restored.json
jq -e '(.anchors | length == 1) and (.anchors[0].active == false)' /tmp/hlive-docker-restored.json > /dev/null
# Missing key must fail closed after recreation, instead of replacing a data key.
docker exec "$name" mv /run/h-live-secrets/master.key /run/h-live-secrets/master.key.saved
docker stop --time 180 "$name" > /dev/null
docker rm "$name" > /dev/null
bash packaging/server/docker-single/run.sh > /dev/null
for ((i=0;i<15;i++)); do
  if docker logs "$name" 2>&1 | grep -q 'Refusing regeneration'; then break; fi
  sleep 1
done
docker logs "$name" 2>&1 | grep -q 'Refusing regeneration'
docker update --restart no "$name" > /dev/null
docker stop --time 180 "$name" > /dev/null
unset token
echo 'PASS: Docker Engine, single app container, automatic same-container initialization, Rust PID1, uid10001, readonly root, auth, persistent SQLite/config, unchanged keys, graceful stop, lost-key refusal'
