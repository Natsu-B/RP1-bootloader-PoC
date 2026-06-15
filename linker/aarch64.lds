OUTPUT_FORMAT(elf64-aarch64)
OUTPUT_ARCH(aarch64)
ENTRY(_start)

PHDRS
{
    headers PT_PHDR PHDRS;
    text PT_LOAD FILEHDR PHDRS;
    rodata PT_LOAD;
    data PT_LOAD;
}

_ALIGN_SIZE = 0x1000;
_RAM_BASE = 0x80000;
_STACK_SIZE = 0x100000;
_STACK_GUARD_SIZE = 0x1000;
_LINUX_IMAGE = 0x3000000;

SECTIONS {
  . = _RAM_BASE;
  __ram_start = _RAM_BASE;
  _PROGRAM_START = .;
  . = . + SIZEOF_HEADERS;

  .text.boot : ALIGN(_ALIGN_SIZE) {
    KEEP(*(.text.boot .text.boot.*))
  } :text

  .text : ALIGN(_ALIGN_SIZE) {
    __text_start = .;
    *(.text .text.*)
    __text_end = .;
  } :text

  .rodata : ALIGN(_ALIGN_SIZE) {
    __rodata_start = .;
    __el2_tls_start = .;
    KEEP(*(.el2_tls .el2_tls.*))
    __el2_tls_end = .;
    __el2_tls_size = __el2_tls_end - __el2_tls_start;
    *(.rodata .rodata.*)
    __rodata_end = .;
  } :rodata

  .data : ALIGN(_ALIGN_SIZE) {
    __data_start = .;
    *(.data .data.*)
    __data_end = .;
  } :data

  .bss (NOLOAD) : ALIGN(_ALIGN_SIZE) {
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

  .got : ALIGN(_ALIGN_SIZE) {
    *(.got .got.*)
  } :data

  _PROGRAM_END = ALIGN(_ALIGN_SIZE);
  . = _PROGRAM_END + _STACK_GUARD_SIZE;
  _STACK_BOTTOM = .;
  . = . + _STACK_SIZE;
  _STACK_TOP = ALIGN(16);
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
  ASSERT(_PROGRAM_END <= _STACK_BOTTOM, "PROGRAM overlaps STACK")
  ASSERT(_STACK_BOTTOM + 0x10000 <= _STACK_TOP, "STACK shortage")
  ASSERT((_STACK_TOP & 0xF) == 0, "STACK_TOP not 16-byte aligned")
  ASSERT(_STACK_TOP < 0x3000000, "STACK_TOP must be below Linux image")
  ASSERT(_LINUX_IMAGE == 0x3000000, "Linux image base changed")
  ASSERT(__el2_tls_bsp_end >= __el2_tls_bsp_start, "TLS BSP range invalid")
}
