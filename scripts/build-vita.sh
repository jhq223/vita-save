#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export VITASDK="${VITASDK:-/usr/local/vitasdk}"
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$VITASDK/bin:$PATH"
bash scripts/build-mount.sh
export CC_armv7_sony_vita_newlibeabihf=arm-vita-eabi-gcc
export AR_armv7_sony_vita_newlibeabihf=arm-vita-eabi-ar
export LIBSQLITE3_FLAGS="SQLITE_OS_OTHER=1 SQLITE_OMIT_LOAD_EXTENSION SQLITE_THREADSAFE=0 SQLITE_TEMP_STORE=3"
cargo vita build vpk --release --locked "$@"

# Verify process-parameter symbols survived cargo-vita's optional strip step.
elf=target/armv7-sony-vita-newlibeabihf/release/vita-save.elf
symbols="$(arm-vita-eabi-readelf --syms --wide "$elf")"
for name in sceLibcHeapSize sceUserMainThreadStackSize _newlib_heap_size_user; do
    if ! awk -v name="$name" '$8 == name && $7 != "UND" { found = 1 } END { exit !found }' <<< "$symbols"; then
        echo "Missing Vita runtime symbol: $name (keep strip_symbols = false)" >&2
        exit 1
    fi
done
