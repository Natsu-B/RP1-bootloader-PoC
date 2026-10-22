//! Cold admission only: the sealed proc1 probe must remain held until Linux HELLO.
use crate::amp_service_admission::ServiceAdmission;

pub const FIRMWARE_SHA256: [u8;32] = [
    0xa9,0x58,0xd0,0x09,0x6b,0x26,0x08,0x07,0x45,0xb6,0xde,0x0f,0xcb,0xba,0xe3,0x6b,
    0xce,0xdb,0xed,0xa6,0xbf,0x78,0x0f,0x50,0xca,0x9d,0xb2,0x9d,0x6c,0x80,0x81,0x42,
];
pub const SCMI_HANDLER: u32 = if cfg!(feature = "rp1-amp-proc1-no-hooks") { 0x2000_0d89 }
    else if cfg!(feature = "rp1-amp-proc1-local-stack") { 0x2000_0dc9 }
    else if cfg!(feature = "rp1-amp-proc1-profile") { 0x2000_0da5 } else { 0x2000_0d45 };
pub const NO_HOOKS_FIRMWARE_SHA256: [u8;32] = [
    0xd2,0xf4,0xd4,0x1b,0xe9,0x0a,0xeb,0xf9,0x9e,0xfa,0x0b,0x3a,0x06,0x05,0xb2,0x55,
    0xdf,0x8a,0xea,0xa8,0x10,0x86,0x74,0x6f,0xde,0x32,0xf0,0xc2,0xca,0xcb,0x25,0xec,
];
pub const LOCAL_STACK_FIRMWARE_SHA256: [u8;32] = [
    0x15,0x48,0xa6,0xff,0x74,0xee,0x34,0x14,0xba,0x92,0xaa,0xc8,0xfc,0xc5,0xf5,0x6a,
    0xbc,0x08,0x69,0x61,0x8f,0x17,0x3b,0xb6,0x7a,0x40,0xc2,0x9d,0xc0,0x4b,0xdd,0xb1,
];
pub const PROFILE_FIRMWARE_SHA256: [u8;32] = [
    0x3e,0x5f,0x75,0x37,0x9f,0xdc,0x8c,0xd8,0x90,0x34,0x84,0x7e,0x95,0x12,0x2d,0x29,
    0x2b,0x7c,0x70,0x31,0xc5,0x70,0x79,0x5f,0x43,0x23,0x9a,0xc0,0xac,0xa1,0x6e,0x5b,
];
pub const OBSERVATION_FIRMWARE_SHA256: [u8;32] = [
    0x2a,0x6b,0x89,0x7b,0xe9,0x09,0x4e,0x60,0x87,0xe7,0x97,0xef,0x59,0x39,0xec,0x7a,
    0x9a,0x57,0xa2,0x81,0x49,0x12,0x39,0x67,0x06,0x2e,0x77,0x07,0x29,0x0c,0xce,0xc6,
];
pub const fn firmware_sha256() -> [u8;32] {
    if cfg!(feature = "rp1-amp-proc1-no-hooks") { NO_HOOKS_FIRMWARE_SHA256 }
    else if cfg!(feature = "rp1-amp-proc1-local-stack") { LOCAL_STACK_FIRMWARE_SHA256 }
    else if cfg!(feature = "rp1-amp-proc1-profile") { PROFILE_FIRMWARE_SHA256 }
    else if cfg!(feature = "rp1-amp-observation") { OBSERVATION_FIRMWARE_SHA256 }
    else { FIRMWARE_SHA256 }
}

