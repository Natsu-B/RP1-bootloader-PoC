# RP1 Chainboot PoC

This repository contains an AArch64 EL2 bootloader PoC for Raspberry Pi 5 /
BCM2712 / RP1 bringup.

The bootloader stays at EL2, reloads RP1 firmware through the RP1 I2C bootstrap
path, keeps the already initialized BCM2712 SDHC block-device instance, loads
Linux artifacts from the same SDHC instance, patches the DTB, and jumps directly
to the arm64 Linux Image at EL2.

## EL2 Direct Handoff

The handoff intentionally does not build an EL2-to-EL1 wrapper. Linux is entered
directly at EL2 following the arm64 boot protocol:

- `x0 = DTB physical address`
- `x1 = x2 = x3 = 0`
- interrupts masked through `DAIF`
- EL2 stage-1 MMU and caches disabled immediately before branch
- stale I-cache invalidated
- kernel, initramfs, and DTB ranges cleaned to PoC before the final branch

Immediately before the branch, the handoff clears the stage-2 root with
`VTTBR_EL2=0`, sets `CNTVOFF_EL2=0`, writes `CPTR_EL2=0`, replaces `HCR_EL2`
with the minimal `RW` value for AArch64 execution, clears `SCTLR_EL2.M/C/I`, and
branches to Image start with no EL1 wrapper.

The Image is decompressed/copied to `KERNEL_LOAD_BASE = 0x06000000`. Before
handoff, the loader validates the arm64 Image header (`text_offset`,
`image_size`, `flags`, and `ARMd` magic) and verifies that the load address is
`text_offset` bytes from a 2 MiB-aligned Image base. Invalid placement is fatal;
the PoC will not branch to Linux from a known-bad address.

The final handoff function is placed in `.text.boot.handoff`, but the current
implementation still assumes the executable mapping remains identity-compatible
while `SCTLR_EL2.M` is cleared. Trap register policy is intentionally minimal
and must be audited on hardware.

## Linker and Raw Image

The linker script places `.text.boot` at `0x80000`, so `_start` is at the raw
image execution address used by the Raspberry Pi firmware path. The script
intentionally avoids `FILEHDR`, `PHDRS`, and `SIZEOF_HEADERS` in front of
`.text.boot`, so `objcopy -O binary` produces an image whose first bytes are
AArch64 instructions rather than an ELF header.

After `cargo xbuild`, verify the entry placement with:

```sh
readelf -s ./bin/rp1_chainboot_poc.elf | grep -E '(_start|_PROGRAM_START|_STACK_TOP|_LINUX_IMAGE)'
xxd -l 64 ./bin/rp1_chainboot_poc.img
```

Expected properties:

- `_start == 0x80000`
- `_PROGRAM_START == 0x80000`
- `_LINUX_IMAGE == 0x6000000`
- `./bin/rp1_chainboot_poc.img` does not start with ELF magic `7f 45 4c 46`

On the current build, `readelf` reports `_start` at `0x80000` and `xxd` shows
the raw image begins with the AArch64 `msr SPSel, #1` instruction bytes rather
than an ELF header.

The stack remains intentionally small at 1 MiB plus one 4 KiB guard page. This
differs from some `rpi_boot` layouts that reserve a much larger stack, but keeps
`_STACK_TOP < 0x6000000` with room for the Linux Image placement.

## SDHC Lifetime

SDHC is initialized exactly once using `bcm2712::sdhc::init_from_dtb(&dtb)`.
After RP1 reset and I2C firmware reload, the same `&'static dyn BlockDevice`
instance is reused to read `/config.txt`, `/kernel_2712.img`, and
`/initramfs_2712`.

This is deliberate: the PoC is testing whether RP1 firmware can be reloaded while
the BCM2712 SDHC instance remains usable. Reinitializing SDHC after RP1 reset
would hide that behavior.

Before Linux handoff, the PoC disables SDHCI interrupt masks/signals and issues
CMD/DATA software reset using a minimal MMIO quiesce path.

## Allocator

This PoC currently uses a static 8 MiB bump allocator. It does not yet build a
full allocator from DTB memory and reserved-memory nodes.

## DTB Placement

The firmware-provided DTB is parsed at `0x20000000`. The patched Linux DTB is
written to `0x20200000`, not back over the input DTB. This keeps the parser input
region and the Linux handoff DTB region separate during bringup.

After all SD files are loaded, SDHC is quiesced before the final DTB patch and
Linux handoff.

## SD Card Files

Required:

- `/kernel_2712.img`
- `/initramfs_2712`

Preferred RP1 firmware image:

