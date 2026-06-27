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

## Timer

Delay handling uses the `timer` crate from `aarch64_type1_hypervisor`, imported
as `arch_timer`. The PoC initializes `SystemTimer`, reads `CNTFRQ_EL0`, and uses
`CNTPCT_EL0` polling through `SystemTimer::wait(Duration)`.

This replaces the old NOP-loop delay, which was not stable under OpenOCD/SWD or
semihosting.

Current RP1 reset timing:

- `RP1_RESET_LOW_US = 50_000`
- `RP1_RESET_HIGH_SETTLE_US = 10_000`

If RP1 does not reliably enter I2C bootstrap mode, increase
`RP1_RESET_LOW_US` first.

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

- `/RP1.elf` (ELF32 little-endian ARM executable)
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

## RP1 ELF and image formats

`/RP1.elf` is preferred when present. The loader accepts only ELF32
little-endian ARM `ET_EXEC` files, materializes `PT_LOAD` segments by their
`p_paddr` values into the RP1 scratch image beginning at `0x20000000`, and
zero-fills holes and `.bss` tails. `PT_INTERP`, `PT_DYNAMIC`, overlap, invalid
alignment, and out-of-range segments are fatal. The generic ELF layer does not
invent a stack or alter the entry; the PoC applies the configured fallback
stack and sets the Thumb bit before RP1 bootstrap.

Search order is `/RP1.elf`, `/rp1/RP1.elf`, `/rp1/rp1.elf`,
`/RP1/RP1.ELF`, then the existing `/RP1.img` paths, then fw-parts when
permitted. A present but invalid `/RP1.elf` is fatal and is never silently
replaced by another firmware source.

### RP1.elf note policy

`/RP1.elf` may carry a `.note.rp1` boot note. The current PoC parses the note
metadata before materializing `PT_LOAD` segments and uses the owner bitmap when
patching the Linux handoff DTB.

- `.note.rp1` valid:
  - boot normally
  - log owner bitmap, mailbox flags, and firmware version kind
  - use note metadata for the DTB ownership policy

- `.note.rp1` missing:
  - require `/config_rp1.txt` with `force_boot = true`
  - otherwise refuse RP1 ELF boot

- `.note.rp1` invalid:
  - always refuse boot
  - `force_boot = true` is ignored

Minimal legacy fallback config:

```text
force_boot = true
linux_pio = false
```

`linux_pio = true` is reserved for a future RP1 PIO firmware mode and is
currently rejected as an invalid config.

### Firmware boot context policy

The bootloader reads firmware-provided boot context from `/chosen/bootloader`
in the input DTB before selecting a default-feature boot path.

- `/chosen/bootloader/boot-mode` is required for default feature builds and is
  parsed as a big-endian `u32`
- `/chosen/bootloader/partition` is parsed as an optional big-endian `u32`
- boot mode `1` (`sd-emmc`) is fatal in default builds:
  `FirmwareBootedFromSdOrEmmc`
- boot mode `4` (`usb-msd`) is fatal in default builds:
  `FirmwareBootedFromUsbMsd`
- boot mode `2` (`network`) is allowed in default builds
- `rpiboot`, `nvme`, `http`, unknown, missing, or malformed boot modes are
  rejected in default builds

TFTP server IP is logged when found in one of these DTB properties:

- `/chosen/bootloader/tftp`
- `/chosen/bootloader/tftp-ip`
- `/chosen/bootloader/tftp-server`
- `/chosen/bootloader/server-ip`
- `/chosen/tftp`
- `/chosen/tftp-ip`
- `/chosen/tftp-server`
- `/chosen/server-ip`

The IP parser accepts either a big-endian four-byte IPv4 value or an ASCII IPv4
string with an optional trailing NUL. Missing TFTP IP is logged as
`tftp_ip=missing` and is not fatal.

### RP1 DTB ownership policy

When `/RP1.elf` is used, the bootloader derives an RP1 ownership policy before
building the Linux handoff DTB.

- `.note.rp1` valid:
  - owner bitmap from note is used
  - `config_rp1.txt` owner table is ignored

- `.note.rp1` missing:
  - `force_boot = true` is required
  - `[owner]` table in `config_rp1.txt` is required

- `owner = rp1`:
  - matching Linux DTB node is disabled

- `owner = disabled`:
  - matching Linux DTB node is disabled

- `owner = linux`:
  - matching Linux DTB node is set to `okay`

- Linux PIO ownership is not supported yet

The current CM5 DTB node map is based on
`bcm2712-rpi-cm5l-cm5io.dtb`/`bcm2712-rpi-cm5l-cm4io.dtb`:

