use alloc::vec::Vec;

use dtb::{
    DeviceTree, DeviceTreeEditExt, DeviceTreeOwned, DeviceTreeQueryExt, NameRef, NodeEditExt,
    ValueRef,
};

use crate::BootError;
use crate::rp1_dtb_policy::{RP1_DEVICE_DTB_NODES, Rp1DtbPolicy};

/// A separately supplied Linux policy must retain same-boot board fixups.
/// This read-only admission is deliberately independent of RP1 ownership.
pub fn validate_firmware_board(
    firmware: &dtb::DtbParser,
    candidate: &dtb::DtbParser,
) -> Result<(), BootError> {
    use dtb::NodeQueryExt;
    let original = DeviceTree::from_parser(firmware).map_err(|_| BootError::DtbPatch)?;
    let linux = DeviceTree::from_parser(candidate).map_err(|_| BootError::DtbPatch)?;
    for (path, props) in [
        ("/memory@0", &["reg"][..]),
        ("/soc@107c000000/pinctrl@7d504100", &["compatible", "reg"][..]),
        ("/soc@107c000000/pinctrl@7d510700", &["compatible", "reg"][..]),
        ("/soc@107c000000/serial@7d001000", &["compatible", "reg", "interrupts"][..]),
    ] {
        let before = original.find_node_by_path(path).and_then(|id| original.node(id))
            .ok_or(BootError::DtbPatch)?;
        let after = linux.find_node_by_path(path).and_then(|id| linux.node(id))
            .ok_or(BootError::DtbPatch)?;
        for property in props {
            let value = before.property(property).ok_or(BootError::DtbPatch)?;
            if after.property(property).map(|p| p.value.as_slice()) != Some(value.value.as_slice()) {
                crate::logln!("[SCMILINUX] failure=board-dtb path={} property={}", path, property);
                return Err(BootError::DtbPatch);
            }
        }
    }
    crate::logln!("[SCMILINUX] board-dtb memory-pinctrl-uart10=match");
    Ok(())
}

pub struct PatchedDtb {
    pub addr: usize,
    pub len: usize,
}

pub fn patch_dtb_for_linux(
    parser: &dtb::DtbParser,
    output_base: usize,
    output_max: usize,
    initrd_start: usize,
    initrd_end: usize,
    bootargs: Option<&[u8]>,
    rp1_policy: Option<&Rp1DtbPolicy>,
) -> Result<PatchedDtb, BootError> {
    let borrowed = DeviceTree::from_parser(parser).map_err(|_| BootError::DtbPatch)?;
    let mut tree: DeviceTreeOwned = borrowed.into_owned();
    let chosen = tree
        .get_or_create_node_by_path("/chosen")
        .map_err(|_| BootError::DtbPatch)?;
    let node = tree.node_mut(chosen).ok_or(BootError::DtbPatch)?;
    node.set_property(
        NameRef::Owned("linux,initrd-start".into()),
        ValueRef::Owned(be64(initrd_start as u64)),
    );
    node.set_property(
        NameRef::Owned("linux,initrd-end".into()),
        ValueRef::Owned(be64(initrd_end as u64)),
    );
    if let Some(cmdline) = bootargs {
        let mut value = Vec::new();
        value.extend_from_slice(trim_ascii_nul_newline(cmdline));
        value.push(0);
        crate::logln!("[DTB] /chosen bootargs set: len={}", value.len() - 1);
        node.set_property(NameRef::Owned("bootargs".into()), ValueRef::Owned(value));
    } else {
        crate::logln!("[DTB] /chosen bootargs absent");
    }
    if let Some(policy) = rp1_policy {
        apply_rp1_policy(&mut tree, policy)?;
    } else {
        crate::logln!("[DTB] RP1 policy absent");
    }
    let dtb = tree.into_dtb_box().map_err(|_| BootError::DtbPatch)?;
    let aligned = output_base & 7 == 0;
    crate::logln!(
        "[DTB] patched output addr=0x{:x} size={} aligned8={} max={}",
        output_base,
        dtb.len(),
        aligned,
        output_max
    );
    if dtb.len() > output_max || !aligned {
        return Err(BootError::DtbPatch);
    }
    // SAFETY: output_base is the selected DTB copy range and the generated box is initialized.
    unsafe {
        core::ptr::copy_nonoverlapping(dtb.as_ptr(), output_base as *mut u8, dtb.len());
    }
    crate::logln!(
        "[DTB] /chosen linux,initrd-start=0x{:x}, linux,initrd-end=0x{:x}",
        initrd_start,
        initrd_end
    );
    Ok(PatchedDtb {
        addr: output_base,
        len: dtb.len(),
    })
}

fn apply_rp1_policy(
    tree: &mut DeviceTreeOwned<'_>,
    policy: &Rp1DtbPolicy,
) -> Result<(), BootError> {
    crate::logln!(
        "[DTB] RP1 policy source={} owner_rp1=0x{:x} owner_linux=0x{:x} owner_disabled=0x{:x}",
        policy.source.as_str(),
        policy.owner_rp1,
        policy.owner_linux,
        policy.owner_disabled
    );
    policy.validate()?;

    for spec in RP1_DEVICE_DTB_NODES {
        let owner = policy.owner_of(spec.bit);
        let Some(status) = owner.linux_status() else {
            crate::logln!(
                "[DTB] rp1 device {} owner={} unspecified",
                spec.name,
                owner.as_str()
            );
            return Err(BootError::Rp1DtbPolicyInvalid);
        };

        if spec.fallback_paths.is_empty() {
            crate::logln!(
                "[DTB] rp1 device {} owner={} no linux dtb node",
                spec.name,
                owner.as_str()
            );
            continue;
        }

        let Some(node_id) = find_existing_node(tree, spec.fallback_paths) else {
            crate::logln!(
                "[DTB] rp1 device {} owner={} node not found",
                spec.name,
                owner.as_str()
            );
            return Err(BootError::Rp1DtbNodeNotFound);
        };
        let node = tree.node_mut(node_id).ok_or(BootError::DtbPatch)?;
        node.set_property(
            NameRef::Owned("status".into()),
            ValueRef::Owned(status_prop(status)),
        );
        crate::logln!(
            "[DTB] rp1 device {} owner={} status={}",
            spec.name,
            owner.as_str(),
            status
        );
    }

    Ok(())
}

fn find_existing_node(tree: &DeviceTreeOwned<'_>, paths: &[&str]) -> Option<usize> {
    for path in paths {
        if let Some(node) = tree.find_node_by_path(path) {
            return Some(node);
        }
    }
    None
}

fn status_prop(value: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(value.as_bytes());
    bytes.push(0);
    bytes
}

fn be64(value: u64) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}

fn trim_ascii_nul_newline(mut bytes: &[u8]) -> &[u8] {
    while let Some((&last, rest)) = bytes.split_last() {
        if last == 0 || last == b'\n' || last == b'\r' {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}