- `/RP1.img`

Fallback RP1 firmware parts:

- `/rp1c0fw1.bin`
- `/rp1c0fw2.bin`

Optional:

- `/cmdline.txt`
- `/config.txt` is probed before and after RP1 reset for bringup logging

Optional file reads only return `None` for FAT `NotFound`. SD mount, open, and
read errors remain fatal so a damaged SD/FAT path is not mistaken for an absent
optional file.

## RP1.img Format

`/RP1.img` starts with a 32-byte little-endian header followed by the payload:

```rust
#[repr(C)]
pub struct Rp1ImgHeader {
    pub magic: u32,      // little-endian "RP1I"
    pub header_len: u32, // usually 0x20
    pub image_len: u32,
    pub load_addr: u32,  // must be 0x20000000
    pub entry: u32,      // loader sets Thumb bit before scratch write
    pub stack: u32,
    pub crc32: u32,      // 0 skips CRC in this PoC
    pub flags: u32,
}
```

Validation checks magic, header bounds, payload length, load address, entry
range, nonzero stack, and CRC32 when provided.

If `/RP1.img` is absent, `rp1c0fw1.bin` and `rp1c0fw2.bin` are concatenated into
one payload:

```text
0x20000000: rp1c0fw1.bin
            rp1c0fw2.bin
```

The second part is not reloaded at `0x20000000`; it follows the first part.

Fallback fw-parts mode uses `RP1_FALLBACK_ENTRY=0x20000141` and
`RP1_FALLBACK_STACK=0x100030d0`. This is analysis-derived and less safe than
`/RP1.img`; prefer `/RP1.img` because it carries exact entry and stack metadata.

Build features:

- `allow-fw-parts-fallback` is enabled by default.
- `require-rp1-img` makes absence of `/RP1.img` fatal and disables fw-parts
  fallback at runtime.
- `skip-rp1-reload` skips RP1 reset/I2C firmware reload entirely. Use it to
  isolate Linux handoff problems from RP1 reload or PCIe state problems.
- `continue-on-rp1-bootstrap-failure` keeps the old bringup behavior of
  continuing toward Linux after RP1 bootstrap failure. It is intentionally not
  enabled by default.

## RP1 Reset and Probe

After RP1_RUN reset, the loader writes `0x00800000` to `0x40017004` through the
same I2C bootstrap write protocol before reading the chip id. This mirrors the
observed bootstrap requirement that a reset clear must occur before chip-id
access.

By default, RP1 bootstrap failure is fatal and Linux handoff is refused. The log
spells this out as:

```text
[RP1BOOT] bootstrap failed: ...; refusing Linux handoff unless continue-on-rp1-bootstrap-failure is enabled
```

For controlled PoC experiments only, build with
`--features continue-on-rp1-bootstrap-failure` to keep going after such a
failure.

## Logging Backend

`log!` and `logln!` route through a small logging facade. The call sites do not
care whether output goes to UART or semihosting, because nobody needs debug code
with tentacles.

Default logging uses the Raspberry Pi 5 debug UART:

```sh
cargo xbuild
```

Equivalent explicit feature selection:

```sh
cargo xbuild --features log-uart
```

For semihosting logs, disable default features and select the desired runtime
features explicitly:

```sh
cargo run -p xtask -- build --no-default-features --features "allow-fw-parts-fallback log-semihosting"
```

The two logging backends are mutually exclusive:

- `log-uart`
- `log-semihosting`

If neither backend is selected, or both are selected, compilation fails. The
semihosting backend uses AArch64 semihosting `SYS_WRITEC` via `hlt #0xf000`, so
it requires a debugger or emulator configured to handle semihosting traps. On
real hardware without semihosting support, use `log-uart`.

## Build

Use a nightly Rust toolchain with `rust-src`, `llvm-tools-preview`, and the
`aarch64-unknown-none-softfloat` target. The included flake provides that setup.

```sh
cargo run
cargo xbuild
cargo xrun
```

Generated artifacts:

- `./bin/rp1_chainboot_poc.elf`
- `./bin/rp1_chainboot_poc.img`

`cargo build -p rp1_chainboot_poc --target aarch64-unknown-none-softfloat` builds
the ELF through Cargo. Stable Cargo has no clean post-link hook for raw image
generation, so raw image creation is intentionally handled by `xtask`. The
crate build script emits a warning pointing users at `cargo xbuild` and
`cargo xrun`.

## Gzip

`kernel_2712.img` is treated as gzip when it starts with `1f 8b`. The PoC parses
the gzip header itself, inflates the deflate payload with `miniz_oxide` in
`no_std + alloc` mode, verifies `ISIZE`, and copies the result to
`0x06000000`.