pub struct AmpProc1Admission { service: ServiceAdmission<1>, held: bool }
impl AmpProc1Admission {
    pub const fn new() -> Self { Self { service: ServiceAdmission::new(), held: true } }
    pub fn sample(&mut self, number: u32, words: &[u32;256], ctrl: u32, done: u32,
                  status: &[u32;32]) -> bool {
        self.held &= ctrl & (1<<31) != 0 && done & (1<<31) == 0
            && status.iter().all(|x| *x == 0);
        self.service.sample(number, words) && self.held
    }
    pub fn complete(&self) -> bool { self.held && self.service.complete() }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(n: u32) -> [u32;256] {
        let mut w=[0;256]; w[0]=u32::from_le_bytes(*b"AG01"); w[1]=1; w[2]=5;
        w[5]=100_000_000; w[19]=0x1357_9bdf; w[22]=99_999; w[29]=2;
        w[30]=0x2000_a6e0; w[31]=0x2000_f000; w[18]=512;
        w[32..34].fill(128); w[42..45].copy_from_slice(&[2,1024,1]);
        w[8]=(n+1)*1000; w[9]=(n+1)*2000; w[14]=n+1; w[11]=w[8]*1000; w
    }
    fn admitted(mut change: impl FnMut(u32,&mut [u32;256],&mut u32,&mut u32,&mut [u32;32])) -> bool {
        let mut gate=AmpProc1Admission::new();
        for n in 0..31 {
            let (mut w,mut ctrl,mut done,mut status)=(record(n),1<<31,0,[0;32]);
            change(n,&mut w,&mut ctrl,&mut done,&mut status);
            gate.sample(n,&w,ctrl,done,&status);
        }
        gate.complete()
    }
    #[test]
    fn phase_and_cold_reset_rejections_are_permanent() {
        assert!(admitted(|_,_,_,_,_| {}));
        assert!(!admitted(|n,w,_,_,_| if n==12 { w[44]=0 }));
        for which in 0..34 {
            assert!(!admitted(|n,_,ctrl,done,status| if n==12 {
                match which {0=>*ctrl=0,1=>*done=1<<31,_=>status[which-2]=1}
            }),"cold mutation {which}");
        }
        assert!(!AmpProc1Admission::new().sample(1,&record(1),1<<31,0,&[0;32]));
        let mut unknown=ServiceAdmission::<2>::new();
        assert!(!unknown.sample(0,&record(0)));
    }
    #[test]
    fn actual_firmware_seal_if_supplied() {
        if let Ok(digest)=std::env::var("AMP_PROC1_FIRMWARE_SHA256") {
            let bytes:Vec<_>=(0..32).map(|i|u8::from_str_radix(&digest[i*2..i*2+2],16).unwrap()).collect();
            assert_eq!(bytes.as_slice(),&firmware_sha256());
            assert_eq!(u32::from_str_radix(&std::env::var("AMP_PROC1_SCMI_HANDLER").unwrap(),16).unwrap(),SCMI_HANDLER);
        }
    }
    #[test]
    fn actual_records_if_supplied() {
        if let Ok(path)=std::env::var("AMP_PROC1_UART_LOG") {
            let text=std::fs::read_to_string(path).unwrap();
            let mut records=vec![[0;256];31]; let mut seen=vec![[false;64];31];
            let mut cold=vec![(0,0,[0;32]);31]; let mut cold_seen=vec![[false;8];31];
            for line in text.lines() {
                if let Some((_,row))=line.split_once("[RTOS] ") {
                    let p:Vec<_>=row.split_whitespace().collect(); if p.len()!=6 {continue}
                    let n:usize=p[0].parse().unwrap(); let off:usize=p[1].parse().unwrap();
                    assert!(n<31 && off<256 && off%4==0 && !seen[n][off/4]); seen[n][off/4]=true;
                    for i in 0..4 {records[n][off+i]=u32::from_str_radix(p[2+i],16).unwrap();}
                }
                if let Some((_,row))=line.split_once("[AMP_PROC1_COLD] ") {
                    let p:Vec<_>=row.split_whitespace().collect(); assert_eq!(p.len(),8);
                    let n:usize=p[0].parse().unwrap(); let off:usize=p[1].parse().unwrap();
                    assert!(n<31 && off<32 && off%4==0 && !cold_seen[n][off/4]); cold_seen[n][off/4]=true;
                    let ctrl=u32::from_str_radix(p[2],16).unwrap(); let done=u32::from_str_radix(p[3],16).unwrap();
                    if off>0 { assert_eq!((ctrl,done),(cold[n].0,cold[n].1)); }
                    cold[n].0=ctrl; cold[n].1=done;
                    for i in 0..4 {cold[n].2[off+i]=u32::from_str_radix(p[4+i],16).unwrap();}
                }
            }
            assert!(seen.iter().flatten().all(|x|*x)); assert!(cold_seen.iter().flatten().all(|x|*x));
            let mut gate=AmpProc1Admission::new();
            for n in 0..31 {assert!(gate.sample(n as u32,&records[n],cold[n].0,cold[n].1,&cold[n].2));}
            assert!(gate.complete());
            assert_eq!(text.matches("[AMP_PROC1] admission=PASS tasks=2 phase=1 proc1=HELD").count(),1);
            assert_eq!(text.matches("[RTOS] observer-complete read-only=1").count(),1);
        }
    }
}