- `gpio`: `/axi/pcie@1000120000/rp1/gpio@d0000`
- `uart0`: `/axi/pcie@1000120000/rp1/serial@30000`
- `uart1`: `/axi/pcie@1000120000/rp1/serial@34000`
- `i2c0`: `/axi/pcie@1000120000/rp1/i2c@70000`
- `i2c1`: `/axi/pcie@1000120000/rp1/i2c@74000`
- `spi0`: `/axi/pcie@1000120000/rp1/spi@50000`
- `pio0`/`pio1`: `/axi/pcie@1000120000/rp1/pio@178000`
- `dma`: `/axi/pcie@1000120000/rp1/dma@188000`
- `timer`: no Linux-visible RP1 timer node in the tested DTB, so it is
  validated in the ownership bitmap but not patched

Minimal config fallback with ownership:

```text
force_boot = true
linux_pio = false

[owner]
gpio = "rp1"
uart0 = "rp1"
uart1 = "linux"
i2c0 = "linux"
i2c1 = "linux"
spi0 = "linux"
pio0 = "rp1"
pio1 = "disabled"
dma = "rp1"
timer = "rp1"
```

`/RP1.img` remains supported and has the following 32-byte little-endian
header:

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
`/RP1.elf` or `/RP1.img`; prefer either explicit image format over fw-parts.

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
nix develop
cargo run
cargo xbuild
cargo xrun
```

The dev shell also provides convenience wrappers:

```sh
build-bootloader
build-bootloader-tftp
```

Generated artifacts:

- `./bin/rp1_chainboot_poc.elf`
- `./bin/rp1_chainboot_poc.img`

`cargo build -p rp1_chainboot_poc --target aarch64-unknown-none-softfloat` builds
the ELF through Cargo. Stable Cargo has no clean post-link hook for raw image
generation, so raw image creation is intentionally handled by `xtask`. The
crate build script emits a warning pointing users at `cargo xbuild` and
`cargo xrun`.

## TFTP boot

The `tftp-boot` feature is an alternate boot source. It leaves the SDHC path
unchanged when disabled, but when enabled it initializes RP1 PCIe in `Auto`
mode, initializes `Rp1Gem`, resolves the configured server with ARP, downloads
`RP1.elf` and optional `config_rp1.txt` from the TFTP root, applies the
`.note.rp1` policy, reloads RP1 through the existing I2C bootstrap path,
downloads `BCM2712.img` through the reusable `net::tftp::download_into` client,
stages and decompresses it when necessary, validates the arm64 Image, patches
the DTB, quiesces GEM, cleans the handoff ranges, and enters the existing EL2
handoff path. The TFTP boot path does not require SDHC to be present. It never
continues after a failed or partial RP1 ELF or kernel download.

Network constants are intentionally kept at the top of
`rp1_chainboot_poc/src/net_boot.rs`. The current lab defaults are local
`192.168.50.25`, server `192.168.50.1`, RP1 filename `RP1.elf`, RP1 config
filename `config_rp1.txt`, and kernel filename `BCM2712.img`. Update these
values together for a different direct Ethernet network. The optional
`tftp-initramfs` feature additionally downloads `initramfs_2712` into the
existing initramfs placement range.

Build with the repository's nightly toolchain:

```sh
cargo +nightly xbuild --features tftp-boot
```

Inside `nix develop`, the `cargo` wrapper accepts the `+nightly` rustup-style
selector for compatibility with this command. `build-bootloader-tftp` is the
same build with the repository dev shell toolchain.

For a direct host link, use a TFTP root that contains the configured filenames,
then bind the server to the host Ethernet interface. `dnsmasq` can run without
a DNS service as follows:

```sh
sudo ip addr replace 192.168.50.1/24 dev <cm5-ethernet-iface>
sudo ip link set <cm5-ethernet-iface> up
sudo dnsmasq --keep-in-foreground --port=0 --interface=<cm5-ethernet-iface> \
  --bind-interfaces --enable-tftp --tftp-root=/tmp/rp1-tftp --log-queries
