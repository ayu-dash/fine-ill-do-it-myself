#!/bin/sh
set -eu

usage() {
    printf '%s\n' \
        'Uninstall the fidim executable for the current user.' \
        '' \
        'Usage: uninstall.sh [--data-dir DIR] [--bin-dir DIR]' \
        '' \
        'This removes the installed executable, its uninstaller, and the matching' \
        'fidim command symlink. Managed projects, runtimes, config, and logs are kept.' \
        '' \
        'Options:' \
        '  --data-dir DIR  Installed fidim data root (overrides XDG default)' \
        '  --bin-dir DIR   Directory containing the fidim command symlink' \
        '  -h, --help      Show this help'
}

if [ -z "${HOME:-}" ]; then
    printf 'error: HOME is not set\n' >&2
    exit 1
fi

data_home=${XDG_DATA_HOME:-"$HOME/.local/share"}
data_root=$data_home/fine-ill-do-it-myself
bin_dir=$HOME/.local/bin

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

binary=$data_root/fidim
uninstaller=$data_root/uninstall.sh
link=$bin_dir/fidim

if [ -L "$link" ]; then
    target=$(readlink "$link")
    if [ "$target" = "$binary" ]; then
        rm -f -- "$link"
        printf 'Removed command link %s\n' "$link"
    else
        printf 'Keeping unrelated symlink %s -> %s\n' "$link" "$target"
    fi
elif [ -e "$link" ]; then
    printf 'Keeping non-symlink command path %s\n' "$link"
fi

if [ -f "$binary" ] && [ ! -L "$binary" ]; then
    rm -f -- "$binary"
    printf 'Removed executable %s\n' "$binary"
elif [ -e "$binary" ] || [ -L "$binary" ]; then
    printf 'Keeping non-regular executable path %s\n' "$binary"
fi
if [ -f "$uninstaller" ] && [ ! -L "$uninstaller" ]; then
    rm -f -- "$uninstaller"
    printf 'Removed uninstaller %s\n' "$uninstaller"
elif [ -e "$uninstaller" ] || [ -L "$uninstaller" ]; then
    printf 'Keeping non-regular uninstaller path %s\n' "$uninstaller"
fi

printf 'Kept managed data in %s (projects, runtimes, config, and logs).\n' "$data_root"
