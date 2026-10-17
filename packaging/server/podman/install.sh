#!/usr/bin/env bash
# Host-side setup only: never starts an initialization container or prints keys.
set -euo pipefail
umask 077
die() { printf '%s\n' "$*" >&2; exit 1; }
[[ $(uname -s) == Linux && $(uname -m) == x86_64 ]] || die 'Requires Linux amd64.'
[[ $(id -u) != 0 ]] || die 'Run as the dedicated unprivileged deployment user, without sudo.'
command -v podman >/dev/null || die 'Install Podman >= 4.9 with Quadlet first.'
version=$(podman version --format '{{.Client.Version}}')
[[ $(printf '%s\n' 4.9.0 "$version" | sort -V | head -n 1) == 4.9.0 ]] || die 'Podman >= 4.9 required.'
[[ $(podman info --format '{{.Host.CgroupsVersion}}') == v2 ]] || die 'cgroup v2 required.'
[[ $(podman info --format '{{.Host.Security.Rootless}}') == true ]] || die 'Rootless Podman required.'
systemctl --user show-environment >/dev/null || die 'A systemd user session is required.'
[[ -z ${XDG_CONFIG_HOME:-} || $XDG_CONFIG_HOME == "$HOME/.config" ]] || die 'This kit requires the standard ~/.config directory.'
podman image exists localhost/h-live-server:0.1.0 || die 'Load the verified image, then tag docker.io/library/h-live-server:0.1.0 as localhost/h-live-server:0.1.0 (see PODMAN.md).'
source_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
data_root="$HOME/.local/share/h-live"
config_root="$HOME/.config/h-live"
quadlet_root="$HOME/.config/containers/systemd"
dropin_root="$HOME/.config/systemd/user/h-live.service.d"
# Refuse links/foreign ownership for all paths this installer writes.
check_path() {
  local path=$1
  while [[ $path != / ]]; do
    [[ ! -L $path ]] || die "Refusing symbolic link: $path"
    if [[ -e $path ]]; then
      [[ -O $path || $path == /home || $path == /var || $path == /var/lib ]] || die "Path is not owned by this user: $path"
    fi
    [[ $path == "$HOME" ]] && break
    path=$(dirname -- "$path")
  done
}
for path in "$data_root/data" "$data_root/secrets" "$config_root" "$quadlet_root" "$dropin_root"; do
  check_path "$path"
done
install -d -m 700 "$data_root" "$data_root/data" "$data_root/secrets" "$config_root" "$quadlet_root" "$dropin_root"
for name in master.key admin-token; do
  key="$data_root/secrets/$name"
  check_path "$key"
  if [[ ! -e $key ]]; then
    [[ -z $(find "$data_root/data" -mindepth 1 -print -quit) ]] || die "Existing data but missing $name: restore your original secrets; refusing regeneration."
    (set -o noclobber; head -c 32 /dev/urandom | base64 > "$key")
    sync -f "$key"
  fi
  [[ -f $key && $(base64 -d < "$key" | wc -c) == 32 ]] || die "Invalid secret: $name (not overwritten)."
  chmod 600 "$key"
done
env_file="$config_root/server.env"
check_path "$env_file"
if [[ ! -e $env_file ]]; then
  (set -o noclobber; printf '%s\n' 'H_LIVE_PUBLIC_ORIGIN=http://localhost:8080' > "$env_file")
fi
chmod 600 "$env_file"
unit="$quadlet_root/h-live.container"
check_path "$unit"
if [[ -e $unit ]]; then
  cmp -s "$source_dir/h-live.container" "$unit" || die 'Existing Quadlet differs: review it manually; not overwritten.'
else
  (set -o noclobber; cat "$source_dir/h-live.container" > "$unit")
fi
dropin="$dropin_root/10-graceful-stop.conf"
check_path "$dropin"
if [[ -e $dropin ]]; then
  cmp -s "$source_dir/10-graceful-stop.conf" "$dropin" || die 'Existing stop override differs: review it manually; not overwritten.'
else
  (set -o noclobber; cat "$source_dir/10-graceful-stop.conf" > "$dropin")
fi
systemctl --user daemon-reload
if [[ $(systemctl --user show h-live.service --property=LoadState --value) != loaded ]]; then
  if [[ -x /usr/lib/systemd/system-generators/podman-system-generator ]]; then
    QUADLET_UNIT_DIRS="$quadlet_root" /usr/lib/systemd/system-generators/podman-system-generator --user --dryrun >&2 || true
  fi
  die 'Quadlet generation failed; check your Podman/systemd installation.'
fi
printf '%s\n' 'Installed; existing secrets and configuration preserved. No container has been started.' \
  'Start: systemctl --user start h-live.service' \
  'Boot/logout persistence: ask your administrator to run loginctl enable-linger YOUR_DEPLOYMENT_USER' \
  'Read the admin token locally from ~/.local/share/h-live/secrets/admin-token (do not share it).'
