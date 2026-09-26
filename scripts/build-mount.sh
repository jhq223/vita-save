#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export VITASDK="${VITASDK:-/usr/local/vitasdk}"
export PATH="$HOME/.cargo/bin:$VITASDK/bin:$PATH"
target=armv7-sony-vita-newlibeabihf
mkdir -p target/native/{kernel,user} runtime/module
for kind in kernel user; do
    cargo build --locked --manifest-path native/Cargo.toml --target "$target" --target-dir "target/mount-$kind" -Zbuild-std=core --release --features "$kind"
    lib="target/mount-$kind/$target/release/libvita_save_mount.a"
    output="target/native/$kind/vita-save-$kind"
    if [[ "$kind" == kernel ]]; then
        symbol=vitaSaveKernelMountById
        libs=(-lSceSysclibForDriver_stub -lSceSysmemForDriver_stub -lSceModulemgrForDriver_stub -lSceThreadmgrForDriver_stub -lSceProcessmgrForDriver_stub -ltaihenForKernel_stub -ltaihenModuleUtils_stub)
        extension=skprx
    else
        symbol=vitaSaveUserMountById
        libs=(-Ltarget/native/kernel -lVitaSaveMountKernel_stub -lSceLibKernel_stub)
        extension=suprx
    fi
    arm-vita-eabi-gcc -nostdlib -Wl,-q,-e,module_start,--gc-sections,--undefined=module_start,--undefined=module_stop,--undefined="$symbol" "$lib" "${libs[@]}" -o "$output.elf"
    vita-elf-create -e "native/$kind/exports.yml" "$output.elf" "$output.velf"
    vita-make-fself -c "$output.velf" "runtime/module/vita-save-$kind.$extension"
    vita-elf-export "$kind" "$output.elf" "native/$kind/exports.yml" "$output.yml"
    vita-libs-gen "$output.yml" "target/native/$kind/stubs"
    make -C "target/native/$kind/stubs" -j4
    cp "target/native/$kind/stubs/"*.a "target/native/$kind/"
done
