#!/usr/bin/env python3
"""Cold-only SCMI receipts; run unchanged R1/readonly validators separately."""
import json
from pathlib import Path
import re
import sys

SEALED_SHA256 = '088048933674965bc5c3fc7be16b4e6196ec71a5f11712da6a8b34b54eb70e6e'
COLD = [0x31494353, 1, 2, 1] + [0] * 15 + [0x20000000, 0x200001c1, 0x2000d200, 0xc0, 1] + [0] * 3
CLOCK = [0x80000001, 4, 20, 0, 0x51010, 0x10000840, 1, 1]
BEGIN = '[SCMI] observer-begin address=20009df0 bytes=108 attempts=4 samples=31'
END = '[SCMI] observer-complete cold-only=1'
RTOS_END = '[RTOS] observer-complete read-only=1'


def clock_line(phase, copy):
    return f'[SCMICLK] scmi-cold-{phase} read={copy} ' + ' '.join(f'{w:08x}' for w in CLOCK)


def row(sample, offset, words):
    return f'[SCMI] {sample} {offset:03} ' + ' '.join(f'{w:08x}' for w in words)


def check(text):
    assert re.findall(r'\[RP1ELF\] file_sha256=([^\r\n]+)', text) == [SEALED_SHA256], 'sealed ELF receipt'
    assert text.count(RTOS_END) == 1, 'read-only completion'
    assert not any(x in text for x in ('[WDT2]', '[WQ3]', 'read-only=0')), 'write-enabled observer'
    receipts = list(re.finditer(r'\[(?:SCMI|SCMICLK)\][^\r\n]*', text))
    expected = [BEGIN, clock_line('before', 0), clock_line('before', 1)]
    for sample in range(31):
        index = len(expected)
        assert index < len(receipts), 'missing SCMI record'
        begin = re.fullmatch(r'\[SCMI\] sample=(\d+) begin seq_before=00000002 seq_after=00000002 attempts=([1-4])', receipts[index][0])
        assert begin and int(begin[1]) == sample, 'SCMI seqlock receipt/order'
        expected.append(begin[0])
        expected.extend(row(sample, i, COLD[i:i + 3]) for i in range(0, 27, 3))
        expected.append(f'[SCMI] sample={sample} end')
        # Pair the added snapshot with the untouched RTOS sample, in order.
        rtos_begin = f'[RTOS] sample={sample} begin'
        rtos_end = f'[RTOS] sample={sample} end'
        assert text.count(rtos_begin) == text.count(rtos_end) == 1, 'RTOS sample markers'
        assert receipts[index - 1].end() < text.index(rtos_begin) < text.index(rtos_end) < receipts[index].start(), 'sample chronology'
    expected.extend([clock_line('after', 0), clock_line('after', 1), END])
    assert [m[0] for m in receipts] == expected, 'cold telemetry/clock tuple, missing/extra receipt'
    assert receipts[-2].end() < text.index(RTOS_END) < receipts[-1].start(), 'completion chronology'
    return dict(result='SCMI_COLD_RECEIPTS_PASS', samples=31, seqlock_attempt_limit=4,
                telemetry_bytes=108, apb_hz=100_000_000, uart_hz=50_000_000,
                irq_delivery_proven=False, linux_transport_proven=False,
                endpoint_reset_survival_proven=False, full_clock_profile_proven=False)


def self_test():
    lines = [f'[RP1ELF] file_sha256={SEALED_SHA256}', BEGIN,
             clock_line('before', 0), clock_line('before', 1)]
    for sample in range(31):
        lines.extend([f'[RTOS] sample={sample} begin', f'[RTOS] sample={sample} end',
                      f'[SCMI] sample={sample} begin seq_before=00000002 seq_after=00000002 attempts=1'])
        lines.extend(row(sample, i, COLD[i:i + 3]) for i in range(0, 27, 3))
        lines.append(f'[SCMI] sample={sample} end')
    lines.extend([clock_line('after', 0), clock_line('after', 1), RTOS_END, END])
    good = '\n'.join(lines) + '\n'
    assert check(good)['samples'] == 31
    assert check(good.replace('attempts=1', 'attempts=4'))['samples'] == 31
    bad = [good + good, good.replace(SEALED_SHA256, '0' * 64),
           good.replace('seq_before=00000002', 'seq_before=00000003', 1),
           good.replace('seq_after=00000002', 'seq_after=00000004', 1),
           good.replace('attempts=1', 'attempts=5', 1),
           good.replace('attempts=1', 'attempts=0', 1),
           good.replace(RTOS_END, ''), good.replace(END, ''),
           good + '[WDT2] request\n', good + '[WQ3]\n', good + '[SCMI] failure=bounded-seqlock\n',
           good.replace('[RTOS] sample=30 end', ''),
           good.replace('[SCMI] sample=30 end', '[SCMI] sample=29 end'),
           good.replace(clock_line('after', 1), ''),
           good.replace(clock_line('before', 0), clock_line('after', 0)),
           good.replace(clock_line('after', 1), clock_line('after', 1) + ' extra')]
    for index in range(27):
        offset = index // 3 * 3
        words = COLD[offset:offset + 3]
        words[index % 3] ^= 1
        bad.append(good.replace(row(12, offset, COLD[offset:offset + 3]), row(12, offset, words)))
    for phase in ('before', 'after'):
        for copy in (0, 1):
            for index in range(8):
                words = CLOCK.copy()
                words[index] ^= 1
                corrupt = f'[SCMICLK] scmi-cold-{phase} read={copy} ' + ' '.join(f'{w:08x}' for w in words)
                bad.append(good.replace(clock_line(phase, copy), corrupt))
    for line in lines[1:]:
        bad.extend([good.replace(line + '\n', '', 1), good.replace(line + '\n', line + '\n' + line + '\n', 1)])
    for text in bad:
        try:
            check(text)
        except AssertionError:
            pass
        else:
            raise AssertionError('negative accepted')
    return dict(result='PASS', positive=2, negative=len(bad))


if __name__ == '__main__':
    if not __debug__:
        raise SystemExit('assertions required; refuse -O/PYTHONOPTIMIZE')
    if sys.argv[1:] in (['--selftest'], ['--self-test']):
        result = self_test()
    else:
        assert len(sys.argv) == 2, 'UART log or --selftest'
        result = check(Path(sys.argv[1]).read_text(errors='replace'))
    print(json.dumps(result, indent=2))
