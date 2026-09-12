# WDT4 final host observation ACK — candidate

`RP1_RTOS_READER_FEATURE=rp1-rtos-watchdog-postack tools/build-rtos-reader.sh /new/absolute/output`
selects the version4 WDT4/WQ04/QA04 peer for the proc0 FreeRTOS candidate.
The unchanged default WDT3 feature remains available.

The observer checks the same disabled256us receipt, current-map GEM NCR required
clear mask0x21c, and request/progress fields. It publishes the eight-word ACK
with magic last and DSB. After ACK it logs only through host UART and returns
to the immediate halt path. No post-ACK RP1 readback or telemetry is permitted.
This is source/caller evidence, not general PCIe/DMA drain instrumentation.

The firmware may arm a separate bounded long interval only after this handoff.
External GPIO events and independent ESP timeout/recovery are required. Host
silence, lost endpoint or reboot of a separate recovery image are not themselves
watchdog reset proof. This feature alone is not hardware experiment admission.
No Linux C, configuration, kernel, module or userspace mediator is changed.
