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
  __ram_start = ABSOLUTE(.);
  _PROGRAM_START = ABSOLUTE(.);

  .text.boot : {
    KEEP(*(.text.boot))
    KEEP(*(.text.boot.*))
  } :text

  .text ALIGN(8) : {
    __text_start = ABSOLUTE(.);
    *(.text .text.*)
    __text_end = ABSOLUTE(.);
  } :text

  .rodata ALIGN(64) : {
    __rodata_start = ABSOLUTE(.);
    __el2_tls_start = ABSOLUTE(.);
    KEEP(*(.el2_tls .el2_tls.*))
    __el2_tls_end = ABSOLUTE(.);
    __el2_tls_size = __el2_tls_end - __el2_tls_start;
    *(.rodata .rodata.*)
    __rodata_end = ABSOLUTE(.);
  } :rodata

  .data ALIGN(8) : {
    __data_start = ABSOLUTE(.);
    *(.data .data.*)
    *(.got .got.*)
    __data_end = ABSOLUTE(.);
  } :data

  .bss ALIGN(8) (NOLOAD) : {
    *(.bss .bss.*)
    *(COMMON)
    . = ALIGN(64);
    . = . + __el2_tls_size;
    . = ALIGN(8);
  } :data

  __bss_start = ADDR(.bss);
  _BSS_START = ADDR(.bss);
  __bss_end = ADDR(.bss) + SIZEOF(.bss);
  _BSS_END = ADDR(.bss) + SIZEOF(.bss);
  __el2_tls_bsp_start = _BSS_END - __el2_tls_size;
  __el2_tls_bsp_end = _BSS_END;

  . = ALIGN(_BSS_END, 0x1000);
  _STACK_BOTTOM = ABSOLUTE(.);
  . = . + _STACK_GUARD_SIZE;
  . = . + _STACK_SIZE;
  _STACK_TOP = ABSOLUTE(ALIGN(16));
  _PROGRAM_END = ABSOLUTE(.);
  __ram_end = _STACK_TOP;

  .debug_abbrev 0 : { *(.debug_abbrev) } :NONE
  .debug_info 0 : { *(.debug_info .gnu.linkonce.wi.*) } :NONE
  .debug_aranges 0 : { *(.debug_aranges) } :NONE
  .debug_ranges 0 : { *(.debug_ranges) } :NONE
  .debug_rnglists 0 : { *(.debug_rnglists) } :NONE
  .debug_line 0 : { *(.debug_line) } :NONE
  .debug_line_str 0 : { *(.debug_line_str) } :NONE
  .debug_str 0 : { *(.debug_str) } :NONE
  .debug_loc 0 : { *(.debug_loc) } :NONE
  .debug_loclists 0 : { *(.debug_loclists) } :NONE
  .debug_frame 0 : { *(.debug_frame) } :NONE

  /DISCARD/ : {
    *(.comment)
    *(.note*)
    *(.eh_frame*)
  }

  . = 0x6000000;
  _LINUX_IMAGE = ABSOLUTE(.);

  ASSERT(_PROGRAM_START == 0x80000, "PROGRAM must start at 0x80000")
  ASSERT(_PROGRAM_START < _PROGRAM_END, "PROGRAM range invalid")
  ASSERT(_BSS_START <= _BSS_END, "BSS range invalid")
  ASSERT(_STACK_BOTTOM + 0x10000 <= _STACK_TOP, "STACK shortage")
  ASSERT((_STACK_BOTTOM & 0xFFF) == 0, "STACK_BOTTOM must be 4KB aligned")
  ASSERT((_STACK_TOP & 0xF) == 0, "STACK_TOP not 16-byte aligned")
  ASSERT(_STACK_TOP < _LINUX_IMAGE, "STACK_TOP must be below Linux image")
  ASSERT(_LINUX_IMAGE == 0x6000000, "Linux image base changed")
  ASSERT(__el2_tls_bsp_end >= __el2_tls_bsp_start, "TLS BSP range invalid")
}