```

Capture `arp or udp` on the host interface during a hardware run. A valid TFTP
server chooses its own UDP transfer port after the RRQ; the client binds that
port after the first DATA packet and rejects later DATA from another port.

### RP1 note policy hardware smoke

CM5 Lite TFTP smoke testing on 2026-06-27 used TFTP root
`/opt/rpi-cm5-hack/tftpboot`, CM5 reboot command
`/opt/rpi-cm5-hack/scripts/cm5ctl.py force-boot`, and UART capture command
`/opt/rpi-cm5-hack/scripts/capture-uart.sh --uart10 /dev/cm5-uart10 --analyze`.
The test temporarily replaced `kernel_2712.img` with `rp1_chainboot_poc.img`,
added `BCM2712.img`, `RP1.elf`, and `config_rp1.txt`, then restored the original
TFTP root. The pre-test manifest is in
`/opt/rpi-cm5-hack/backups/rp1-note-policy-20260627-005533/manifest.txt`, and
the restore log is in
`/opt/rpi-cm5-hack/logs/20260627-011738-rp1-note-policy-tftp-restore/restore.log`.

Observed UART10 policy results:

- missing `.note.rp1` with `force_boot = true`:
  `[RP1NOTE] missing; legacy ELF boot allowed by /config_rp1.txt force_boot=true`
- missing `.note.rp1` with `force_boot = false`:
  `[RP1NOTE] missing; refusing legacy ELF boot without force_boot=true`
  followed by `[FATAL] MissingRp1Note`
- invalid `.note.rp1` with `force_boot = true`:
  `[RP1NOTE] invalid; refusing RP1 ELF boot` followed by
  `[FATAL] InvalidRp1Note`
- valid `.note.rp1`:
  `[RP1NOTE] valid: owner_rp1=0x343 owner_linux=0x34 owner_disabled=0x80 mailbox=0x0 version_kind=0`

The valid and missing-allowed tests reached RP1 I2C bootstrap and logged
`[RP1BOOT] proc0 started`. The smoke `RP1.elf` was a minimal test image, not a
real GEM firmware, so the later `BCM2712.img` TFTP download timed out after the
RP1 reload. That timeout is outside the note policy check.

### RP1 DTB ownership hardware smoke

CM5 Lite TFTP smoke testing on 2026-06-27 used TFTP root
`/opt/rpi-cm5-hack/tftpboot`, CM5 reboot command
`/opt/rpi-cm5-hack/scripts/cm5ctl.py force-boot`, and UART capture command
`/opt/rpi-cm5-hack/scripts/capture-uart.sh --uart10 /dev/cm5-uart10 --analyze`.
The test temporarily replaced `kernel_2712.img` with a
`tftp-boot skip-rp1-reload` `rp1_chainboot_poc.img`, added `BCM2712.img`,
`RP1.elf`, and `config_rp1.txt`, then restored the original TFTP root. The
pre-test manifest is in
`/opt/rpi-cm5-hack/backups/20260627-012832-dtb-policy/manifest.txt`, and the
restore log is in
`/opt/rpi-cm5-hack/logs/20260627-015144-rp1-dtb-policy-tftp-restore/restore.log`.

Observed UART10 results:

- valid `.note.rp1`:
  `/opt/rpi-cm5-hack/logs/20260627-013750-uart/uart10.log`
  logged `source=note`, `uart0 owner=rp1 status=disabled`, `uart1 owner=linux
  status=okay`, `pio0 owner=rp1 status=disabled`, and `pio1 owner=disabled
  status=disabled`
- missing `.note.rp1` with config owner table:
  `/opt/rpi-cm5-hack/logs/20260627-014731-uart/uart10.log`
  logged `source=config` with the same DTB status changes
- invalid owner table with `pio0 = "linux"`:
  `/opt/rpi-cm5-hack/logs/20260627-014906-uart/uart10.log`
  logged `[FATAL] Rp1DtbPolicyInvalid`
- missing owner table:
  `/opt/rpi-cm5-hack/logs/20260627-015026-uart/uart10.log`
  logged `[FATAL] Rp1ConfigInvalid`

### Firmware boot context hardware smoke

CM5 Lite TFTP smoke testing on 2026-06-27 used TFTP root
`/opt/rpi-cm5-hack/tftpboot`, CM5 reboot command
`/opt/rpi-cm5-hack/scripts/cm5ctl.py force-boot`, and UART capture command
`/opt/rpi-cm5-hack/scripts/capture-uart.sh --uart10 /dev/cm5-uart10 --analyze`.
The test temporarily replaced `kernel_2712.img` with a default-feature
`rp1_chainboot_poc.img` and restored it afterward from
`/opt/rpi-cm5-hack/backups/20260627-122208-bootctx-default-retry/files/kernel_2712.img.bak`.

Observed UART10 log:

```text
/opt/rpi-cm5-hack/logs/20260627-122220-uart/uart10.log
[BOOTCTX] boot_mode=2 source=network partition=0
[BOOTCTX] tftp_ip=missing
```

The default feature policy allowed the network boot context, then continued into
the existing SDHC path and stopped at `SdMount` after `SDHC init failed:
InvalidState`. No SD/eMMC or USB-MSD hardware smoke was run in this pass.

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
[TIMER] generic timer freq=... Hz
[DTB] parse ok
[ALLOC] static bump allocator ok: size=...
[RP1] init_rp1 ok
[RP1] existing RP1 visible
[SDHC] init ok
[SD] /config.txt before reset ok: size=...
[SD] /RP1.elf found / not found
[RP1NOTE] valid: owner_rp1=... owner_linux=... owner_disabled=... mailbox=... version_kind=...
# or, for legacy ELF only when explicitly allowed:
# [RP1NOTE] missing; legacy ELF boot allowed by /config_rp1.txt force_boot=true
[RP1ELF] load_base=... image_len=... entry=... stack=...
[RP1IMG] source=RP1.elf / RP1.img / fw-parts
[RP1BOOT] reset low
[RP1BOOT] reset low delay 50000 us
[RP1BOOT] reset high
[RP1BOOT] reset high settle 10000 us
[RP1BOOT] reset clear ok after ... retries
[RP1BOOT] reset clear for chip-id probe
[RP1BOOT] i2c 0x43 ack ok after ... retries
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
