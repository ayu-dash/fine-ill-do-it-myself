# fine-ill-do-it-myself (fidim)

Local web development environment for Linux.
Manage web servers, PHP, databases, runtimes, and projects from one CLI/TUI.

## Install

Download the Linux installer from Releases, then run:

```sh
chmod +x fine-ill-do-it-myself-<version>-x86_64-linux.run
./fine-ill-do-it-myself-<version>-x86_64-linux.run
```

Or install from source:

```sh
cargo install --path crates/fidim
```

The default data directory is:

```text
~/.local/share/fine-ill-do-it-myself/
```

Override it with `--data-root DIR` or `FIDIM_DATA_ROOT`.

## Install components

```sh
fidim install caddy
fidim install httpd       # Apache
fidim install nginx
fidim install lighttpd
fidim install mariadb
fidim install composer
fidim install phpmyadmin

fidim php install 8.4
fidim php use 8.4
fidim node install 24
fidim python install 3.13
fidim go install 1.27
fidim java install 25
```

Apache can be selected using either `apache` or `httpd`:

```sh
fidim web use caddy
fidim web use apache
fidim web use nginx
fidim web use lighttpd
fidim web show
```

Run `fidim --help` or `fidim <command> --help` for all available options.

## Daily use

Running `fidim` without arguments opens the TUI.

```sh
fidim start
fidim stop
fidim restart
fidim status
fidim projects
```

Start or stop one service:

```sh
fidim start httpd
fidim stop httpd
```

Show or change ports:

```sh
fidim ports show
fidim ports set http 8080
```

## Create projects

```sh
fidim new blog
fidim new laravel shop
fidim new wordpress wp
fidim new html landing
fidim new node app --db
fidim new --template python api
```

Projects are created under the managed `projects/` directory and are available
at `http://<project>.test` or `https://<project>.test`.

## `.test` resolution

Use the DNS setup when `systemd-resolved` is available:

```sh
fidim dns install
fidim dns status
fidim dns uninstall --yes
```

If DNS setup is unavailable, use `/etc/hosts`:

```sh
fidim hosts --dry-run
fidim hosts
```

Trust the local certificate authority when browser HTTPS warnings remain:

```sh
fidim trust
```

## Shell environment

Use managed PHP, database, and language clients in the current shell:

```sh
eval "$(fidim env)"
```

Install persistent command wrappers:

```sh
fidim env --install
fidim env --install --shell
```

## Data layout

```text
<data-root>/
├── bin/       managed binaries and active version links
├── projects/  local web projects
├── runtimes/  extracted runtime files
├── etc/       generated configuration and certificates
├── logs/      service logs
├── run/       PID files and sockets
└── lock.toml  installed versions and checksums
```

## Development

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
