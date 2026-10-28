# fine-ill-do-it-myself (fidim)

A local web dev stack manager for Linux, in the spirit of Laragon. It wires up
Caddy + PHP-FPM + (optional system) MariaDB, one folder per project under
`~/Sites`, reachable at `*.test` domains with real HTTPS.

## What you get

- `~/Sites/<name>` folders become `https://<name>.test` vhosts automatically
- A local certificate authority (one root, one leaf per project) generated with
  openssl — real TLS, no `tls internal` surprises
- php-fpm pool + caddy config auto-generated per start
- Download manager with sha256 verification for managed binaries and runtimes
- Optional MariaDB 11.4.13 Linux x86_64 runtime, downloaded with curl into the
  project data root; a system `mariadbd` on `PATH` remains supported
- phpMyAdmin 5.2.3 as a managed site (`https://phpmyadmin.test`), installed on
  demand and pointed at the fidim-managed MariaDB
- A TUI (`fidim` default command), status CLI, unix-socket daemon

## Install

```sh
cargo build --release
cargo install --path crates/fidim
```

Bootstrap binaries from the manifest (defaults ship inside the binary):

```sh
fidim install caddy
fidim install mariadb   # optional, portable Linux x86_64 runtime (downloaded with curl)
fidim install phpmyadmin   # optional: served at https://phpmyadmin.test
fidim php install 8.4
fidim php use 8.4
```

Language runtimes are managed per version:

```sh
fidim node install 24       # available: 20, 22, 24, 26
fidim node use 24
fidim python install 3.13   # available: 3.11, 3.12, 3.13, 3.14, 3.15
fidim go install 1.27       # available: 1.22, 1.23, 1.26, 1.27
fidim java install 25       # available: 17, 21, 25
fidim node list             # installed versions (plus `system` when found)
```

Each runtime is extracted under the data root (`runtimes/<lang>/<major>/`) with
its binaries symlinked into `bin/<lang>/`; `fidim <lang> use <version>` switches
the active `bin/<lang>/<lang>` symlink. Checksums are pinned in the manifest.
Run `fidim env --install` after selecting a version so `node`, `python3`, `go`,
`java`, ... resolve to the managed runtime from any directory.

## Day to day

```sh
fidim start     # caddy + php-fpm (+ mariadb if available)
fidim status
fidim projects
fidim dns status    # is seamless *.test resolution installed?
fidim dns install   # one-time setup; prints the plan and the sudo commands
fidim               # TUI
fidim trust     # prints the root steps to trust the local CA
fidim web use apache   # pick the web server: caddy (default), apache, nginx, lighttpd
fidim web show         # print the server currently selected
fidim ports show       # active HTTP/HTTPS/FPM/MariaDB ports
fidim ports set http 8080   # override ports in etc/ports.toml
fidim daemon &  # unix socket at ~/.local/share/fine-ill-do-it-myself/data/fidim.sock
```

**Use `mysql`/`mariadb`/`php`/`node`/`python`/`go`/`java` straight from your
terminal** — two options:

```sh
eval "$(fidim env)"     # per shell, inside this data root
fidim env --install     # everywhere: writes wrappers to ~/.local/bin
```

`fidim env` prepends the managed client dirs to `PATH` (mariadb, php, node,
python, go, java) and points `MYSQL_UNIX_PORT`/`MYSQL_TCP_PORT` at this data
root, so `mariadb -u root` (or the `mysql` alias) talks to the fidim-managed
server.

`fidim env --install` writes tiny `mysql`, `mariadb`, `php`, `node`, `npm`,
`python3`, `pip3`, `go`, `java`, `javac`, ... wrappers into `~/.local/bin`
(override with `$FIDIM_SHIM_DIR`). They follow the active
`fidim <lang> use <version>` selection and point at this data root, so they work
from any directory; re-run it after moving the project or installing a new
language. `~/.local/bin` must be on your `PATH`.

## Seamless `*.test` resolution

