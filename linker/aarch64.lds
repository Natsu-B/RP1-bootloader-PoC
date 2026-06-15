OUTPUT_ARCH(aarch64)
ENTRY(_start)

PHDRS
{
    text PT_LOAD FLAGS(0x5);
    rodata PT_LOAD FLAGS(0x4);
    data PT_LOAD FLAGS(0x6);
}

_STACK_SIZE = 0x100000;
_STACK_GUARD_SIZE = 0x1000;

SECTIONS {
  . = 0x80000;
  __ram_start = .;
  _PROGRAM_START = .;

  .text.boot : {
    KEEP(*(.text.boot .text.boot.*))
  } :text

  .text ALIGN(8) : {
    __text_start = .;
    *(.text .text.*)
    __text_end = .;
  } :text

  .rodata ALIGN(64) : {
    __rodata_start = .;
    __el2_tls_start = .;
    KEEP(*(.el2_tls .el2_tls.*))
    __el2_tls_end = .;
    __el2_tls_size = __el2_tls_end - __el2_tls_start;
    *(.rodata .rodata.*)
    __rodata_end = .;
  } :rodata

  .data ALIGN(8) : {
    __data_start = .;
    *(.data .data.*)
    *(.got .got.*)
    __data_end = .;
  } :data

  .bss ALIGN(8) (NOLOAD) : {
    __bss_start = .;
    _BSS_START = .;
    *(.bss .bss.*)
    *(COMMON)
    . = ALIGN(64);
    __el2_tls_bsp_start = .;
    . = . + __el2_tls_size;
    __el2_tls_bsp_end = .;
    . = ALIGN(8);
    _BSS_END = .;
    __bss_end = .;
  } :data

  . = ALIGN(0x1000);
  _STACK_BOTTOM = .;
  . = . + _STACK_GUARD_SIZE;
  . = . + _STACK_SIZE;
  _STACK_TOP = ALIGN(16);
  _PROGRAM_END = .;
  __ram_end = _STACK_TOP;

  .debug_abbrev 0 : { *(.debug_abbrev) }
  .debug_info 0 : { *(.debug_info .gnu.linkonce.wi.*) }
  .debug_aranges 0 : { *(.debug_aranges) }
  .debug_ranges 0 : { *(.debug_ranges) }
  .debug_rnglists 0 : { *(.debug_rnglists) }
  .debug_line 0 : { *(.debug_line) }
  .debug_line_str 0 : { *(.debug_line_str) }
  .debug_str 0 : { *(.debug_str) }
  .debug_loc 0 : { *(.debug_loc) }
  .debug_loclists 0 : { *(.debug_loclists) }
  .debug_frame 0 : { *(.debug_frame) }

  /DISCARD/ : {
    *(.comment)
    *(.note*)
    *(.eh_frame*)
  }

  ASSERT(_PROGRAM_START == 0x80000, "PROGRAM must start at 0x80000")
  ASSERT(_PROGRAM_START < _PROGRAM_END, "PROGRAM range invalid")
  ASSERT(_BSS_START <= _BSS_END, "BSS range invalid")
  ASSERT(_STACK_BOTTOM + 0x10000 <= _STACK_TOP, "STACK shortage")
  ASSERT((_STACK_BOTTOM & 0xFFF) == 0, "STACK_BOTTOM must be 4KB aligned")
  ASSERT((_STACK_TOP & 0xF) == 0, "STACK_TOP not 16-byte aligned")
  ASSERT(_STACK_TOP < 0x3000000, "STACK_TOP must be below Linux image")
  ASSERT(_LINUX_IMAGE == 0x3000000, "Linux image base changed")
  ASSERT(__el2_tls_bsp_end >= __el2_tls_bsp_start, "TLS BSP range invalid")

  . = 0x3000000;
  _LINUX_IMAGE = .;
}
