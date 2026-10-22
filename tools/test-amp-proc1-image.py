#!/usr/bin/env python3
"""Compile the actual pure loader ELF inspector against both sealed ELFs."""
import argparse
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import re

p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--firmware',type=Path,required=True)
p.add_argument('--service-firmware',type=Path,required=True)
p.add_argument('--amp-observation',action='store_true')
p.add_argument('--amp-proc1-profile',action='store_true')
p.add_argument('--amp-clk-sys-200',action='store_true')
a=p.parse_args()
repo=Path(__file__).parents[1]
seal=(repo/'rp1_chainboot_poc/src/amp_service_admission.rs').read_text()
literal=re.search(r'pub const FIRMWARE_SHA256: \[u8;32\] = \[([^]]+)\]',seal).group(1)
expected=bytes(int(x,16) for x in re.findall(r'0x([0-9a-f]{2})',literal)).hex()
assert hashlib.sha256(a.service_firmware.read_bytes()).hexdigest()==expected,'changed retained phase0 ELF'
source=(repo/'rp1_chainboot_poc/src/rp1_image.rs').read_text()
# Compile its unmodified inspector/helpers; unrelated materialization uses an external crate.
pure=source[:source.index('/// Builds an RP1 bootstrap image')]+source[source.index('fn le32('):]
test=r'''
#[derive(Debug)] pub enum BootError {Rp1ImageInvalid,AddressOverflow}
mod image { SOURCE }
#[test] fn actual_images_and_mutations() {
    let i=std::fs::read(std::env::var("I_ELF").unwrap()).unwrap();
    let h=std::fs::read(std::env::var("H_ELF").unwrap()).unwrap();
    assert_eq!(image::inspect_rp1_elf(&h).unwrap().load_count,6);
    let result=image::inspect_rp1_elf(&i);
    if !cfg!(feature="rp1-amp-proc1") || (cfg!(observation_input) && !cfg!(feature="rp1-amp-observation"))
        || (cfg!(profile_input) && !cfg!(feature="rp1-amp-proc1-profile"))
        || (cfg!(clk_sys_200_input) && !cfg!(feature="rp1-amp-clk-sys-200")) {
        assert!(result.is_err());return;
    }
    let info=result.unwrap(); assert_eq!(info.load_count,if cfg!(any(profile_input,clk_sys_200_input)) {13} else if cfg!(observation_input) {12} else {9});
    assert_eq!(info.vector0_sp,0x2000f000);
    assert_eq!(info.vector1_reset,info.entry);
    let ph=u32::from_le_bytes(i[28..32].try_into().unwrap()) as usize;
    let stride=u16::from_le_bytes(i[42..44].try_into().unwrap()) as usize;
    let count=u16::from_le_bytes(i[44..46].try_into().unwrap()) as usize;
    let spare=(0..count).find(|n| u32::from_le_bytes(i[ph+n*stride..ph+n*stride+4].try_into().unwrap())!=1).unwrap();
    let mut extra=i.clone(); extra.copy_within(ph..ph+stride,ph+spare*stride);
    assert!(image::inspect_rp1_elf(&extra).is_err());
    let mut bad=i.clone(); bad[ph+16..ph+20].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(image::inspect_rp1_elf(&bad).is_err());
    let mut missing=i.clone(); missing[ph..ph+4].fill(0);
    assert!(image::inspect_rp1_elf(&missing).is_err());
    assert!(image::inspect_rp1_elf(&i[..52]).is_err());
    if cfg!(any(profile_input,clk_sys_200_input)) {
        let slot=(0..count).find(|n|u32::from_le_bytes(i[ph+n*stride+12..ph+n*stride+16].try_into().unwrap())==0x2000f000).unwrap();
        for (field,value) in [(12,0x2000f800u32),(20,0x800),(24,7),(8,0x2000e000)] {
            let mut bad=i.clone(); let off=ph+slot*stride+field;
            bad[off..off+4].copy_from_slice(&value.to_le_bytes());
            assert!(image::inspect_rp1_elf(&bad).is_err());
        }
        for (size,accepted) in [(0u32,false),(0x600,true),(0x601,false),(0x800,false)] {
            let mut boundary=i.clone();
            for field in [16,20] {
                let off=ph+slot*stride+field;
                boundary[off..off+4].copy_from_slice(&size.to_le_bytes());
            }
            assert_eq!(image::inspect_rp1_elf(&boundary).is_ok(),accepted,"extra segment size {size:x}");
        }
    }
}
'''.replace('SOURCE',pure)
with tempfile.TemporaryDirectory(prefix='amp-proc1-image-') as d:
    src=Path(d)/'inspector.rs';src.write_text(test)
    variants=['legacy','phase1'] + (['observation'] if a.amp_observation else []) + (['profile'] if a.amp_proc1_profile else []) + (['clk-sys-200'] if a.amp_clk_sys_200 else [])
    for feature in variants:
        exe=Path(d)/feature
        cfg=['--cfg','feature="rp1-amp-proc1"'] if feature!='legacy' else []
        if feature in ('observation','profile','clk-sys-200'):cfg += ['--cfg','feature="rp1-amp-observation"']
        if feature=='profile':cfg += ['--cfg','feature="rp1-amp-proc1-profile"']
        if feature=='clk-sys-200':cfg += ['--cfg','feature="rp1-amp-clk-sys-200"']
        if a.amp_observation:cfg += ['--cfg','observation_input']
        if a.amp_proc1_profile:cfg += ['--cfg','profile_input']
        if a.amp_clk_sys_200:cfg += ['--cfg','clk_sys_200_input']
        subprocess.run(['rustc','--edition=2024','--test','-A','dead_code',*cfg,str(src),'-o',str(exe)],check=True)
        subprocess.run([str(exe)],env={**os.environ,'I_ELF':str(a.firmware.resolve()),'H_ELF':str(a.service_firmware.resolve())},check=True)
print('HOST_PASS actual inspector: H preserved; explicit phase1/observation/profile/clk-sys-200 segment gates; extra/truncated/invalid/missing-vector segments refused')
