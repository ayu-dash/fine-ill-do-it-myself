#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d)
cleanup() {
    rm -rf -- "$test_root"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

make_fake_fidim() {
    version=$1
    destination=$2
    cat > "$destination" <<EOF
#!/bin/sh
if [ "\${1:-}" = "--version" ]; then
    printf 'fidim %s\\n' '$version'
else
    printf 'fake fidim %s\\n' '$version'
fi
EOF
    chmod 755 "$destination"
}

fake_binary=$test_root/fidim
installer=$test_root/fine-ill-do-it-myself-test.run
home=$test_root/home
mkdir -p "$home"

make_fake_fidim 1.0 "$fake_binary"
"$repo_root/scripts/build-run-installer.sh" "$fake_binary" "$installer"
help=$(HOME=$home "$installer" --help)
printf '%s\n' "$help" | grep -F "$home/.local/share/fine-ill-do-it-myself" >/dev/null
printf '%s\n' "$help" | grep -F "$home/.local/bin/fidim" >/dev/null
HOME=$home "$installer" --yes --data-dir "$home/.local/share/fine-ill-do-it-myself" --bin-dir "$home/.local/bin"
test "$("$home/.local/bin/fidim" --version)" = "fidim 1.0"
test -x "$home/.local/share/fine-ill-do-it-myself/fidim"
test "$(readlink "$home/.local/bin/fidim")" = "$home/.local/share/fine-ill-do-it-myself/fidim"

make_fake_fidim 2.0 "$fake_binary"
"$repo_root/scripts/build-run-installer.sh" "$fake_binary" "$installer"
if HOME=$home "$installer" --data-dir "$home/.local/share/fine-ill-do-it-myself" --bin-dir "$home/.local/bin" </dev/null; then
    printf 'error: installer replaced an existing link without confirmation\n' >&2
    exit 1
fi
test "$("$home/.local/bin/fidim" --version)" = "fidim 1.0"

HOME=$home "$installer" --yes --data-dir "$home/.local/share/fine-ill-do-it-myself" --bin-dir "$home/.local/bin"
test "$("$home/.local/bin/fidim" --version)" = "fidim 2.0"

mkdir "$test_root/link-target"
rm -f "$home/.local/bin/fidim"
ln -s "$test_root/link-target" "$home/.local/bin/fidim"
HOME=$home "$installer" --yes --data-dir "$home/.local/share/fine-ill-do-it-myself" --bin-dir "$home/.local/bin"
test "$(readlink "$home/.local/bin/fidim")" = "$home/.local/share/fine-ill-do-it-myself/fidim"
test -z "$(find "$test_root/link-target" -mindepth 1 -print -quit)"

default_home=$test_root/default-home
mkdir -p "$default_home"
HOME=$default_home "$installer" --yes
test "$("$default_home/.local/bin/fidim" --version)" = "fidim 2.0"
test -x "$default_home/.local/share/fine-ill-do-it-myself/fidim"

xdg_home=$test_root/xdg-home
mkdir -p "$xdg_home"
HOME=$xdg_home XDG_DATA_HOME=$test_root/xdg-data "$installer" --yes
test "$("$xdg_home/.local/bin/fidim" --version)" = "fidim 2.0"
test -x "$test_root/xdg-data/fine-ill-do-it-myself/fidim"
printf 'Installer tests passed.\n'
