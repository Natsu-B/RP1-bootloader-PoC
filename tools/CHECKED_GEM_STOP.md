# Checked GEM stop

The pinned SoC dependency99b8c563daef60ef1da639474aa32379f2370a84 returns
`Result<u32, Rp1GemError>` from its stop/handoff operations. The successful
value is the observed NCR; timeout carries the last NCR. After1000 unsuccessful
reads, firmware reload and Linux jump are refused and the singleton/DMA storage
remain owned. This is a finite read-count bound, not a calibrated time deadline.

The existing writes are unchanged. The readback requires RE,TE,TSTART,MPE clear
(`NCR & 0x21c == 0`). No ISR read, unknown register or reset/POWER write was added.
Control bits clear does **not** prove AXI/PCIe transactions have drained.

Build the existing bare-metal reader with tools/build-rtos-reader.sh and its
fixed nightly-2026-08-27 target/flags. Linux source, configuration and kernel
are not modified. This source/build improvement is not an on-hardware result;
it does not admit watchdog expiry or resolve unexpected platform resets.

The existing workspace path patches for io-api/net remain explicit build inputs;
capture their source state as well as Cargo.lock when reproducing a build.
No source change to those local dependencies is implied by this patch.
