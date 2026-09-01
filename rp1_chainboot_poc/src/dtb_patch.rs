#[cfg(feature = "rp1-linux-scmi-uart-clock")]
use alloc::string::String;
use alloc::vec::Vec;

#[cfg(feature = "rp1-linux-handoff-no-gem")]
use dtb::NodeQueryExt;
use dtb::{
    DeviceTree, DeviceTreeEditExt, DeviceTreeOwned, DeviceTreeQueryExt, NameRef, NodeEditExt,
    ValueRef,
};
#[cfg(feature = "rp1-linux-scmi-uart-clock")]
use rp1_abi::owner::{DEV_UART0, DEV_UART1};

use crate::BootError;
#[cfg(feature = "rp1-linux-scmi-uart-clock")]
use crate::rp1_dtb_policy::Rp1DeviceOwner;
use crate::rp1_dtb_policy::{RP1_DEVICE_DTB_NODES, Rp1DtbPolicy};

#[cfg(feature = "rp1-linux-handoff-no-gem")]
const RP1_ETHERNET_DTB_PATHS: &[&str] = &[
    "/axi/pcie@1000120000/rp1/ethernet@100000",
    "/axi/pcie@120000/rp1/ethernet@100000",
    "/soc/rp1/ethernet@100000",
];

#[cfg(feature = "rp1-linux-scmi-uart-clock")]
const RP1_MBOX_NODE_PATHS: &[&str] = &[
    "/axi/pcie@1000120000/rp1/mailbox@8000",
    "/axi/pcie@120000/rp1/mailbox@8000",
    "/soc/rp1/mailbox@8000",
];
#[cfg(feature = "rp1-linux-scmi-uart-clock")]
const RP1_SRAM_NODE_PATHS: &[&str] = &[
    "/axi/pcie@1000120000/rp1/sram@400000",
    "/axi/pcie@120000/rp1/sram@400000",
    "/soc/rp1/sram@400000",
];
#[cfg(feature = "rp1-linux-scmi-uart-clock")]
const RP1_GPIO_NODE_PATHS: &[&str] = &[
    "/axi/pcie@1000120000/rp1/gpio@d0000",
    "/axi/pcie@120000/rp1/gpio@d0000",
    "/soc/rp1/gpio@d0000",
];
#[cfg(feature = "rp1-linux-scmi-uart-clock")]
const SCMI_CLOCK_UART: u32 = 0;
#[cfg(feature = "rp1-linux-scmi-uart-clock")]
const SCMI_CLOCK_UART_APB: u32 = 1;
#[cfg(feature = "rp1-linux-scmi-uart-clock")]
const SCMI_PROTOCOL_CLOCK: u32 = 0x14;
#[cfg(feature = "rp1-linux-scmi-uart-clock")]
const SCMI_MBOX_CHANNEL: u32 = 1;
#[cfg(feature = "rp1-linux-scmi-uart-clock")]
const SCMI_SHMEM_OFFSET: u32 = 0xfb00;
#[cfg(feature = "rp1-linux-scmi-uart-clock")]
const SCMI_SHMEM_SIZE: u32 = 0x100;

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
    #[cfg(feature = "rp1-linux-handoff-no-gem")]
    let disabled_ethernet_nodes = disable_rp1_ethernet(&mut tree)?;
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
    #[cfg(feature = "rp1-linux-handoff-no-gem")]
    {
        // SAFETY: the bounded destination was populated immediately above.
        let handoff_dtb =
            unsafe { core::slice::from_raw_parts(output_base as *const u8, dtb.len()) };
        verify_serialized_rp1_ethernet_disabled(handoff_dtb, disabled_ethernet_nodes)?;
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

#[cfg(feature = "rp1-linux-handoff-no-gem")]
fn disable_rp1_ethernet(tree: &mut DeviceTreeOwned<'_>) -> Result<usize, BootError> {
    let mut disabled = 0;
    for path in RP1_ETHERNET_DTB_PATHS {
        let Some(node_id) = tree.find_node_by_path(path) else {
            continue;
        };
        tree.node_mut(node_id)
            .ok_or(BootError::DtbPatch)?
            .set_property(
                NameRef::Owned("status".into()),
                ValueRef::Owned(status_prop("disabled")),
            );
        disabled += 1;
        crate::logln!("[DTB] Phase5 disable {}", path);
    }
    if disabled == 0 {
        crate::logln!("[DTB] Phase5 RP1 ethernet@100000 node not found");
        return Err(BootError::Rp1DtbNodeNotFound);
    }
    Ok(disabled)
}

#[cfg(feature = "rp1-linux-handoff-no-gem")]
fn verify_serialized_rp1_ethernet_disabled(dtb: &[u8], expected: usize) -> Result<(), BootError> {
    let tree = DeviceTree::from_dtb(dtb).map_err(|_| BootError::DtbPatch)?;
    let mut verified = 0;
    for path in RP1_ETHERNET_DTB_PATHS {
        let Some(node_id) = tree.find_node_by_path(path) else {
            continue;
        };
        let status = tree
            .node(node_id)
            .and_then(|node| node.property("status"))
            .map(|property| property.value.as_slice());
        if status != Some(b"disabled\0".as_slice()) {
            crate::logln!("[DTB] Phase5 ethernet status verify failed {}", path);
            return Err(BootError::DtbPatch);
        }
        verified += 1;
    }
    if verified != expected {
        return Err(BootError::DtbPatch);
    }
    crate::logln!(
        "[DTB] Phase5 serialized ethernet status=disabled nodes={}",
        verified
    );
    Ok(())
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

    #[cfg(feature = "rp1-linux-scmi-uart-clock")]
    apply_scmi_uart_clock_coexistence(tree, policy)?;

    Ok(())
}

#[cfg(feature = "rp1-linux-scmi-uart-clock")]
fn apply_scmi_uart_clock_coexistence(
    tree: &mut DeviceTreeOwned<'_>,
    policy: &Rp1DtbPolicy,
) -> Result<(), BootError> {
    if policy.source.as_str() != "note"
        || policy.memory_profile != crate::rp1_note::Rp1MemoryProfile::SharedSramV2
        || policy.owner_of(DEV_UART0) != Rp1DeviceOwner::Rp1
        || policy.owner_of(DEV_UART1) != Rp1DeviceOwner::Linux
    {
        crate::logln!(
            "[DTB] SCMI UART clock policy rejected: source={} memory={} uart0={} uart1={}",
            policy.source.as_str(),
            policy.memory_profile.as_str(),
            policy.owner_of(DEV_UART0).as_str(),
            policy.owner_of(DEV_UART1).as_str()
        );
        return Err(BootError::Rp1DtbPolicyInvalid);
    }

    let mbox_node_id =
        find_existing_node(tree, RP1_MBOX_NODE_PATHS).ok_or(BootError::Rp1DtbNodeNotFound)?;
    let mbox_phandle = node_phandle_or_allocate(tree, mbox_node_id)?;
    set_node_status(tree, mbox_node_id, "okay")?;

    let sram_path =
        find_existing_path(tree, RP1_SRAM_NODE_PATHS).ok_or(BootError::Rp1DtbNodeNotFound)?;
    let mut shmem_path = String::from(sram_path);
    shmem_path.push_str("/scmi-sram-section@fb00");
    let shmem_node_id = tree
        .get_or_create_node_by_path(&shmem_path)
        .map_err(|_| BootError::DtbPatch)?;
    {
        let node = tree.node_mut(shmem_node_id).ok_or(BootError::DtbPatch)?;
        node.set_property(
            NameRef::Owned("compatible".into()),
            ValueRef::Owned(string_prop("arm,scmi-shmem")),
        );
        node.set_property(
            NameRef::Owned("reg".into()),
            ValueRef::Owned(be32_cells(&[SCMI_SHMEM_OFFSET, SCMI_SHMEM_SIZE])),
        );
    }
    let shmem_phandle = node_phandle_or_allocate(tree, shmem_node_id)?;

    let scmi_node_id = tree
        .get_or_create_node_by_path("/scmi")
        .map_err(|_| BootError::DtbPatch)?;
    {
        let node = tree.node_mut(scmi_node_id).ok_or(BootError::DtbPatch)?;
        node.set_property(
            NameRef::Owned("compatible".into()),
            ValueRef::Owned(string_prop("arm,scmi")),
        );
        node.set_property(
            NameRef::Owned("mboxes".into()),
            ValueRef::Owned(be32_cells(&[mbox_phandle, SCMI_MBOX_CHANNEL])),
        );
        node.set_property(
            NameRef::Owned("mbox-names".into()),
            ValueRef::Owned(string_prop("tx")),
        );
        node.set_property(
            NameRef::Owned("shmem".into()),
            ValueRef::Owned(be32(shmem_phandle)),
        );
        node.set_property(
            NameRef::Owned("#address-cells".into()),
            ValueRef::Owned(be32(1)),
        );
        node.set_property(
            NameRef::Owned("#size-cells".into()),
            ValueRef::Owned(be32(0)),
        );
        node.set_property(
            NameRef::Owned("status".into()),
            ValueRef::Owned(status_prop("okay")),
        );
    }

    let clock_protocol_id = tree
        .get_or_create_node_by_path("/scmi/protocol@14")
        .map_err(|_| BootError::DtbPatch)?;
    {
        let node = tree
            .node_mut(clock_protocol_id)
            .ok_or(BootError::DtbPatch)?;
        node.set_property(
            NameRef::Owned("reg".into()),
            ValueRef::Owned(be32(SCMI_PROTOCOL_CLOCK)),
        );
        node.set_property(
            NameRef::Owned("#clock-cells".into()),
            ValueRef::Owned(be32(1)),
        );
    }
    let scmi_clock_phandle = node_phandle_or_allocate(tree, clock_protocol_id)?;

    let uart1 = RP1_DEVICE_DTB_NODES
        .iter()
        .find(|spec| spec.bit == DEV_UART1)
        .and_then(|spec| find_existing_node(tree, spec.fallback_paths))
        .ok_or(BootError::Rp1DtbNodeNotFound)?;
    rewrite_uart1_clocks(tree, uart1, scmi_clock_phandle)?;

    if let Some(rp1_firmware) = tree.find_node_by_path("/rp1_firmware") {
        set_node_status(tree, rp1_firmware, "disabled")?;
    }
    reserve_gpio_range(tree, 14, 2)?;

    crate::logln!(
        "[DTB] SCMI UART clock coexistence: channel={} shmem=0x{:x}+0x{:x}",
        SCMI_MBOX_CHANNEL,
        SCMI_SHMEM_OFFSET,
        SCMI_SHMEM_SIZE
    );
    Ok(())
}

#[cfg(feature = "rp1-linux-scmi-uart-clock")]
fn rewrite_uart1_clocks(
    tree: &mut DeviceTreeOwned<'_>,
    uart_node_id: usize,
    scmi_clock_phandle: u32,
) -> Result<(), BootError> {
    let node = tree.node_mut(uart_node_id).ok_or(BootError::DtbPatch)?;
    node.set_property(
        NameRef::Owned("clocks".into()),
        ValueRef::Owned(be32_cells(&[
            scmi_clock_phandle,
            SCMI_CLOCK_UART,
            scmi_clock_phandle,
            SCMI_CLOCK_UART_APB,
        ])),
    );
    node.set_property(
        NameRef::Owned("clock-names".into()),
        ValueRef::Owned(string_list_prop(&["uartclk", "apb_pclk"])),
    );
    Ok(())
}

#[cfg(feature = "rp1-linux-scmi-uart-clock")]
fn reserve_gpio_range(
    tree: &mut DeviceTreeOwned<'_>,
    start: u32,
    count: u32,
) -> Result<(), BootError> {
    let gpio_node_id =
        find_existing_node(tree, RP1_GPIO_NODE_PATHS).ok_or(BootError::Rp1DtbNodeNotFound)?;
    let existing = property_bytes(tree, gpio_node_id, "gpio-reserved-ranges")
        .map(|value| value.to_vec())
        .unwrap_or_default();
    if existing.len() % 8 != 0 {
        return Err(BootError::DtbPatch);
    }
    for cells in existing.chunks_exact(8) {
        let range_start = read_be32(&cells[0..4]);
        let range_count = read_be32(&cells[4..8]);
        if start >= range_start
            && start.saturating_add(count) <= range_start.saturating_add(range_count)
        {
            return Ok(());
        }
    }
    let mut ranges = existing;
    ranges.extend_from_slice(&start.to_be_bytes());
    ranges.extend_from_slice(&count.to_be_bytes());
    let node = tree.node_mut(gpio_node_id).ok_or(BootError::DtbPatch)?;
    node.set_property(
        NameRef::Owned("gpio-reserved-ranges".into()),
        ValueRef::Owned(ranges),
    );
    Ok(())
}

#[cfg(feature = "rp1-linux-scmi-uart-clock")]
fn node_phandle_or_allocate(
    tree: &mut DeviceTreeOwned<'_>,
    node_id: usize,
) -> Result<u32, BootError> {
    if let Some(phandle) = node_phandle(tree, node_id) {
        return Ok(phandle);
    }
    let phandle = max_phandle(tree)
        .checked_add(1)
        .ok_or(BootError::DtbPatch)?;
    let node = tree.node_mut(node_id).ok_or(BootError::DtbPatch)?;
    node.set_property(
        NameRef::Owned("phandle".into()),
        ValueRef::Owned(be32(phandle)),
    );
    Ok(phandle)
}

#[cfg(feature = "rp1-linux-scmi-uart-clock")]
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
        let value = property.value.as_slice();
        if value.len() == 4 {
            return Some(read_be32(value));
        }
    }
    None
}