No `/etc/hosts` editing per project. `fidim` ships a wildcard DNS responder: it
answers every `A` query under `.test` with `127.0.0.1` and everything else with
NXDOMAIN, so a new folder in `~/Sites` is reachable as `http://<folder>.test`
the moment it exists.

Install it once (root is used exactly once, and only if you pass `--yes`):

```sh
fidim dns install        # prints what will change and the exact sudo commands
fidim dns install --yes  # applies it
fidim dns status         # drop-in, user unit and service state
```

What gets written:

- `/etc/systemd/resolved.conf.d/90-fidim.conf` — `DNS=127.0.0.1:15353` plus
  `Domains=~test`, so **only** `.test` is sent to fidim; every other name keeps
  using your normal resolver.
- `~/.config/systemd/user/fidim-dns.service` — a **user** unit running
  `fidim dns serve --port 15353`. It starts with your session; no system
  service, no daemon running as root.

Port `15353` is unprivileged and deliberately not mDNS `5353`. Check the
resolver directly at any time:

```sh
fidim dns serve --port 15353   # foreground, for debugging
dig @127.0.0.1 -p 15353 blog.test A +short   # -> 127.0.0.1
```

Undo everything with `fidim dns uninstall --yes`.

If `systemd-resolved` is not available (or you prefer the old way), fall back to
`/etc/hosts` entries generated from your projects:

```sh
fidim hosts            # appends the missing lines via sudo
fidim hosts --dry-run  # print them, change nothing
```

**PHP drop-in**: drop an extracted PHP tree (containing `php-fpm`) under
`~/.local/share/fine-ill-do-it-myself/php/<version>/` and `fidim php list`
picks it up automatically (`fidim php use <version>` selects it).

**phpMyAdmin**: `fidim install phpmyadmin` extracts phpMyAdmin 5.2.3 under
`runtimes/phpmyadmin/` and links it into `~/Sites/phpmyadmin`, so the usual
vhost/certificate/hosts generation serves it at
`https://phpmyadmin.test` (plus the HTTP variant). Its `config.inc.php` points
at the fidim-managed MariaDB socket and uses the normal login form: user
`root`, empty password. With `fidim dns install` it is reachable with no extra
setup; otherwise add the entry once via `fidim hosts`. The symlink is only
replaced when it is a fidim symlink, never when a real project folder has that
name.

Inside `~/.local/share/fine-ill-do-it-myself/etc/keys.toml` you can remap TUI
keys (defaults q/r/s/x/a):

```toml
quit = z
restart = t      # restart service under the cursor
start = b        # start service under the cursor
stop = n         # stop service under the cursor
restart_all = g  # stop+start everything
```

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

Inside a project (a directory with `.git`, `Cargo.toml`, or `.fidim`) the data
root is `<project>/.fidim/`; run outside a project it falls back to the XDG
directory `~/.local/share/fine-ill-do-it-myself/`. Use `fidim --data-root <dir>`
to pin one explicitly.

```
<data root>/
  bin/        managed binaries (caddy, php-fpm-<ver>, node, ...)
  runtimes/   extracted language/web runtimes (node/24, phpmyadmin/5.2.3, ...)
  etc/        generated configs (Caddyfile, php-fpm conf, my.cnf, certs/)
  run/        pid files + sockets
  logs/       service logs
  lock.toml   what is installed + sha256 pins
  sources.toml download manifest (copy of default_manifest.toml)
```

Tool directories under `bin/` are created only when something is installed
there, so a fresh data root stays small.

## Architecture

```
crates/core            logic: paths, manifest/lock, installer, proc runner,
                       supervisor (Env, PortPlan), orchestrator (start/stop/
                       status/hosts/certs), caddyfile + fpm + mariadb config
crates/fidim           clap CLI + ratatui TUI + unix socket daemon
```

Everything network/process-dependent lives behind small traits (`Fetcher`,
`Runner`) so the whole suite runs offline, no daemon needed.

## Development

- TDD: red → green, commit per step (Conventional Commits, tags per stage)
- `cargo fmt && cargo clippy --workspace --all-targets -- -D warnings && cargo test`
- pre-commit hook enforces fmt/clippy/test
