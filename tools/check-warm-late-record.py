#!/usr/bin/env python3
"""Decode two WQLATE samples of the fixed AZ warm schema, not whole HW admission.

Caller must separately join image/source identity, current nonce, full type8,
external peer, causal pre-read completion and recovery. Individual volatile
words/identity brackets are not an atomic 256-word snapshot.
"""
import json
from pathlib import Path
import re
import sys

assert __debug__
IDENTITY = [0,1,2,3,4,96,97,98,99,100,124,125]
STACK_WORDS = [512,128,128,256,256,256,256,512]
HWM = [32,33,34,35,36,37,38,160]
PROGRESS = [8,9,14,15,17,49,50,64,80]
IPSRS = [35,41,24]
LIMITS = [5,84,132]
DIGESTS = [[0x6db986cd,0x6ab98214],[0x9e17d962,0x99906dc7],[0x41eb6616,0x7553f956]]

def require(ok, message):
    if not ok:
        raise ValueError(message)

def delta(a, b):
    value = (b-a) & 0xffffffff
    require(0 < value < 0x80000000, 'stopped/reversed/ambiguous progress')
    return value

def receipt_indices(slot):
    return [i + (i >= 160) for i in range(126+7*slot,133+7*slot)]

def check_words(w, nonce, warm_len, warm_digest):
    require(len(w) == 256 and all(type(v) is int and 0 <= v <= 0xffffffff for v in w), 'word shape')
    require(w[:5] == [0x31305452,1,5,0,0], 'RT01/stage/fault')
    require(w[96:101] == [0x384e524b,nonce,1,warm_len,warm_digest], 'warm epoch/reason/data identity')
    require(w[124:126] == [0x31305a41,0x3f07], 'AZ01/incomplete owner')
    require(w[19] == 0x13579bdf and w[20] == 0 and w[39] == 1, 'startup/sync sentinels')
    require(not any(w[i] for i in [56,57,58,59,70,86,183]), 'fault/context error')
    # RFT1 publishes its body before magic192; reject partial fault publication too.
    require(not any(w[105:108]+w[184:256]), 'warm reserved/fault publication')
    require(w[14] >= 5 and w[8] >= 5000 and w[9] >= 5000, 'warm progress gate')
    require(w[101] >= 5000 and w[102] >= 5000 and w[103] >= 20 and w[104] >= 10, 'latched warm gate')
    require(0 < w[18] <= 3968 and w[18] % 4 == 0, 'MSP watermark')
    for i, capacity in zip(HWM, STACK_WORDS):
        require(32 <= w[i] <= capacity, 'task watermark')
    contexts = [w[28:32], w[65:69], w[81:85], w[175:178]]
    for c in contexts:
        require(c[0] == 0 and c[1] & 3 == 2 and c[2] % 8 == 0 and 0x20000000 <= c[2] < 0x2000e000, 'task IPSR/CONTROL/PSP')
        if len(c) == 4:
            require(c[3] % 8 == 0 and 0x2000e000 <= c[3] <= 0x2000f000, 'MSP range')
    require(len({c[2] for c in contexts}) == len(contexts), 'distinct PSPs')
    receipts = []
    for slot in range(6):
        k, g = slot % 3, slot // 3 + 1
        length = [4,19,2 if g == 1 else 31][k]
        r = [w[i] for i in receipt_indices(slot)]
        require(r[0] == 0xa5000000 | k<<16 | length<<8 | g, 'receipt header')
        require(r[1] == DIGESTS[k][g-1], 'receipt payload digest')
        require(0 < r[2] <= LIMITS[k] and r[3] & 255 == IPSRS[k] and r[3] >> 8 <= r[2] and r[4] > 0, 'receipt IRQ/IPSR/time')
        if k == 0:
            require(r[3] >> 8 == 0 and r[5] <= r[4] and r[6] <= r[4], 'SPI compact bounds')
        elif k == 1:
            require(r[3] >> 8 > 0 and r[5] >= 8000 and 4 <= r[6] <= 0xffff, 'UART cleanup')
        else:
            require(r[5] >= 4000 and r[6] & 0xffff >= 2 and r[6] >> 16 < 10000, 'I2C cleanup')
        receipts.append(r)
    require(w[169:172] == [receipts[k][2]+receipts[k+3][2] for k in range(3)], 'IRQ totals')
    require(w[172:175] == IPSRS, 'direct IRQ IPSRs')
    require(12000 <= w[178] <= 14000 and 28000 <= w[179] <= 30000, 'owner pulse range')
    return receipts

