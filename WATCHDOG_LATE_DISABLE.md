# WDT5 late-disable observer — candidate

Build with `RP1_RTOS_READER_FEATURE=rp1-rtos-watchdog-late-disable` using the
normal reader build script. It selects WDT5/WQ05/QA05 version5/sequence1, otherwise
the same checked disabled receipt and final post-reload NCR gate as WDT4.

ACK remains the final RP1 access, no readback. Selected caller then halts; only
host UART logs follow ACK. This is not a bus/DMA-drain proof. The paired firmware
performs a15s still-counting/explicit-disable positive control and external A618
terminal event, not count0 or reset. Independent ESP recovery is mandatory.
No Linux C/config/kernel/module/daemon or new hardware is required or changed.
