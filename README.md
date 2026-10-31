# fine-ill-do-it-myself (fidim)

A local web dev stack manager for Linux, in the spirit of Laragon. It wires up
Caddy + PHP-FPM + (optional system) MariaDB, one folder per project under
`~/.local/share/fine-ill-do-it-myself/projects`, reachable at `*.test` domains
with real HTTPS.

## What you get

- Managed project folders become `https://<name>.test` vhosts automatically
- Framework-aware docroot: a project with `public/index.php` (Laravel, Symfony)
  is served from `public/`, and unknown paths reach `index.php` (nginx,
  lighttpd and Caddy natively, Apache through the project's `.htaccess`);
  flat PHP projects serve their own folder and keep a real 404
- Quick create from templates: Laravel, WordPress, PHP, static HTML, blank,
  Node and Python, optionally with a MariaDB database
- A local certificate authority (one root, one leaf per project) generated with
  openssl — real TLS, no `tls internal` surprises
- php-fpm pool + caddy config auto-generated per start
- Download manager with sha256 verification for managed binaries and runtimes
- Optional MariaDB 11.4.13 Linux x86_64 runtime, downloaded with curl into the
  project data root; a system `mariadbd` on `PATH` remains supported
- phpMyAdmin 5.2.3 as a managed site (`https://phpmyadmin.test`), bundled in
  the Linux `.run` installer and pointed at the fidim-managed MariaDB
- A TUI (`fidim` default command), status CLI, unix-socket daemon

## Install

Download the Linux `.run` installer from GitHub Releases and run it as your
normal user. It installs the executable under
`~/.local/share/fine-ill-do-it-myself/` and creates
`~/.local/bin/fidim`; it does not require `sudo`.

```sh
chmod +x fine-ill-do-it-myself-<version>-x86_64-linux.run
./fine-ill-do-it-myself-<version>-x86_64-linux.run
```

The installer asks before replacing an existing `~/.local/bin/fidim`; pass
`--yes` to replace it without prompting. Ensure `~/.local/bin` is on `PATH`.
It also unpacks the bundled phpMyAdmin 5.2.3 (with its license files) into
`runtimes/phpmyadmin/`, the bundled Composer 2.10.3 into `bin/composer`, and
the bundled PHP 8.4 (FPM + CLI) into `bin/php/`, so none of them needs a
download; existing trees, binaries, a generated `config.inc.php` or an updated
Composer are left untouched on reinstall. The PHP License 3.01 text is written
to `licenses/php-3.01.txt`. To build a `.run` installer locally after a
release build, use `scripts/build-run-installer.sh`;
`FIDIM_BUNDLE_PHPMYADMIN=0`, `FIDIM_BUNDLE_COMPOSER=0` and
`FIDIM_BUNDLE_PHP=0` leave those components out. Apache is not bundled: run
`fidim install httpd`, which assembles Apache 2.4.67 from the pinned Debian
bookworm packages in `sources.toml` (it needs glibc 2.36+: Debian 12+, Ubuntu
24.04+, current Fedora/Arch), then `fidim web use apache`. The portable tarball
and `cargo install` remain available as alternatives.

To uninstall, run
`~/.local/share/fine-ill-do-it-myself/uninstall.sh`. It removes the managed
`fidim` executable, its matching command symlink, and generated command shims
from `fidim env --install`, while keeping projects, downloaded runtimes,
configuration, and logs. Shims are removed only when they have fidim's
generated-file marker; unrelated commands are kept. Pass `--shim-dir DIR` if
you installed shims in a custom directory. If you installed fidim with custom
directories, pass the same `--data-dir DIR` and `--bin-dir DIR` options.

```sh
cargo build --release
cargo install --path crates/fidim
```

Bootstrap binaries from the manifest (defaults ship inside the binary):

```sh
fidim install caddy
fidim install httpd     # Apache from the pinned Debian package recipe in sources.toml
fidim install mariadb   # optional, portable Linux x86_64 runtime (downloaded with curl)
fidim install composer  # optional: pinned composer.phar, run through the managed PHP
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
Selecting a version refreshes fidim's command shims automatically. The first
PHP version and each language's first managed version are activated
automatically; additional versions can be selected with
`fidim <lang> use <version>`. PHP and language management panels in the TUI
also list installable versions.

`fidim <lang> remove <version>` deletes the extracted runtime tree, drops the
version from `lock.toml` and re-points the active `bin/<lang>/<lang>` link (or
removes it when no managed version is left). Removing a version that is not
installed is an error, not a silent no-op.

## Day to day

```sh
fidim start     # caddy + php-fpm (+ mariadb if available); safe to re-run
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
fidim daemon &  # socket at <data root>/data/fidim.sock; SIGTERM/SIGINT removes it
```

The daemon answers one line per request (`ping`, `status`, `start <service>`,
`stop <service>`); `status` reports the same services as `fidim status`.

**Use `mysql`/`mariadb`/`php`/`node`/`python`/`go`/`java` straight from your
terminal** — two options:

```sh
eval "$(fidim env)"     # per shell, inside this data root
fidim env --install     # everywhere: writes wrappers to ~/.local/bin
fidim env --install --shell  # offer a confirmed Bash/Zsh startup-file update
```

`fidim env` prepends the managed client dirs to `PATH` (mariadb, php, node,
python, go, java) and points `MYSQL_UNIX_PORT`/`MYSQL_TCP_PORT` at this data
root, so `mariadb -u root` (or the `mysql` alias) talks to the fidim-managed
server.

`fidim env --install` writes tiny `mysql`, `mariadb`, `php`, `node`, `npm`,
`python3`, `pip3`, `go`, `java`, `javac`, ... wrappers into `~/.local/bin`
(override with `$FIDIM_SHIM_DIR`). They follow the active
`fidim <lang> use <version>` selection and point at this data root, so they work
from any directory. Installing or switching a runtime refreshes these wrappers
automatically; re-run `fidim env --install` after moving the data root.
If you opted into persistent shell integration, re-run
`fidim env --install --shell` to update its managed block.
`~/.local/bin` must be on your `PATH`; by default fidim does not edit shell
startup files. The optional `--shell` flow asks for confirmation before adding
a managed PATH block to `.bashrc` or `.zshrc`, and backs up an existing file as
`<rc-file>.fidim.bak`. Non-interactive runs never edit shell startup files.

## Create a project

```sh
fidim new blog              # PHP welcome page in the managed projects folder
fidim new html landing      # static index.html
fidim new laravel shop      # composer create-project
fidim new wordpress wp      # pinned tarball, sha256 checked
fidim new node app --db     # Node template plus database `app`
fidim new --template python api
```

Templates are `laravel`, `wordpress`, `php`, `html`, `blank`, `node` and
`python`; a name may only contain `a-z`, `0-9` and `-`. Everything is written to
a hidden staging folder and renamed into place, so a failed download or a
missing composer leaves no half project. With `--db`, a failure to create the
database removes the new project folder again and the command exits non-zero.

## Seamless `*.test` resolution

No `/etc/hosts` editing per project. `fidim` ships a wildcard DNS responder: it
answers every `A` query under `.test` with `127.0.0.1` and everything else with
NXDOMAIN, so a new managed project folder is reachable as `http://<folder>.test`
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

**phpMyAdmin**: the `.run` installer already extracts phpMyAdmin 5.2.3 (with
`--strip-components=1`) under `runtimes/phpmyadmin/<version>/`; installs from
the tarball or `cargo install` get it with `fidim install phpmyadmin`. Either
way fidim links it into the managed `projects/phpmyadmin`,
so the usual
vhost/certificate/hosts generation serves it at
`https://phpmyadmin.test` (plus the HTTP variant). Its `config.inc.php` points
at the fidim-managed MariaDB socket and uses the normal login form: user
`root`, empty password. With `fidim dns install` it is reachable with no extra
setup; otherwise add the entry once via `fidim hosts`. The symlink is only
replaced when it is a fidim symlink, never when a real project folder has that
name.

**Composer**: the `.run` installer already places the pinned `composer.phar`
(currently 2.10.3) in `bin/composer`; other installs fetch it with
`fidim install composer`. Either way fidim never relies on a system-wide
Composer. `fidim env --install` writes a shim that runs it through the managed
PHP (`bin/php/php`), with `COMPOSER_HOME` and `COMPOSER_CACHE_DIR` kept under
the data root instead of `$HOME`. In the TUI the same install is available from
the management panel (`composer.install <version>`). The PHP management panel
also offers installation of available PHP versions; MariaDB and phpMyAdmin are
offered from the MariaDB panel and the global config list. Language panels list
installable Node, Python, Go and Java versions. These optional components are
installed only when selected.

Inside `~/.local/share/fine-ill-do-it-myself/etc/keys.toml` you can remap TUI
keys (defaults q/r/s/x/a/o):

```toml
quit = z
restart = t      # restart service under the cursor
start = b        # start service under the cursor
stop = n         # stop service under the cursor
restart_all = g  # stop+start everything
editor = o       # open the project under the cursor in your code editor
```

`Enter` on a project opens it in the browser; `o` opens the project folder in a
code editor. The editor is taken from `FIDIM_EDITOR`, then `$VISUAL`, then
`$EDITOR`, then the first installed GUI editor (`code`, `codium`, `cursor`,
`zed`, `subl`, `idea`, `kate`, `geany`) and finally a terminal editor
(`nvim`, `vim`, `nano`, `hx`, `micro`). GUI editors are spawned detached;
terminal editors take over the terminal while they run. `FIDIM_EDITOR` may
carry arguments:

```sh
FIDIM_EDITOR="code --reuse-window" fidim
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

The data root is `$XDG_DATA_HOME/fine-ill-do-it-myself/` (normally
`~/.local/share/fine-ill-do-it-myself/`) regardless of the current directory.
Use `fidim --data-root <dir>` or `FIDIM_DATA_ROOT` to override it explicitly.
Existing `~/Sites` and project-local `.fidim` data are left untouched; no
migration is performed. Copy projects you want fidim to manage into
`<data root>/projects/`. Runtime/config data at old roots is not used
automatically.

```
<data root>/
  bin/        managed binaries (caddy, php-fpm-<ver>, node, ...)
  projects/  web projects created and served by fidim
  runtimes/   extracted language/web runtimes (node/24, phpmyadmin/5.2.3, ...)
  etc/        generated configs (Caddyfile, php-fpm conf, my.cnf, certs/)
  licenses/   license texts of components bundled with the installer
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

## Third-party binaries and licenses

`fidim` does not ship third-party binaries in its repository or in its release
tarball. The Linux `.run` installer is the exception: it embeds the unmodified
phpMyAdmin 5.2.3 tarball (license files included), the Composer 2.10.3 phar,
and the PHP 8.4 static builds with the PHP License 3.01 text, so they work
without network access (`FIDIM_BUNDLE_PHPMYADMIN=0`,
`FIDIM_BUNDLE_COMPOSER=0` and `FIDIM_BUNDLE_PHP=0` exclude them). Apache is
fetched by `fidim install httpd` from the pinned Debian packages listed in
`default_manifest.toml`. Everything else is fetched by `fidim install <name>`
from the upstream URLs listed there, where every entry pins a sha256; a
mismatch aborts before extraction. Each archive keeps its own license:

| Component | License |
|---|---|
| Caddy | Apache-2.0 |
| nginx | BSD-2-Clause |
| Apache httpd | Apache-2.0 |
| lighttpd | BSD-3-Clause |
| PHP (static-php-cli builds) | PHP License 3.01 |
| MariaDB server | GPL-2.0 |
| MariaDB client libraries | LGPL-2.1 |
| phpMyAdmin | GPL-2.0-or-later |
| Composer | MIT |
| Node.js (npm: Artistic-2.0) | MIT |
| Python | PSF-2.0 |
| Go | BSD-3-Clause |
| Eclipse Temurin JDK | GPL-2.0 with Classpath Exception |
| WordPress (template only) | GPL-2.0-or-later |

Runtimes fetched by `fidim install phpmyadmin` and `fidim new wordpress` are
downloaded to your machine, not redistributed by this project. Bundling any of
them into a release artifact means shipping its license text and, for the GPL
components, the corresponding source or a written offer.

## Development

- TDD: red → green, commit per step (Conventional Commits, tags per stage)
- `cargo fmt && cargo clippy --workspace --all-targets -- -D warnings && cargo test`
- pre-commit hook enforces fmt/clippy/test
