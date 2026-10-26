# fine-ill-do-it-myself (fidim)

A local web dev stack manager for Linux, in the spirit of Laragon. It wires up
Caddy + PHP-FPM + (optional system) MariaDB, one folder per project under
`~/Sites`, reachable at `*.test` domains with real HTTPS.

## What you get

- `~/Sites/<name>` folders become `https://<name>.test` vhosts automatically
- A local certificate authority (one root, one leaf per project) generated with
  openssl — real TLS, no `tls internal` surprises
- php-fpm pool + caddy config auto-generated per start
- Download manager with retries + sha256 pinning (tarballs in
  `sources.toml`), so caddy and static php binaries are reproducible
- Optional MariaDB via the system package (`mariadbd` found on PATH)
- A TUI (`fidim` default command), status CLI, unix-socket daemon

## Install

```sh
cargo build --release
cargo install --path crates/fidim
```

Bootstrap binaries from the manifest (defaults ship inside the binary):

```sh
fidim install caddy
fidim php install 8.4
fidim php use 8.4
```

## Day to day

```sh
fidim start     # caddy + php-fpm (+ mariadb if available)
fidim status
fidim projects
fidim hosts     # prints the missing /etc/hosts lines (needs root to apply)
fidim           # TUI
fidim trust     # prints the root steps to trust the local CA
fidim web use apache   # pick the web server: caddy (default), apache, nginx
fidim web show         # print the server currently selected
fidim daemon &  # unix socket at ~/.local/share/fine-ill-do-it-myself/data/fidim.sock
```

Inside `~/.local/share/fine-ill-do-it-myself/etc/keys.toml` you can remap TUI
keys: `quit = z`, `restart = a` (defaults q/r).

Non-standard ports (no root needed):

```sh
FIDIM_HTTP_PORT=8080 FIDIM_HTTPS_PORT=8443 fidim start
curl --resolve blog.test:8443:127.0.0.1 https://blog.test:8443/
```

To trust the local CA system-wide (**requires root**):

```sh
sudo cp ~/.local/share/fine-ill-do-it-myself/etc/ca.crt \
  /usr/local/share/ca-certificates/fidim-ca.crt
sudo update-ca-certificates
# browsers also need NSS: apt install libnss3-tools
```

## Where things live

```
~/.local/share/fine-ill-do-it-myself/
  bin/        managed binaries (caddy, php-fpm-<ver>)
  etc/        generated configs (Caddyfile, php-fpm conf, my.cnf, certs/)
  run/        pid files + sockets
  logs/       service logs
  lock.toml   what is installed + sha256 pins
  sources.toml download manifest (copy of default_manifest.toml)
```

## Architecture

```
crates/core            logic: paths, manifest/lock, installer, proc runner,
                       supervisor (Env, PortPlan), orchestrator (start/stop/
                       status/hosts/certs), caddyfile + fpm + mariadb config
crates/fidim           clap CLI + ratatui TUI + unix socket daemon
```

Everything network/process-dependent lives behind small traits (`Fetcher`,
`Runner`) so the whole suite runs offline — 58 tests, no daemon needed.

## Development

- TDD: red → green, commit per step (Conventional Commits, tags per stage)
- `cargo fmt && cargo clippy --workspace --all-targets -- -D warnings && cargo test`
- pre-commit hook enforces fmt/clippy/test
