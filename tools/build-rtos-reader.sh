#!/usr/bin/env bash
set -euo pipefail
repo=$(cd -- "$(dirname -- "$0")/.." && pwd)
[[ $# == 1 && "$1" == /* && ! -e "$1" ]] || { echo 'usage: build-rtos-reader.sh /new/output' >&2; exit 2; }
out=$1
feature=${RP1_RTOS_READER_FEATURE:-rp1-rtos-record}
case "$feature" in rp1-rtos-watchdog-quiescence|rp1-rtos-watchdog-receipt|rp1-rtos-record|rp1-rtos-soak|rp1-rtos-mixed-repeat) ;; *) exit 2 ;; esac
mkdir -p "$out"
exec > "$out/build.txt" 2>&1
cd "$repo"
date --iso-8601=seconds
git branch --show-current
git rev-parse HEAD
printf 'selected_feature=%s\n' "$feature"
git diff --binary > "$out/source.diff"
git diff --cached --binary > "$out/index.diff"
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
date --iso-8601=seconds