#[cfg(feature = "rp1-linux-scmi-uart-clock")]
fn max_phandle(tree: &DeviceTreeOwned<'_>) -> u32 {
    let mut max = 0;
    for node in &tree.nodes {
        for property in &node.properties {
            if property.name.as_str() != "phandle" && property.name.as_str() != "linux,phandle" {
                continue;
            }
            let value = property.value.as_slice();
            if value.len() == 4 {
                max = max.max(read_be32(value));
            }
        }
    }
    max
}

#[cfg(feature = "rp1-linux-scmi-uart-clock")]
fn property_bytes<'a>(
    tree: &'a DeviceTreeOwned<'_>,
    node_id: usize,
    name: &str,
) -> Option<&'a [u8]> {
    tree.nodes
        .get(node_id)?
        .properties
        .iter()
        .find(|property| property.name.as_str() == name)
        .map(|property| property.value.as_slice())
}

#[cfg(feature = "rp1-linux-scmi-uart-clock")]
fn set_node_status(
    tree: &mut DeviceTreeOwned<'_>,
    node_id: usize,
    status: &str,
) -> Result<(), BootError> {
    let node = tree.node_mut(node_id).ok_or(BootError::DtbPatch)?;
    node.set_property(
        NameRef::Owned("status".into()),
        ValueRef::Owned(status_prop(status)),
    );
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

#[cfg(feature = "rp1-linux-scmi-uart-clock")]
fn find_existing_path<'a>(tree: &DeviceTreeOwned<'_>, paths: &'a [&str]) -> Option<&'a str> {
    paths
        .iter()
        .copied()
        .find(|path| tree.find_node_by_path(path).is_some())
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

#[cfg(feature = "rp1-linux-scmi-uart-clock")]
fn string_list_prop(values: &[&str]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in values {
        bytes.extend_from_slice(value.as_bytes());
        bytes.push(0);
    }
    bytes
}

#[cfg(feature = "rp1-linux-scmi-uart-clock")]
fn be32(value: u32) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}

#[cfg(feature = "rp1-linux-scmi-uart-clock")]
fn be32_cells(values: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes
}

#[cfg(feature = "rp1-linux-scmi-uart-clock")]
fn read_be32(value: &[u8]) -> u32 {
    u32::from_be_bytes([value[0], value[1], value[2], value[3]])
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