def validate(text, nonce, warm_len, warm_digest):
    require(0 < nonce <= 0xffff and 0 < warm_len <= 0x10000 and 0 <= warm_digest <= 0xffffffff, 'expected identity')
    require('observer-quiesced no-more-rp1-access=1' not in text, 'old no-access claim')
    lines = []
    for line in text.splitlines():
        if '[WQLATE]' in line:
            require(line.count('[WQLATE] ') == 1, 'marker framing')
            lines.append(line.split('[WQLATE] ',1)[1])
    at = 0
    def take(pattern):
        nonlocal at
        require(at < len(lines), 'missing WQLATE row')
        m = re.fullmatch(pattern, lines[at]); at += 1
        require(m is not None, 'malformed/out-of-order WQLATE row')
        return m.groups()
    n, hz, ack = map(int, take(r'armed version=1 nonce=(\d+) counter_hz=(\d+) ack_counter=(\d+) delays_s=120,122 addr=0x2000f800 bytes=1024 samples=2 bracket_words=12 post-ack-rp1-read=1 post-ack-rp1-write=0 no-reinit=1'))
    # Selected commissioning explicitly excludes wrapping the local u64 clock.
    require(n == nonce and 0 < hz <= 0xffffffff and 0 <= ack < 2**64-hz*124, 'observer identity/clock/no-wrap')
    samples = []
    for sample in range(2):
        deadline, begin = map(int, take(rf'sample={sample} BEGIN deadline_counter=(\d+) begin_counter=(\d+)'))
        require(deadline == ack+hz*(120+2*sample) and deadline <= begin < deadline+hz, 'absolute schedule')
        before = [int(x,16) for x in take(rf'sample={sample} identity-before ((?:[0-9a-f]{{8}} ){{11}}[0-9a-f]{{8}})')[0].split()]
        words = []
        for row in range(0,256,4):
            words += [int(x,16) for x in take(rf'sample={sample} words {row:03} ((?:[0-9a-f]{{8}} ){{3}}[0-9a-f]{{8}})')[0].split()]
        after = [int(x,16) for x in take(rf'sample={sample} identity-after ((?:[0-9a-f]{{8}} ){{11}}[0-9a-f]{{8}})')[0].split()]
        read_begin, read_end = map(int, take(rf'sample={sample} END read_begin_counter=(\d+) read_end_counter=(\d+)'))
        require(begin <= read_begin <= read_end < begin+hz, 'read interval')
        require(before == after == [words[i] for i in IDENTITY], 'identity bracket change')
        receipts = check_words(words, nonce, warm_len, warm_digest)
        samples.append(dict(words=words, begin_counter=begin, read_begin_counter=read_begin,
            read_end_counter=read_end, receipts=receipts, msp_used_bytes=words[18],
            task_stack_free_words=[words[i] for i in HWM],
            task_stack_used_bytes=[4*(size-words[i]) for i,size in zip(HWM,STACK_WORDS)]))
    take(r'observer-complete samples=2 post-ack-rp1-read=1 post-ack-rp1-write=0 no-reinit=1')
    require(at == len(lines), 'duplicate/trailing WQLATE rows')
    a, b = samples
    require(a['read_end_counter'] < b['begin_counter'], 'overlapping reads')
    stable = IDENTITY + list(range(96,108)) + list(range(169,184)) + sum([receipt_indices(s) for s in range(6)],[])
    require(all(a['words'][i] == b['words'][i] for i in stable), 'warm identity/receipt/quiet IRQ changed')
    require(all(x >= y for x,y in zip(a['task_stack_free_words'],b['task_stack_free_words'])), 'minimum-free HWM increased')
    require(b['msp_used_bytes'] >= a['msp_used_bytes'], 'MSP used decreased')
    progress = {str(i):delta(a['words'][i],b['words'][i]) for i in PROGRESS}
    # Compare old publication to the later complete copy, not simultaneity of
    # volatile live fields within one copy. Publication fields are stable above.
    for latched, live in [(101,8),(102,9),(103,49),(104,50)]:
        require((b['words'][live]-a['words'][latched]) & 0xffffffff < 0x80000000, 'latched counter ahead of later live progress')
    return dict(result='WARM_TWO_SAMPLE_RECORD_VALID', classification='OBSERVATION_ONLY',
        hardware_acceptance=False, nonce=nonce, counter_hz=hz, ack_counter=ack,
        sample_gap_us=(b['read_begin_counter']-a['read_begin_counter'])*1e6/hz,
        progress_deltas=progress, samples=samples, atomic_snapshot_proven=False,
        boundary='Needs same-run image/type8/peer/causal-before-read/recovery joins. Returned fixed BAR2 samples only; not uninterrupted link or full R1/R2/R3 admission.')

