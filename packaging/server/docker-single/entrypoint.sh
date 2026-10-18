#!/bin/sh
# Runs inside the sole application container as UID 10001; Rust becomes PID 1.
set -eu
umask 077
data=${H_LIVE_DATA_DIR:-/var/lib/h-live}
secrets=${H_LIVE_SECRETS_DIR:-/run/h-live-secrets}
if [ ! -f "$secrets/master.key" ] || [ ! -f "$secrets/admin-token" ]; then
  # A named Docker volume copies the image's .volume marker on first use.
  # Any other existing data requires the original keys, never regeneration.
  if [ -n "$(find "$data" -mindepth 1 -maxdepth 1 ! -name .volume -print -quit)" ]; then
    echo 'Existing data but missing secrets; restore the original master.key and admin-token. Refusing regeneration.' >&2
    exit 1
  fi
fi
h-live-server init-secrets "$secrets"
exec h-live-server serve
