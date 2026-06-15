use allocator::AlignedSliceBox;
use block_device_api::BlockDevice;
use file::{OpenOptions, StorageDevice, StorageDeviceErr};
use filesystem::FileSystemErr;

use crate::BootError;
use crate::rp1_image::checksum32;

const FILE_READ_ALIGN: usize = 8;

pub fn read_file(
    dev: &'static dyn BlockDevice,
    path: &str,
) -> Result<AlignedSliceBox<u8>, BootError> {
    let storage = StorageDevice::from_ready_block_device(dev).map_err(|_| BootError::SdMount)?;
    let handle = storage
        .open(0, path, &OpenOptions::Read)
        .map_err(map_open_error)?;
    let expected_size = handle.size().map_err(|_| BootError::SdRead)?;
    // FileHandle::read takes an alignment, not a byte count.
    let bytes = handle
        .read(FILE_READ_ALIGN)
        .map_err(|_| BootError::SdRead)?;
    if bytes.len() as u64 != expected_size {
        crate::logln!(
            "[SD] {} size mismatch: stat={} read={}",
            path,
            expected_size,
            bytes.len()
        );
        return Err(BootError::SdRead);
    }
    Ok(bytes)
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
        Err(BootError::SdFileNotFound) => Ok(None),
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

fn map_open_error(err: StorageDeviceErr) -> BootError {
    match err {
        StorageDeviceErr::FileSystemErr(FileSystemErr::NotFound) => BootError::SdFileNotFound,
        StorageDeviceErr::FileSystemErr(_) => BootError::SdOpen,
        StorageDeviceErr::IoErr(_) | StorageDeviceErr::StillUsed => BootError::SdOpen,
    }
}
