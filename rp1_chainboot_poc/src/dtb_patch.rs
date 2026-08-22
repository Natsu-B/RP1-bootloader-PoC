use alloc::vec::Vec;

use dtb::{
    DeviceTree, DeviceTreeEditExt, DeviceTreeOwned, DeviceTreeQueryExt, NameRef, NodeEditExt,
    ValueRef,
};
use rp1_abi::owner::{DEV_UART0, DEV_UART1};

use crate::BootError;
use crate::rp1_dtb_policy::{RP1_DEVICE_DTB_NODES, Rp1DeviceOwner, Rp1DtbPolicy};

const RP1_CLOCK_NODE_PATHS: &[&str] = &[
    "/axi/pcie@1000120000/rp1/clocks@18000",
    "/axi/pcie@120000/rp1/clocks@18000",
    "/soc/rp1/clocks@18000",
];

// Keep these local rather than depending on Linux headers in the bootloader.
// They are the ABI IDs from include/dt-bindings/clock/rp1.h.
const RP1_PLL_SYS_PRI_PH: u32 = 6;
const RP1_CLK_UART: u32 = 15;

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

    apply_uart_shared_clock_keepers(tree, policy)?;

    Ok(())
}

fn apply_uart_shared_clock_keepers(
    tree: &mut DeviceTreeOwned<'_>,
    policy: &Rp1DtbPolicy,
) -> Result<(), BootError> {
    let uart0 = policy.owner_of(DEV_UART0);
    let uart1 = policy.owner_of(DEV_UART1);
    let linux_owns_uart = uart0 == Rp1DeviceOwner::Linux || uart1 == Rp1DeviceOwner::Linux;
    let firmware_owns_uart = uart0 == Rp1DeviceOwner::Rp1 || uart1 == Rp1DeviceOwner::Rp1;

    if !(linux_owns_uart && firmware_owns_uart) {
        crate::logln!(
            "[DTB] RP1 UART shared-clock keeper not needed uart0={} uart1={}",
            uart0.as_str(),
            uart1.as_str()
        );
        return Ok(());
    }

    let Some(clock_node_id) = find_existing_node(tree, RP1_CLOCK_NODE_PATHS) else {
        crate::logln!("[DTB] RP1 clock provider node not found for UART coexistence");
        return Err(BootError::Rp1DtbNodeNotFound);
    };
    let Some(clock_phandle) = node_phandle(tree, clock_node_id) else {
        crate::logln!("[DTB] RP1 clock provider phandle missing for UART coexistence");
        return Err(BootError::DtbPatch);
    };

    add_clock_keeper(
        tree,
        "/regulator-rp1-uartclk-keeper",
        "rp1-uartclk-coexistence-keeper",
        clock_phandle,
        RP1_CLK_UART,
    )?;
    add_clock_keeper(
        tree,
        "/regulator-rp1-uart-apb-keeper",
        "rp1-uart-apb-coexistence-keeper",
        clock_phandle,
        RP1_PLL_SYS_PRI_PH,
    )?;

    crate::logln!(
        "[DTB] RP1 UART shared clocks pinned: clk_uart={} pll_sys_pri_ph={} provider_phandle=0x{:x}",
        RP1_CLK_UART,
        RP1_PLL_SYS_PRI_PH,
        clock_phandle
    );
    Ok(())
}

fn add_clock_keeper(
    tree: &mut DeviceTreeOwned<'_>,
    path: &str,
    regulator_name: &str,
    clock_phandle: u32,
    clock_id: u32,
) -> Result<(), BootError> {
    let node_id = tree
        .get_or_create_node_by_path(path)
        .map_err(|_| BootError::DtbPatch)?;
    let node = tree.node_mut(node_id).ok_or(BootError::DtbPatch)?;

    node.set_property(
        NameRef::Owned("compatible".into()),
        ValueRef::Owned(string_prop("regulator-fixed-clock")),
    );
    node.set_property(
        NameRef::Owned("regulator-name".into()),
        ValueRef::Owned(string_prop(regulator_name)),
    );
    // A fixed regulator requires a single fixed voltage even though this node is
    // used only as a clock-backed CCF vote. The numeric value has no hardware
    // voltage meaning here.
    node.set_property(
        NameRef::Owned("regulator-min-microvolt".into()),
        ValueRef::Owned(be32(1)),
    );
    node.set_property(
        NameRef::Owned("regulator-max-microvolt".into()),
        ValueRef::Owned(be32(1)),
    );
    node.set_property(
        NameRef::Owned("clocks".into()),
        ValueRef::Owned(be32_cells(&[clock_phandle, clock_id])),
    );
    node.set_property(
        NameRef::Owned("regulator-boot-on".into()),
        ValueRef::Owned(Vec::new()),
    );
    node.set_property(
        NameRef::Owned("regulator-always-on".into()),
        ValueRef::Owned(Vec::new()),
    );

    Ok(())
}

fn node_phandle(tree: &DeviceTreeOwned<'_>, node_id: usize) -> Option<u32> {
    let node = tree.nodes.get(node_id)?;
    for property_name in ["phandle", "linux,phandle"] {
        let Some(property) = node
            .properties
            .iter()
            .find(|property| property.name.as_str() == property_name)
        else {
            continue;
        };
        let bytes = property.value.as_slice();
        if bytes.len() == 4 {
            return Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
        }
    }
    None
}

fn find_existing_node(tree: &DeviceTreeOwned<'_>, paths: &[&str]) -> Option<usize> {
    for path in paths {
        if let Some(node) = tree.find_node_by_path(path) {
            return Some(node);
        }
    }
    None
}

fn string_prop(value: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(value.as_bytes());
    bytes.push(0);
    bytes
}

fn status_prop(value: &str) -> Vec<u8> {
    string_prop(value)
}

fn be32(value: u32) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}

fn be32_cells(values: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
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