def self_test():
    w = [0]*256
    w[:5] = [0x31305452,1,5,0,0]; w[96:105] = [0x384e524b,23,1,16,0x1234,6000,10000,100,100]
    w[124:126] = [0x31305a41,0x3f07]; w[19]=0x13579bdf; w[39]=1; w[18]=1024
    for i in PROGRESS: w[i]=10000
    for i,cap in zip(HWM,STACK_WORDS):w[i]=cap-32
    for start,psp in [(28,0x20008000),(65,0x20009000),(81,0x2000a000)]:w[start:start+4]=[0,2,psp,0x2000f000]
    w[175:178]=[0,2,0x2000b000];w[178:180]=[13000,29000]
    for slot in range(6):
        k,g=slot%3,slot//3+1;length=[4,19,2 if g==1 else 31][k]
        r=[0xa5000000|k<<16|length<<8|g,DIGESTS[k][g-1],1,IPSRS[k]|(256 if k==1 else 0),10000,100 if k==0 else 9000,100 if k==0 else 4]
        for i,v in zip(receipt_indices(slot),r):w[i]=v
    w[169:175]=[2,2,2,*IPSRS]
    z=w.copy()
    for i in PROGRESS:z[i]+=20
    def encode(pair):
        rows=['armed version=1 nonce=23 counter_hz=1000 ack_counter=100 delays_s=120,122 addr=0x2000f800 bytes=1024 samples=2 bracket_words=12 post-ack-rp1-read=1 post-ack-rp1-write=0 no-reinit=1']
        for n,words in enumerate(pair):
            t=120100+n*2000;ident=' '.join(f'{words[i]:08x}' for i in IDENTITY)
            rows += [f'sample={n} BEGIN deadline_counter={t} begin_counter={t}',f'sample={n} identity-before {ident}']
            rows += [f'sample={n} words {r:03} '+' '.join(f'{v:08x}' for v in words[r:r+4]) for r in range(0,256,4)]
            rows += [f'sample={n} identity-after {ident}',f'sample={n} END read_begin_counter={t+1} read_end_counter={t+2}']
        rows += ['observer-complete samples=2 post-ack-rp1-read=1 post-ack-rp1-write=0 no-reinit=1']
        return '\n'.join('[WQLATE] '+r for r in rows)
    text=encode([w,z]);require(validate(text,23,16,0x1234)['hardware_acceptance'] is False,'not HW alone')
    for values in [[10020]*4,[0xfffffff0]*4]:
        a,b=w.copy(),z.copy();a[101:105]=b[101:105]=values
        require(validate(encode([a,b]),23,16,0x1234)['hardware_acceptance'] is False,'ordered/wrapped publication counters')
    negatives=[text.replace('version=1','version=2'),text+'\n'+text,text.rsplit('\n',1)[0],
        text.replace('deadline_counter=122100','deadline_counter=122099'),text+'\nobserver-quiesced no-more-rp1-access=1',
        text.replace('sample=0 words 004','sample=0 words 000'),text.replace('read_end_counter=120102','read_end_counter=119999')]
    for i,value in [(0,0),(97,24),(98,2),(100,0),(125,1),(70,1),(192,0x31544652),(193,3),(198,0x100),(160,31),
        (175,35),(177,0x2000e000),(169,3),(172,41),(127,0),(183,1),(178,1),(101,0),(18,4096)]:
        bad=z.copy();bad[i]=value;negatives.append(encode([w,bad]))
    for i in PROGRESS:
        bad=z.copy();bad[i]=w[i];negatives.append(encode([w,bad]))
    bad=z.copy();bad[160]+=1;negatives.append(encode([w,bad]))
    for i in [105,106,107,184,185,191,193,198,209,255,101,102,103,104]:
        a,b=w.copy(),z.copy();a[i]=b[i]=0x7fffffff if i in [101,102,103,104] else 1
        negatives.append(encode([a,b]))
    negatives.append(text.replace('ack_counter=100',f'ack_counter={2**64-1}'))
    for bad in negatives:
        try:validate(bad,23,16,0x1234)
        except ValueError:continue
        raise AssertionError('negative accepted')
    require(delta(0xfffffff0,16)==32,'wrap')
    print(f'PASS synthetic two-sample parser, {len(negatives)} refusals and wrap; no hardware')

if __name__ == '__main__':
    if sys.argv[1:] == ['--self-test']:
        self_test()
    else:
        require(len(sys.argv)==5,'usage: UART nonce warm-data-bytes warm-data-fnv1a | --self-test')
        result=validate(Path(sys.argv[1]).read_text(),*(int(v,0) for v in sys.argv[2:]))
        print(json.dumps(result,indent=2))
