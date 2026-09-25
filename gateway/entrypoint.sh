#!/bin/sh
set -eu
umask 077
test -s /config/authorized_keys
if [ ! -f /config/ssh_host_ed25519_key ]; then
  ssh-keygen -q -t ed25519 -N '' -f /config/ssh_host_ed25519_key
fi
chmod 600 /config/ssh_host_ed25519_key
cp /config/authorized_keys /etc/ssh/homeproxy_authorized_keys
chmod 644 /etc/ssh/homeproxy_authorized_keys
/usr/sbin/sshd -D -e &
sshd_pid=$!
socat TCP-LISTEN:18081,fork,reuseaddr TCP:127.0.0.1:18080 &
relay_pid=$!
trap 'kill "$sshd_pid" "$relay_pid" 2>/dev/null || true' EXIT INT TERM
wait -n
