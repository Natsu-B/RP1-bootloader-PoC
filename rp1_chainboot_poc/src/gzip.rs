use crate::BootError;

pub struct KernelImageInfo {
    pub base: usize,
    pub len: usize,
    pub was_gzip: bool,
}

pub fn decompress_kernel_if_needed(
    input: &[u8],
    output_base: usize,
    output_max: usize,
) -> Result<KernelImageInfo, BootError> {
    if !is_gzip(input) {
        crate::logln!("[KERNEL] not gzip");
        if input.len() > output_max {
            return Err(BootError::LinuxImageInvalid);
        }
        // SAFETY: output_base is a fixed kernel placement range checked by placement.rs.
        unsafe {
            core::ptr::copy_nonoverlapping(input.as_ptr(), output_base as *mut u8, input.len());
        }
        return Ok(KernelImageInfo {
            base: output_base,
            len: input.len(),
            was_gzip: false,
        });
    }

    crate::logln!("[KERNEL] gzip detected");
    let (deflate, isize) = parse_gzip(input)?;
    // SAFETY: output_base is a fixed kernel placement range checked by placement.rs.
    let out = unsafe { core::slice::from_raw_parts_mut(output_base as *mut u8, output_max) };
    let len = match miniz_oxide::inflate::decompress_slice_iter_to_slice(
        out,
        core::iter::once(deflate),
        false,
        false,
    ) {
        Ok(len) => len,
        Err(status) => {
            let len = isize as usize;
            if len <= output_max && looks_like_arm64_image(&out[..len.min(64)]) {
                crate::logln!(
                    "[KERNEL] gzip inflate returned status {:?}; accepting Image by gzip ISIZE",
                    status
                );
                len
            } else {
                return Err(BootError::Gzip);
            }
        }
    };
    if !looks_like_arm64_image(&out[..len]) {
        return Err(BootError::Gzip);
    }
    if len as u32 != isize {
        crate::logln!(
            "[KERNEL] gzip ISIZE mismatch: inflated={} trailer={}; continuing with Image header",
            len,
            isize
        );
    }
    crate::logln!("[KERNEL] decompressed size={}", len);
    Ok(KernelImageInfo {
        base: output_base,
        len,
        was_gzip: true,
    })
}

fn is_gzip(input: &[u8]) -> bool {
    input.len() >= 2 && input[0] == 0x1f && input[1] == 0x8b
}

fn looks_like_arm64_image(input: &[u8]) -> bool {
    input.len() >= 64 && input[56..60] == [0x41, 0x52, 0x4d, 0x64]
}

fn parse_gzip(input: &[u8]) -> Result<(&[u8], u32), BootError> {
    if input.len() < 18 || input[0] != 0x1f || input[1] != 0x8b || input[2] != 8 {
        return Err(BootError::Gzip);
    }
    let flg = input[3];
    let mut off = 10usize;

    if (flg & 0x04) != 0 {
        let xlen = le16(input, off)? as usize;
        off = off
            .checked_add(2)
            .and_then(|v| v.checked_add(xlen))
            .ok_or(BootError::Gzip)?;
    }
    if (flg & 0x08) != 0 {
        off = skip_cstr(input, off)?;
    }
    if (flg & 0x10) != 0 {
        off = skip_cstr(input, off)?;
    }
    if (flg & 0x02) != 0 {
        off = off.checked_add(2).ok_or(BootError::Gzip)?;
    }
    if off >= input.len().saturating_sub(8) {
        return Err(BootError::Gzip);
    }
    let trailer = input.len() - 8;
    let isize = le32(input, input.len() - 4)?;
    Ok((&input[off..trailer], isize))
}

fn skip_cstr(input: &[u8], mut off: usize) -> Result<usize, BootError> {
    while off < input.len() {
        let b = input[off];
        off += 1;
        if b == 0 {
            return Ok(off);
        }
    }
    Err(BootError::Gzip)
}

fn le16(input: &[u8], off: usize) -> Result<u16, BootError> {
    let bytes = input.get(off..off + 2).ok_or(BootError::Gzip)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn le32(input: &[u8], off: usize) -> Result<u32, BootError> {
    let bytes = input.get(off..off + 4).ok_or(BootError::Gzip)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}
