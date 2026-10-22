#!/usr/bin/env bash
set -euo pipefail
repo=$(cd -- "$(dirname -- "$0")/.." && pwd)
[[ $# == 1 && "$1" == /* && ! -e "$1" ]] || { echo 'usage: build-rtos-reader.sh /new/output' >&2; exit 2; }
out=$1
feature=${RP1_RTOS_READER_FEATURE:-rp1-rtos-record}
case "$feature" in rp1-amp-proc1-profile|rp1-amp-observation|rp1-amp-proc1|rp1-amp-service|rp1-time-anchor|rp1-scmi-linux-preloaded|rp1-scmi-cold-observer|rp1-rtos-watchdog-quiescence|rp1-rtos-watchdog-receipt|rp1-rtos-record|rp1-rtos-soak|rp1-rtos-mixed-repeat) ;; *) exit 2 ;; esac
family=$feature
if [[ "$feature" == rp1-amp-proc1-profile ]]; then family=rp1-amp-observation; fi
mkdir -p "$out"
exec > "$out/build.txt" 2>&1
cd "$repo"
date --iso-8601=seconds
git branch --show-current
git rev-parse HEAD
printf 'selected_feature=%s\n' "$feature"
git diff --binary > "$out/source.diff"
git diff --cached --binary > "$out/index.diff"
if [[ "$family" == rp1-time-anchor || "$family" == rp1-amp-service || "$family" == rp1-amp-proc1 || "$family" == rp1-amp-observation ]]; then
    : "${RP1_TIME_ANCHOR_DTB:?set the actual reviewed input DTB path}"
    admission_args=(--time-anchor)
    if [[ "$family" == rp1-amp-service || "$family" == rp1-amp-proc1 || "$family" == rp1-amp-observation ]]; then
        : "${RP1_AMP_SERVICE_ELF:?set the actual sealed AG01 firmware ELF}"
        admission_args+=(--amp-service --firmware "$RP1_AMP_SERVICE_ELF")
        if [[ "$family" == rp1-amp-proc1 || "$family" == rp1-amp-observation ]]; then admission_args+=(--amp-proc1); fi
        if [[ "$family" == rp1-amp-observation ]]; then admission_args+=(--amp-observation); fi
        if [[ "$feature" == rp1-amp-proc1-profile ]]; then admission_args+=(--amp-proc1-profile); fi
    fi
    python3 -B tools/test-scmi-linux-preloaded.py "${admission_args[@]}" --dtb "$RP1_TIME_ANCHOR_DTB"
    if [[ "$family" == rp1-amp-proc1 || "$family" == rp1-amp-observation ]]; then
        : "${RP1_AMP_SERVICE_REFERENCE:?set the retained phase0 ELF for inspector regression}"
        image_args=()
        if [[ "$family" == rp1-amp-observation ]]; then image_args+=(--amp-observation); fi
        if [[ "$feature" == rp1-amp-proc1-profile ]]; then image_args+=(--amp-proc1-profile); fi
        python3 -B tools/test-amp-proc1-image.py --firmware "$RP1_AMP_SERVICE_ELF" --service-firmware "$RP1_AMP_SERVICE_REFERENCE" "${image_args[@]}"
        sha256sum "$RP1_AMP_SERVICE_REFERENCE" > "$out/input-service-elf.sha256"
    fi
    sha256sum "$RP1_TIME_ANCHOR_DTB" > "$out/input-dtb.sha256"
fi
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_ENCODED_RUSTFLAGS=''
export CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_PROFILE_RELEASE_STRIP=debuginfo
# build.rs already supplies the linker script; do not add the .cargo flag twice.
cargo +nightly-2026-08-27 build --offline --locked -j1 \
 -Z build-std=core,alloc,compiler_builtins -Z build-std-features=compiler-builtins-mem \
 -p rp1_chainboot_poc --target aarch64-unknown-none-softfloat --release \
 --no-default-features --features "log-uart,tftp-boot,require-rp1-img,$feature"
cp "${CARGO_TARGET_DIR:-target}/aarch64-unknown-none-softfloat/release/rp1_chainboot_poc" "$out/reader.elf"
llvm-objcopy -O binary "$out/reader.elf" "$out/kernel_2712.img"
sha256sum "$out/reader.elf" "$out/kernel_2712.img" > "$out/output.sha256"
llvm-readelf -lSW "$out/reader.elf" > "$out/readelf.txt"
python3 "$repo/tools/check-reader-elf.py" "$out/reader.elf" > "$out/elf-check.json"
date --iso-8601=seconds