Non-gzip input is copied directly as an arm64 Image.

After decompression/copy, the arm64 Image header is validated before handoff.
The loader reads `text_offset`, `image_size`, `flags`, and the `ARMd` magic. The
computed Image base (`load_address - text_offset`) must be 2 MiB-aligned. If the
header declares an `image_size` larger than the decompressed file but still
within the reserved kernel range, the gap is zero-filled; otherwise validation
fails with `BootError::LinuxImageInvalid`.

## I2C

The RP1 bootstrap code is bus-generic over a small `Rp1I2cBus` trait equivalent
to the required subset of `embedded-hal` I2C. The BCM2712 I2C controller driver
is implemented in this PoC as a polling MMIO driver with bounded timeouts.

The RP1 write packet format is:

```text
BE32(destination_address) + up to 0x40 bytes of data
```

Scratch registers are programmed before issuing the RP1 boot command.

The current `write_read` path may issue a write followed by a read rather than a
true repeated-start transaction. Firmware writes use plain I2C writes and are
the primary path. Chip-id read failures should first check this limitation and
the reset-clear sequence. The code has `RP1_PROBE_CHIP_ID_REQUIRED=true` by
default; during early bringup it can be changed to allow firmware writes to
continue after a probe read failure.

To avoid doubling this read-side risk, `reset_into_bootrom()` performs the
chip-id probe once and returns the result to the caller.

## Known Limits

- RP1 PCIe re-enumeration after firmware reload is not fully implemented in this
  PoC.
- `skip-rp1-reload` is available for Linux handoff isolation, but it does not
  validate the post-firmware RP1 state.
- DTB discovery for AON GPIO/I2C/SDHCI quiesce currently has documented fallback
  MMIO addresses; this should be tightened during hardware bringup.
- The gzip path currently inflates through an allocated `Vec` before copying to
  the fixed kernel destination.
- EL2 handoff writes `HCR_EL2=RW` only, `CPTR_EL2=0`, `CNTVOFF_EL2=0`, and
  `VTTBR_EL2=0`, but this register policy is still minimal and should be
  audited on real hardware. `VTCR_EL2`, timer state, RES bits, and any firmware
  trap configuration remain bringup risks.
- The final MMU-off sequence assumes identity-compatible execution during the
  `SCTLR_EL2.M` clear.

VPU bootmain flows such as `clear_rp1_cache_globals()`, PCIe2 reset/init, and
RP1 PCIe enumeration are useful references but are not required for the first
PoC goal: reload RP1 firmware, continue using BCM2712 SDHC, then boot Linux.

## Expected Log Order

The following order is the same for UART and semihosting backends:

```text
[BOOT] start EL2
[DTB] parse ok
[ALLOC] static bump allocator ok: size=...
[RP1] init_rp1 ok
[RP1] existing RP1 visible
[SDHC] init ok
[SD] /config.txt before reset ok: size=...
[SD] /RP1.img found / not found
[RP1IMG] source=RP1.img / fw-parts
[RP1BOOT] reset low
[RP1BOOT] reset high
[RP1BOOT] reset clear for chip-id probe
[RP1BOOT] i2c 0x43 ack ok
[RP1BOOT] chip id = ...
# If chip-id probing is made optional for bringup, this may instead be:
# [RP1BOOT] chip id unavailable; continuing with write-only bootstrap path
[RP1BOOT] image loaded
[RP1BOOT] scratch programmed
[RP1BOOT] proc0 started
[SD] /config.txt after reset ok: size=...
[SD] /kernel_2712.img ok: size=...
[KERNEL] gzip detected / not gzip
[KERNEL] Image header ok, entry=..., image_size=..., text_offset=..., flags=..., image_base=...
[SD] /initramfs_2712 ok: size=...
[SDHC] quiesce begin
[SDHC] quiesce done
[DTB] /chosen bootargs set: len=...
[DTB] patched output addr=..., size=..., aligned8=true, max=...
[DTB] /chosen linux,initrd-start=..., linux,initrd-end=...
[LINUX] handoff kernel entry=..., image_size=..., text_offset=..., flags=..., image_base=...
[LINUX] handoff dtb=..., len=..., initrd=...
[LINUX] EL2 regs before handoff DAIF=..., CurrentEL=..., SCTLR_EL2=..., HCR_EL2=..., VTTBR_EL2=..., CNTVOFF_EL2=..., CPTR_EL2=...
[LINUX] jumping at EL2
```
