# HomeProxy

Use a home Mac as the egress connection for remote browser workers. HomeProxy
provides a Rust CLI, a Mac desktop controller, and an SSH/SOCKS gateway for Linux.
The connector opens an outbound SSH connection. Workers connect to the gateway's
private SOCKS port; websites see the home connection's public address.

## Install the connector

Build the CLI with `cargo build --release -j4 -p homeproxy-cli`. Copy the resulting
`homeproxy` executable into your PATH. The installed service copies this executable
to `~/.config/homeproxy/homeproxy`; no private scripts are needed.

Create `~/.config/homeproxy/config.json` with mode 600:

```json
{
  "gateway": "gateway.example.com",
  "ssh_port": 2222,
  "user": "proxy",
  "identity_file": "/Users/you/.ssh/homeproxy",
  "known_hosts_file": "/Users/you/.ssh/homeproxy_known_hosts",
  "remote_port": 18080,
  "verify_port": 18083,
  "relay_port": 18082,
  "mode": "direct",
  "upstream": null
}
```

Install the gateway public host key in `known_hosts_file`, verifying its fingerprint
through your gateway administrator. Unknown or changed keys are rejected. Install
your connector's public SSH key in the gateway authorized-keys file.

Run `homeproxy install`, then `homeproxy verify`. Other commands: `status --json`,
`on`, `off`, `mode direct`, and `logs`. launchd supervises the connection and restarts
failed tunnels with a ten-second throttle. `HOMEPROXY_CONFIG_DIR` selects an isolated
configuration directory.

For an upstream SOCKS service, set `upstream` to an object with `host`, `port`,
`username`, and `password`, then select `homeproxy mode upstream`. The `nordvpn`
mode uses the same mechanism with credentials and endpoint supplied by you. Only
proxy traffic takes this route. Credentials never appear in SSH arguments.

Build the Mac controller with `pnpm install --frozen-lockfile` and `pnpm tauri build`.
Install the CLI service before opening the controller.

## Gateway

Build with `docker build -t homeproxy-gateway gateway`. Mount a private writable
`/config` directory containing `authorized_keys`; retain it across restarts so the
host key stays stable. Publish TCP 2222 for the connector. TCP 18081 is an
unauthenticated SOCKS endpoint and must be reachable only by authorized workers,
using a private network and firewall or Kubernetes NetworkPolicy.

Configure a browser profile with `socks5://gateway:18081`. A missing home connection
causes connection failure. No component falls back to direct egress.

The connector's local verification port is bound to loopback. `verify` requests
`https://example.com/` through that port, the SSH gateway, and the home/upstream
route without printing the response.

## Development

Run `cargo test -j4 --workspace`, `cargo clippy -j4 --workspace --all-targets -- -D warnings`,
`cargo fmt --all -- --check`, and `pnpm build`. Tests use local fixtures.
