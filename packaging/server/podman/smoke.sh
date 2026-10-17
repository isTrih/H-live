#!/usr/bin/env bash
# Ephemeral Linux CI user only. Never consumes real rooms, keys or user data.
set -euo pipefail
[[ $(id -u) != 0 && ${H_LIVE_EPHEMERAL_CI:-} == 1 ]] || exit 1
cleanup() {
  result=$?
  if ((result != 0)); then journalctl --user -u h-live.service --no-pager -n 80 >&2 || true; fi
  systemctl --user stop h-live.service || true
}
trap cleanup EXIT
bash packaging/server/podman/install.sh
before=$(sha256sum "$HOME/.local/share/h-live/secrets/"*)
bash packaging/server/podman/install.sh
[[ $before == "$(sha256sum "$HOME/.local/share/h-live/secrets/"*)" ]]
systemctl --user start h-live.service
ready() {
  for ((i=0;i<60;i++)); do
    if curl -fsS http://localhost:8080/healthz > /dev/null; then return; fi
    sleep 1
  done
  journalctl --user -u h-live.service --no-pager -n 60 >&2
  return 1
}
ready
[[ $(podman ps --format '{{.Names}}') == h-live ]]
[[ $(podman exec h-live id -u) == 10001 ]]
[[ $(podman exec h-live sh -c 'command -v python || command -v python3 || true') == '' ]]
[[ $(curl -s -o /dev/null -w '%{http_code}' -X POST -H 'content-type: application/json' -d '{}' http://localhost:8080/api/v1/state) == 401 ]]
podman exec h-live sh -c '! touch /opt/h-live/write-test && ! touch /run/h-live-secrets/write-test' 2>/dev/null
token=$(cat "$HOME/.local/share/h-live/secrets/admin-token")
request() { printf 'header = "Authorization: Bearer %s"\n' "$token" | curl -fsS -K - -H 'content-type: application/json' -X POST -d "$2" "http://localhost:8080/api/v1/$1"; }
request state '{}' > /tmp/hlive-initial.json
jq -e '.anchors | length == 0' /tmp/hlive-initial.json > /dev/null
request anchors/import '{"values":"1"}' > /dev/null
systemctl --user stop h-live.service
journalctl --user -u h-live.service --no-pager | grep -q 'All monitoring workers stopped and archives flushed'
[[ $(systemctl --user show h-live.service --property=ExecMainStatus --value) == 0 ]]
systemctl --user start h-live.service
ready
request state '{}' > /tmp/hlive-restored.json
jq -e '(.anchors | length == 1) and (.anchors[0].active == false)' /tmp/hlive-restored.json > /dev/null
# Fail-closed recovery: never regenerate a lost key over an existing archive.
mv "$HOME/.local/share/h-live/secrets/master.key" "$HOME/master.key.saved"
if bash packaging/server/podman/install.sh > /dev/null 2>&1; then exit 1; fi
[[ ! -e $HOME/.local/share/h-live/secrets/master.key ]]
mv "$HOME/master.key.saved" "$HOME/.local/share/h-live/secrets/master.key"
# Kill only this disposable container; systemd must restart it automatically.
old_id=$(podman inspect h-live --format '{{.Id}}')
podman kill h-live > /dev/null
for ((i=0;i<60;i++)); do
  new_id=$(podman inspect h-live --format '{{.Id}}' 2>/dev/null || true)
  if [[ -n $new_id && $new_id != "$old_id" ]] && curl -fsS http://localhost:8080/healthz > /dev/null; then break; fi
  sleep 1
done
[[ -n $new_id && $new_id != "$old_id" ]]
ready
request state '{}' > /tmp/hlive-recovered.json
jq -e '(.anchors | length == 1) and (.anchors[0].active == false)' /tmp/hlive-recovered.json > /dev/null
unset token
printf '%s\n' 'PASS: rootless Quadlet, one container, uid10001, read-only mounts, auth, idempotent keys, lost-key refusal, persisted data, SIGTERM flush, automatic restart'
