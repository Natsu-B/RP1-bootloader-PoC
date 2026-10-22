//! AG01 phase0 services only. No R1 stress, proc1, IRQ26 or realtime claim.

pub const FIRMWARE_SHA256: [u8;32] = [
    0x6f,0xa0,0x65,0x04,0x01,0xec,0x19,0x0e,0x67,0x56,0xf0,0xf5,0x3d,0x7b,0x09,0x93,
    0x67,0xfe,0x7f,0x1a,0x70,0x03,0x45,0x3e,0xfd,0x84,0x01,0x2f,0xd9,0x83,0x88,0x17,
];
pub const SCMI_HANDLER: u32 = 0x2000_0221;

pub type AmpServiceAdmission = ServiceAdmission<0>;

pub struct ServiceAdmission<const PHASE: u32> {
    next: u32,
    valid: bool,
    first: [u32; 3],
    previous: [u32; 4],
}

impl<const PHASE: u32> ServiceAdmission<PHASE> {
    pub const fn new() -> Self {
        Self { next: 0, valid: true, first: [0; 3], previous: [0; 4] }
    }

    pub fn sample(&mut self, number: u32, w: &[u32; 256]) -> bool {
        self.valid &= number == self.next && number < 31
            && w[0] == u32::from_le_bytes(*b"AG01") && w[1] == 1
            && w[2] != u32::MAX && w[3..5] == [0,0] && w[192] != 0x3154_4652
            && PHASE <= 1 && w[42..45] == [2,1024,PHASE];
        if number >= 3 {
            self.valid &= w[2] == 5 && w[19..21] == [0x1357_9bdf,0]
                && w[28] == 0 && w[29] & 3 == 2 && w[30] & 7 == 0
                && (0x2000_0000..0x2000_e000).contains(&w[30])
                && (0x2000_e000..=0x2000_f000).contains(&w[31])
                && w[21] & 0x700 == 0 && w[5] >= 1000
                && u64::from(w[22])+1 == u64::from(w[5]/1000)
                && (1..=3968).contains(&w[18]) && w[32..34].iter().all(|x| (32..=512).contains(x))
                && w[34..39].iter().all(|x| *x == 0)
                && w[64] == 0 && w[80] == 0 && w[45] == 0;
            if number == 3 {
                self.first = [w[8],w[11],w[14]];
            } else {
                self.valid &= [8,9,14].iter().enumerate().all(|(i,j)|
                    (1..0x8000_0000).contains(&w[*j].wrapping_sub(self.previous[i])));
            }
            self.previous = [w[8],w[9],w[14],w[11]];
        }
        self.next += 1;
        self.valid
    }

    pub fn complete(&self) -> bool {
        let ticks = u64::from(self.previous[0].wrapping_sub(self.first[0]));
        let elapsed = u64::from(self.previous[3].wrapping_sub(self.first[1]));
        self.valid && self.next == 31 && ticks > 0
            && 950*ticks <= elapsed && elapsed <= 1050*ticks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(n: u32) -> [u32;256] {
        let mut w = [0;256];
        w[0] = u32::from_le_bytes(*b"AG01"); w[1]=1; w[2]=5; w[5]=100_000_000;
        w[19]=0x1357_9bdf; w[22]=99_999; w[29]=2;
        w[30]=0x2000_a6e0; w[31]=0x2000_f000; w[18]=512;
        w[32..34].fill(128); w[42..45].copy_from_slice(&[2,1024,0]);
        w[8]=(n+1)*1000; w[9]=(n+1)*2000; w[14]=n+1; w[11]=w[8]*1000;
        w
    }
    fn admitted(records: &[[u32;256]]) -> bool {
        let mut gate = AmpServiceAdmission::new();
        for (n,w) in records.iter().enumerate() { gate.sample(n as u32,w); }
        gate.complete()
    }
    #[test]
    fn progress_context_stack_and_profile_rejections() {
        let good:Vec<_> = (0..31).map(record).collect(); assert!(admitted(&good));
        assert!(!admitted(&good[..30]));
        for (word,value) in [(0,0x31305452),(1,0),(2,4),(3,1),(4,1),(192,0x31544652),
            (19,0),(20,1),(21,0x700),(22,0),(5,0),(28,1),(29,0),(30,1),(30,0),
            (31,0),(18,0),(18,3972),(32,31),(33,513),(34,1),(38,1),
            (42,8),(43,2304),(44,1),(45,128),(64,1),(80,1)] {
            let mut bad=good.clone(); bad[12][word]=value;
            assert!(!admitted(&bad),"word={word} value={value}");
        }
        for word in [8,9,14] {
            let mut bad=good.clone(); bad[12][word]=bad[11][word]; assert!(!admitted(&bad));
        }
        for mean in [949,1051] {
            let mut bad=good.clone(); bad[30][11]=bad[3][11]+27_000*mean; assert!(!admitted(&bad));
        }
        assert!(!AmpServiceAdmission::new().sample(1,&record(1)));
    }
    #[test]
    fn actual_firmware_seal_if_supplied() {
        if let Ok(digest)=std::env::var("AMP_SERVICE_FIRMWARE_SHA256") {
            let bytes:Vec<_>=(0..32).map(|i|u8::from_str_radix(&digest[i*2..i*2+2],16).unwrap()).collect();
            assert_eq!(bytes.as_slice(),&FIRMWARE_SHA256);
            let vector=u32::from_str_radix(&std::env::var("AMP_SERVICE_SCMI_HANDLER").unwrap(),16).unwrap();
            assert_eq!(vector,SCMI_HANDLER);
        }
    }
    #[test]
    fn actual_records_if_supplied() {
        if let Ok(path)=std::env::var("AMP_SERVICE_UART_LOG") {
            let text=std::fs::read_to_string(path).unwrap();
            let mut records=vec![[0;256];31]; let mut seen=vec![[false;64];31];
            for line in text.lines() {
                let Some((_,row))=line.split_once("[RTOS] ") else {continue};
                let parts:Vec<_>=row.split_whitespace().collect(); if parts.len()!=6 {continue}
                let n:usize=parts[0].parse().unwrap(); let off:usize=parts[1].parse().unwrap();
                assert!(n<31 && off<256 && off%4==0 && !seen[n][off/4]); seen[n][off/4]=true;
                for i in 0..4 {records[n][off+i]=u32::from_str_radix(parts[2+i],16).unwrap();}
            }
            assert!(seen.iter().flatten().all(|x| *x)); assert!(admitted(&records));
            assert_eq!(text.matches("[RTOS] observer-complete read-only=1").count(),1);
            assert_eq!(text.matches("[AMP_SERVICE] admission=PASS tasks=2 phase=0").count(),1);
        }
    }
}
