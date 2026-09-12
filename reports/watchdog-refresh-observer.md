# WDL1 bounded observer candidate

Explicit rp1-rtos-watchdog-refresh preserves the original WDT2/default modes.
For WDL1/version10 READY only, publish one exact WQL1 request to BAR2 SRAM
0x2000f800 +176*4..183*4: payload/checksum first, DSB, commit token last, DSB,
then exact readback. Diagnostic nonce folds the existing raw-timer word27 and
maps zero to1. It is not random/authenticated; a formal cohort rejects repeats.

Keep31x256word snapshots and the watchdog-request footer. No retry, fallback,
quiescence ACK, later long arm, peripheral write or generic MMIO transport.
This is the existing bare-metal CM5 observer, not a modified Linux kernel.
The whole SRAM read is non-atomic and has no bus-access fault/timeout contract.

Source derives from verified bounded observer3581cf2; original branch/WIP is
unchanged. New source/build/HW admission is separate. Do not reuse the old
seven-file/absent-config_rp1.txt harness against the current eight-file baseline.
The current recovery contract must be retained for any hardware experiment.

## Allocator/link alignment prerequisite

Reader02 exposed a pre-existing allocator bug: it aligned only the heap-relative
offset. A heap base ending in8 cannot satisfy alignment16 even at offset0.
The common GlobalAlloc path now aligns the absolute address with checked
arithmetic and then converts back to an in-bounds offset. This covers every
allocator caller and does not rely on a particular linker placement. The
single-core/no-concurrent-allocator contract is unchanged; no atomics added.
The independent .bss output-section alignment is64, with a linker ASSERT.
The existing DMA_STORAGE object's alignment must also be checked in the ELF.

The build runs the actual pure alignment helper's host test:64 heap bases,
5 offsets,21 power-of-two alignments, the reader02 misalignment regression,
capacity/overflow/invalid-alignment rejection. These are STATIC/BUILD evidence,
not proof of hardware allocator behavior. Reader02 must not be deployed.
