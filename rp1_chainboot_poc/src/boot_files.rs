use allocator::AlignedSliceBox;
use block_device_api::BlockDevice;
use file::{OpenOptions, StorageDevice};

use crate::BootError;
use crate::rp1_image::checksum32;

pub fn read_file(
    dev: &'static dyn BlockDevice,
    path: &str,
) -> Result<AlignedSliceBox<u8>, BootError> {
    let storage = StorageDevice::from_ready_block_device(dev).map_err(|_| BootError::SdFile)?;
    let handle = storage
        .open(0, path, &OpenOptions::Read)
        .map_err(|_| BootError::SdFile)?;
    handle.read(8).map_err(|_| BootError::SdFile)
}

pub fn read_required_file(
    dev: &'static dyn BlockDevice,
    path: &str,
) -> Result<AlignedSliceBox<u8>, BootError> {
    let bytes = read_file(dev, path)?;
    crate::logln!("[SD] {} ok: size={}", path, bytes.len());
    Ok(bytes)
}

pub fn read_optional_file(
    dev: &'static dyn BlockDevice,
    path: &str,
) -> Result<Option<AlignedSliceBox<u8>>, BootError> {
    match read_file(dev, path) {
        Ok(bytes) => {
            crate::logln!("[SD] {} found: size={}", path, bytes.len());
            Ok(Some(bytes))
        }
        Err(BootError::SdFile) => Ok(None),
        Err(err) => Err(err),
    }
}

pub fn probe_file(dev: &'static dyn BlockDevice, path: &str, label: &str) -> Result<(), BootError> {
    let bytes = read_file(dev, path)?;
    crate::logln!(
        "[SD] {} ok: size={} checksum=0x{:08x}",
        label,
        bytes.len(),
        checksum32(&bytes)
    );
    Ok(())
}
