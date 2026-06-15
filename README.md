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

The Image entry address is the start of the decompressed/copied Image at
`0x03000000`. `text_offset` is treated as placement metadata, not as an entry
offset.

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
`0x03000000`.

Non-gzip input is copied directly as an arm64 Image.

## I2C

The RP1 bootstrap code is bus-generic over a small `Rp1I2cBus` trait equivalent
to the required subset of `embedded-hal` I2C. The BCM2712 I2C controller driver
is implemented in this PoC as a polling MMIO driver with bounded timeouts.

The RP1 write packet format is:

```text
BE32(destination_address) + up to 0x40 bytes of data
```

Scratch registers are programmed before issuing the RP1 boot command.

## Known Limits

- RP1 PCIe re-enumeration after firmware reload is not fully implemented in this
  PoC.
- DTB discovery for AON GPIO/I2C/SDHCI quiesce currently has documented fallback
  MMIO addresses; this should be tightened during hardware bringup.
- The gzip path currently inflates through an allocated `Vec` before copying to
  the fixed kernel destination.
- EL2 handoff system register policy is minimal and should be audited on real
  hardware, especially `HCR_EL2`, `SCTLR_EL2`, `VTTBR_EL2`, timer state, and
  any trap bits inherited from earlier firmware.

VPU bootmain flows such as `clear_rp1_cache_globals()`, PCIe2 reset/init, and
RP1 PCIe enumeration are useful references but are not required for the first
PoC goal: reload RP1 firmware, continue using BCM2712 SDHC, then boot Linux.

## Expected UART Order

```text
[BOOT] start EL2
[DTB] parse ok
[ALLOC] init ok
[PCIE] init ok
[RP1] existing RP1 visible
[SDHC] init ok
[SD] /config.txt before reset ok: size=...
[SD] /RP1.img found / not found
[RP1IMG] source=RP1.img / fw-parts
[RP1BOOT] reset low
[RP1BOOT] reset high
[RP1BOOT] i2c 0x43 ack ok
[RP1BOOT] chip id = ...
[RP1BOOT] image loaded
[RP1BOOT] scratch programmed
[RP1BOOT] proc0 started
[SD] /config.txt after reset ok: size=...
[SD] /kernel_2712.img ok: size=...
[KERNEL] gzip detected / not gzip
[KERNEL] Image header ok, entry=...
[SD] /initramfs_2712 ok: size=...
[DTB] initrd-start=..., initrd-end=...
[SDHC] quiesce begin
[SDHC] quiesce done
[LINUX] jumping at EL2
```
