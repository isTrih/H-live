#!/usr/bin/env bash
# Docker Engine only; no Compose, Podman, sidecar or initialization container.
set -euo pipefail
image=h-live-server:0.1.0
name=${H_LIVE_CONTAINER_NAME:-h-live}
data_volume=${H_LIVE_DATA_VOLUME:-h-live-data}
secret_volume=${H_LIVE_SECRET_VOLUME:-h-live-secrets}
origin=${H_LIVE_PUBLIC_ORIGIN:-http://localhost:8080}
port=${H_LIVE_PORT:-8080}
for value in "$name" "$data_volume" "$secret_volume"; do
  [[ $value =~ ^[a-zA-Z0-9][a-zA-Z0-9_.-]*$ ]] || { echo 'Invalid container/volume name.' >&2; exit 1; }
done
[[ $data_volume != "$secret_volume" ]] || { echo 'Use separate data and secrets volumes.' >&2; exit 1; }
if [[ ! $port =~ ^[0-9]+$ || ${#port} -gt 5 ]]; then echo 'Invalid port.' >&2; exit 1; fi
if ((10#$port < 1 || 10#$port > 65535)); then echo 'Invalid port.' >&2; exit 1; fi
docker image inspect "$image" > /dev/null
if docker container inspect "$name" > /dev/null 2>&1; then
  printf 'Container already exists. Use docker start %s, or stop/back up/remove it before recreating.\n' "$name" >&2
  exit 1
fi
source_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
bootstrap=$(cat "$source_dir/entrypoint.sh")
docker run --detach --name "$name" --restart unless-stopped --stop-timeout 180 \
  --publish "127.0.0.1:$port:8080" \
  --env "H_LIVE_PUBLIC_ORIGIN=$origin" \
  --mount "type=volume,source=$data_volume,target=/var/lib/h-live" \
  --mount "type=volume,source=$secret_volume,target=/run/h-live-secrets" \
  --read-only --tmpfs /tmp:size=128m,mode=1777 \
  --cap-drop ALL --security-opt no-new-privileges:true \
  --pids-limit 128 --memory 2g \
  --log-driver json-file --log-opt max-size=10m --log-opt max-file=3 \
  --entrypoint /bin/sh "$image" -c "$bootstrap"
