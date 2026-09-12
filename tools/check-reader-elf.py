#!/usr/bin/env python3
"""Check the AArch64 reader's allocated sections and startup memory contract."""
import json
import re
import struct
import subprocess
import sys
from pathlib import Path


def check(path):
    header = Path(path).read_bytes()[:64]
    assert header[:6] == b'\x7fELF\x02\x01', 'expected little-endian ELF64'
    assert struct.unpack_from('<HH', header, 16) == (2, 183), 'expected AArch64 EXEC'
    assert struct.unpack_from('<Q', header, 24)[0] == 0x80000, 'entry changed'
    text = subprocess.check_output(['llvm-readelf', '-SW', '-sW', str(path)], text=True)
    sections = {}
    for line in text.splitlines():
        match = re.match(r'\s*\[\s*\d+\]\s+(\S+)\s+\S+\s+([0-9a-fA-F]+)\s+[0-9a-fA-F]+\s+([0-9a-fA-F]+)\s+\S+\s+([A-Z]+)\s+\d+\s+\d+\s+(\d+)\s*$', line)
        if not match:
            continue
        name, address, size, flags, alignment = match.groups()
        if 'A' in flags:
            address, size, alignment = int(address, 16), int(size, 16), int(alignment)
            assert alignment and address % alignment == 0, f'{name}: bad alignment'
            sections[name] = (address, size)
    assert {'.text.boot', '.text', '.rodata', '.data', '.bss'} <= sections.keys(), 'missing section'
    ranges = sorted((start, start + size, name) for name, (start, size) in sections.items())
    assert all(a[1] <= b[0] for a, b in zip(ranges, ranges[1:])), 'section overlap'
    symbols = {}
    for line in text.splitlines():
        fields = line.split()
        if len(fields) == 8 and fields[0].endswith(':') and fields[0][:-1].isdigit():
            symbols[fields[7]] = int(fields[1], 16)
    start, size = sections['.bss']
    assert symbols['_BSS_START'] == start and symbols['_BSS_END'] == start + size
    assert start % 64 == 0 and (start + size) % 8 == 0, 'startup zero-loop alignment'
    bottom, top = symbols['_STACK_BOTTOM'], symbols['_STACK_TOP']
    assert start + size <= bottom and bottom % 4096 == 0
    assert top - bottom == 0x101000 and top % 16 == 0, 'stack/guard budget changed'
    assert symbols['_PROGRAM_START'] == 0x80000 and symbols['_PROGRAM_END'] == top
    assert top < symbols['_LINUX_IMAGE'] == 0x6000000, 'Linux image overlap'
    return dict(status='PASS', bss_start=start, bss_bytes=size, stack_bottom=bottom,
                stack_top=top, stack_guard_bytes=0x1000, stack_bytes=0x100000,
                allocated_sections=len(sections))


if __name__ == '__main__':
    if len(sys.argv) != 2:
        raise SystemExit('usage: check-reader-elf.py reader.elf')
    print(json.dumps(check(sys.argv[1]), sort_keys=True))
