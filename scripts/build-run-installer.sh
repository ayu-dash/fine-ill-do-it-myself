#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
binary=${1:-"$repo_root/target/release/fidim"}
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo_root/Cargo.toml" | head -n 1)
output=${2:-"$repo_root/dist/fine-ill-do-it-myself-${version}-x86_64-linux.run"}

if [ ! -f "$binary" ] || [ ! -x "$binary" ]; then
    printf 'error: executable binary not found: %s\n' "$binary" >&2
    exit 1
fi

output_dir=$(dirname -- "$output")
output_name=$(basename -- "$output")
mkdir -p "$output_dir"
output_dir=$(CDPATH= cd -- "$output_dir" && pwd)
output_path=$output_dir/$output_name

work_dir=$(mktemp -d)
temporary_output=$output_path.tmp.$$
cleanup() {
    rm -rf -- "$work_dir"
    rm -f -- "$temporary_output"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

mkdir "$work_dir/payload"
cp "$binary" "$work_dir/payload/fidim"
cp "$repo_root/scripts/uninstall-fidim.sh" "$work_dir/payload/uninstall.sh"
chmod 755 "$work_dir/payload/uninstall.sh"
tar -czf "$work_dir/payload.tar.gz" -C "$work_dir/payload" .

cat > "$temporary_output" <<'INSTALLER'
#!/bin/sh
set -eu

usage() {
    printf '%s\n' \
        'Install fine ill do it myself for the current user.' \
        '' \
        'Usage: installer.run [--data-dir DIR] [--bin-dir DIR] [--yes]' \
        '' \
        'Defaults:' \
        "  data directory: $data_root" \
        "  command link:   $bin_dir/fidim" \
        '' \
        'Options:' \
        '  --data-dir DIR  Install the fidim executable in DIR (overrides XDG default)' \
        '  --bin-dir DIR   Put the fidim command symlink in DIR' \
        '  -y, --yes       Replace an existing command at the link path without prompting' \
        '  -h, --help      Show this help'
}

if [ -z "${HOME:-}" ]; then
    printf 'error: HOME is not set\n' >&2
    exit 1
fi

data_home=${XDG_DATA_HOME:-"$HOME/.local/share"}
data_root=$data_home/fine-ill-do-it-myself
bin_dir=$HOME/.local/bin
assume_yes=0

while [ "$#" -gt 0 ]; do
    case "$1" in
        --data-dir)
            [ "$#" -ge 2 ] || { printf 'error: --data-dir requires a path\n' >&2; exit 2; }
            data_root=$2
            shift 2
            ;;
        --bin-dir)
            [ "$#" -ge 2 ] || { printf 'error: --bin-dir requires a path\n' >&2; exit 2; }
            bin_dir=$2
            shift 2
            ;;
        -y|--yes)
            assume_yes=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            printf 'error: unknown option: %s\n' "$1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

case "$data_root:$bin_dir" in
    /*:/*) ;;
    *)
        printf 'error: install paths must be absolute\n' >&2
        exit 1
        ;;
esac

link_path=$bin_dir/fidim
if { [ -e "$link_path" ] || [ -L "$link_path" ]; } && [ ! -L "$link_path" ] && [ -d "$link_path" ]; then
    printf 'error: command link path is a directory: %s\n' "$link_path" >&2
    exit 1
fi
if [ -e "$link_path" ] || [ -L "$link_path" ]; then
    if [ "$assume_yes" -ne 1 ]; then
        if [ ! -t 0 ]; then
            printf 'error: %s already exists; rerun with --yes to replace it\n' "$link_path" >&2
            exit 1
        fi
        printf '%s already exists. Replace it? [y/N] ' "$link_path" >&2
        IFS= read -r answer || answer=
        case "$answer" in
            y|Y|yes|YES) ;;
            *) printf 'Installation cancelled.\n' >&2; exit 1 ;;
        esac
    fi
fi

payload_line=$(awk '$0 == "__FIDIM_PAYLOAD_BELOW__" { print NR + 1; exit }' "$0")
if [ -z "$payload_line" ]; then
    printf 'error: installer payload marker is missing\n' >&2
    exit 1
fi

work_dir=$(mktemp -d)
cleanup() {
    rm -rf -- "$work_dir"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
tail -n +"$payload_line" "$0" > "$work_dir/payload.tar.gz"
tar -xzf "$work_dir/payload.tar.gz" -C "$work_dir"
if [ ! -f "$work_dir/fidim" ]; then
    printf 'error: installer payload does not contain fidim\n' >&2
    exit 1
fi
if [ ! -f "$work_dir/uninstall.sh" ]; then
    printf 'error: installer payload does not contain uninstall.sh\n' >&2
    exit 1
fi

mkdir -p "$data_root" "$bin_dir"
staged_binary=$data_root/.fidim.new.$$
staged_uninstaller=$data_root/.uninstall.new.$$
staged_link=$bin_dir/.fidim.link.$$
cp "$work_dir/fidim" "$staged_binary"
chmod 755 "$staged_binary"
mv -f "$staged_binary" "$data_root/fidim"
cp "$work_dir/uninstall.sh" "$staged_uninstaller"
chmod 755 "$staged_uninstaller"
mv -f "$staged_uninstaller" "$data_root/uninstall.sh"
rm -f -- "$link_path"
ln -s "$data_root/fidim" "$staged_link"
mv -f "$staged_link" "$link_path"

printf 'Installed fidim to %s/fidim\n' "$data_root"
printf 'Command link: %s\n' "$link_path"
case ":${PATH:-}:" in
    *":$bin_dir:"*) ;;
    *) printf 'Note: add %s to PATH to run fidim by name.\n' "$bin_dir" ;;
esac

exit 0
INSTALLER

printf '\n__FIDIM_PAYLOAD_BELOW__\n' >> "$temporary_output"
cat "$work_dir/payload.tar.gz" >> "$temporary_output"
chmod 755 "$temporary_output"
mv -f "$temporary_output" "$output_path"
trap - EXIT HUP INT TERM
cleanup
printf 'Created %s\n' "$output_path"