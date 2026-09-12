# Opt-in warm SRAM record observer

`rp1-rtos-warm-late-record` is a separate experimental observer, not a change to
the existing warm-guard no-post-ACK-access contract. It inherits the WDT9 request
and checked ACK, then uses the established BAR2 translation to read only
`0x2000f800..0x2000fc00`. It does not reinitialize PCIe or issue another RP1 write.
The complete run still has the original pre-tail request/ACK writes.

Build with the pinned repository toolchain/lockfile and sufficient Cargo cache
capacity (including eager git submodules even if not compiled):

```sh
RP1_RTOS_READER_FEATURE=rp1-rtos-warm-late-record \
  CARGO_TARGET_DIR=/absolute/disposable-target \
  bash tools/build-rtos-reader.sh /absolute/new-output
python3 -B tools/check-warm-late-structure.py
python3 -B tools/check-warm-late-record.py --self-test
```

A task-local `CARGO_HOME` may be supplied for a fresh pinned offline checkout;
the required registry/git object database must already be available. Do not
delete a shared cache or alter dependency revisions to hide checkout failures.
RAM/disk free blocks alone do not establish available user quota.

The two samples start at ACK+120/122seconds using the local architectural
counter. Each has12 identity loads,256 aligned volatile words and12 trailing
identity loads, with barriers and local begin/end counters. Arrays are local;
UART output after the reads uses those arrays. WQLATE identifies the new path;
the old `no-more-rp1-access=1` footer is never emitted by this feature.

The decoder is for the selected AZ KRN8/AZ01 warm schema. Pass expected nonce,
warm .data byte length and FNV1a from the independently fixed M3 ELF:

```sh
python3 -B tools/check-warm-late-record.py capture.txt NONCE DATA_BYTES DATA_FNV
```

It checks six compact receipts, actual task/MSP watermarks, quiet IRQ totals,
reserved/fault fields, stable identity and advancing counters. It refuses the
selected window near local u64 counter wrap; u32 progress uses half-range
comparison. A passing result alone is **not hardware acceptance**: join current
source/image identity, same-run external payload/type8 trace and final recovery.

Fixed delay is not completion notification, a bus timeout or a liveness
guarantee. A stalled endpoint may hang a load. Stable identity brackets do not
make the whole record atomic. Host log/trace observation ordering has UART/USB
timestamp uncertainty. Two returned advancing samples demonstrate only sampled
BAR2 reachability/progress, not uninterrupted link operation through reset,
Linux usability, full R1/R2/R3 completion or safe autonomous watchdog feeding.
