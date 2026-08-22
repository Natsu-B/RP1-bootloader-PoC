use alloc::string::String;
use alloc::vec::Vec;

use dtb::{
    DeviceTree, DeviceTreeEditExt, DeviceTreeOwned, DeviceTreeQueryExt, NameRef, NodeEditExt,
    ValueRef,
};
use rp1_abi::owner::{DEV_GPIO, DEV_UART0, DEV_UART1};

use crate::BootError;
use crate::rp1_dtb_policy::{RP1_DEVICE_DTB_NODES, Rp1DeviceOwner, Rp1DtbPolicy};

const RP1_CLOCK_NODE_PATHS: &[&str] = &[
    "/axi/pcie@1000120000/rp1/clocks@18000",
    "/axi/pcie@120000/rp1/clocks@18000",
    "/soc/rp1/clocks@18000",
];
const RP1_MBOX_NODE_PATHS: &[&str] = &[
    "/axi/pcie@1000120000/rp1/mailbox@8000",
    "/axi/pcie@120000/rp1/mailbox@8000",
    "/soc/rp1/mailbox@8000",
];
const RP1_SRAM_NODE_PATHS: &[&str] = &[
    "/axi/pcie@1000120000/rp1/sram@400000",
    "/axi/pcie@120000/rp1/sram@400000",
    "/soc/rp1/sram@400000",
];
const RP1_GPIO_NODE_PATHS: &[&str] = &[
    "/axi/pcie@1000120000/rp1/gpio@d0000",
    "/axi/pcie@120000/rp1/gpio@d0000",
    "/soc/rp1/gpio@d0000",
];

const RP1_PLL_SYS_PRI_PH: u32 = 6;
const RP1_CLK_UART: u32 = 15;
const SCMI_CLOCK_UART: u32 = 0;
const SCMI_CLOCK_UART_APB: u32 = 1;
const SCMI_PROTOCOL_CLOCK: u32 = 0x14;
const SCMI_MBOX_CHANNEL: u32 = 1;
const SCMI_SHMEM_OFFSET: u32 = 0xfb00;
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

    if mixed_uart_ownership(policy) {
        apply_uart_scmi_coexistence(tree, policy)?;
    }

    Ok(())
}

fn mixed_uart_ownership(policy: &Rp1DtbPolicy) -> bool {
    let uart0 = policy.owner_of(DEV_UART0);
    let uart1 = policy.owner_of(DEV_UART1);
    let linux_owns_uart = uart0 == Rp1DeviceOwner::Linux || uart1 == Rp1DeviceOwner::Linux;
    let firmware_owns_uart = uart0 == Rp1DeviceOwner::Rp1 || uart1 == Rp1DeviceOwner::Rp1;
    linux_owns_uart && firmware_owns_uart
}

fn apply_uart_scmi_coexistence(
    tree: &mut DeviceTreeOwned<'_>,
    policy: &Rp1DtbPolicy,
) -> Result<(), BootError> {
    let Some(clock_node_id) = find_existing_node(tree, RP1_CLOCK_NODE_PATHS) else {
        return Err(BootError::Rp1DtbNodeNotFound);
    };
    let clock_phandle = node_phandle_or_allocate(tree, clock_node_id)?;

    let Some(mbox_node_id) = find_existing_node(tree, RP1_MBOX_NODE_PATHS) else {
        return Err(BootError::Rp1DtbNodeNotFound);
    };
    let mbox_phandle = node_phandle_or_allocate(tree, mbox_node_id)?;
    set_node_status(tree, mbox_node_id, "okay")?;

    let Some(sram_path) = find_existing_path(tree, RP1_SRAM_NODE_PATHS) else {
        return Err(BootError::Rp1DtbNodeNotFound);
    };
    let mut shmem_path = String::from(sram_path);
    shmem_path.push_str("/scmi-shmem@fb00");
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

    let scmi_clock_node_id = tree
        .get_or_create_node_by_path("/scmi/protocol@14")
        .map_err(|_| BootError::DtbPatch)?;
    {
        let node = tree
            .node_mut(scmi_clock_node_id)
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
    let scmi_clock_phandle = node_phandle_or_allocate(tree, scmi_clock_node_id)?;

    for (dev_bit, pin_symbol) in [(DEV_UART0, "rp1_uart0_14_15"), (DEV_UART1, "rp1_uart1_0_1")] {
        if policy.owner_of(dev_bit) != Rp1DeviceOwner::Linux {
            continue;
        }
        let spec = RP1_DEVICE_DTB_NODES
            .iter()
            .find(|spec| spec.bit == dev_bit)
            .ok_or(BootError::Rp1DtbNodeNotFound)?;
        let uart_node_id =
            find_existing_node(tree, spec.fallback_paths).ok_or(BootError::Rp1DtbNodeNotFound)?;
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
        ensure_pinctrl_default(tree, uart_node_id, pin_symbol)?;
    }

    // Keep the physical RP1 clock provider alive in CCF. The UART consumers use
    // SCMI, while these always-on fixed regulators are the permanent Linux-side
    // references to the underlying physical gates.
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

    // Do not let clk-rp1 reprogram the shared APB phase rate from its DT
    // assigned-clocks table. Firmware establishes the agreed 100 MHz rate.
    drop_assigned_clock_rate(tree, clock_node_id, clock_phandle, RP1_PLL_SYS_PRI_PH)?;

    // Bridge the early-boot interval before the fixed-clock keepers have probed.
    // After probe, the keepers provide the explicit CCF references.
    ensure_bootarg(tree, "clk_ignore_unused")?;

    // Replacement firmware in this PoC does not service the stock channel-0
    // rp1-firmware ABI. Leave mailbox channel 1 to SCMI and disable channel 0's
    // Linux client instead of allowing a timeout during probe.
    if let Some(rp1_fw) = tree.find_node_by_path("/rp1_firmware") {
        set_node_status(tree, rp1_fw, "disabled")?;
    }

    if policy.owner_of(DEV_UART0) == Rp1DeviceOwner::Rp1 {
        reserve_gpio_range(tree, 14, 2)?;
    }

    crate::logln!(
        "[DTB] RP1 UART SCMI coexistence: mbox_ch={} shmem=0x{:x}+0x{:x} scmi_phandle=0x{:x}",
        SCMI_MBOX_CHANNEL,
        SCMI_SHMEM_OFFSET,
        SCMI_SHMEM_SIZE,
        scmi_clock_phandle
    );
    Ok(())
}

fn ensure_pinctrl_default(
    tree: &mut DeviceTreeOwned<'_>,
    node_id: usize,
    symbol: &str,
) -> Result<(), BootError> {
    if property_bytes(tree, node_id, "pinctrl-0")
        .map(|value| !value.is_empty())
        .unwrap_or(false)
    {
        return Ok(());
    }
    let phandle = symbol_phandle(tree, symbol).ok_or(BootError::DtbPatch)?;
    let node = tree.node_mut(node_id).ok_or(BootError::DtbPatch)?;
    node.set_property(
        NameRef::Owned("pinctrl-names".into()),
        ValueRef::Owned(string_prop("default")),
    );
    node.set_property(
        NameRef::Owned("pinctrl-0".into()),
        ValueRef::Owned(be32(phandle)),
    );
    Ok(())
}

fn symbol_phandle(tree: &DeviceTreeOwned<'_>, symbol: &str) -> Option<u32> {
    let symbols = tree.find_node_by_path("/__symbols__")?;
    let value = property_bytes(tree, symbols, symbol)?;
    let path_bytes = trim_ascii_nul_newline(value);
    let path = core::str::from_utf8(path_bytes).ok()?;
    let node_id = tree.find_node_by_path(path)?;
    node_phandle(tree, node_id)
}

fn reserve_gpio_range(
    tree: &mut DeviceTreeOwned<'_>,
    start: u32,
    count: u32,
) -> Result<(), BootError> {
    let gpio_node_id = find_existing_node(tree, RP1_GPIO_NODE_PATHS)
        .or_else(|| {
            RP1_DEVICE_DTB_NODES
                .iter()
                .find(|spec| spec.bit == DEV_GPIO)
                .and_then(|spec| find_existing_node(tree, spec.fallback_paths))
        })
        .ok_or(BootError::Rp1DtbNodeNotFound)?;

    let existing = property_bytes(tree, gpio_node_id, "gpio-reserved-ranges")
        .map(|value| value.to_vec())
        .unwrap_or_default();
    if existing.len() % 8 != 0 {
        return Err(BootError::DtbPatch);
    }

    for cells in existing.chunks_exact(8) {
        let range_start = read_be32(&cells[0..4]);
        let range_count = read_be32(&cells[4..8]);
        let range_end = range_start.saturating_add(range_count);
        let wanted_end = start.saturating_add(count);
        if start >= range_start && wanted_end <= range_end {
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

fn drop_assigned_clock_rate(
    tree: &mut DeviceTreeOwned<'_>,
    node_id: usize,
    provider_phandle: u32,
    clock_id: u32,
) -> Result<(), BootError> {
    let Some(clocks) = property_bytes(tree, node_id, "assigned-clocks").map(|v| v.to_vec()) else {
        return Ok(());
    };
    let Some(rates) = property_bytes(tree, node_id, "assigned-clock-rates").map(|v| v.to_vec())
    else {
        return Ok(());
    };
    if clocks.len() % 8 != 0 || rates.len() % 4 != 0 || clocks.len() / 8 != rates.len() / 4 {
        return Err(BootError::DtbPatch);
    }

    let mut new_clocks = Vec::new();
    let mut new_rates = Vec::new();
    for (index, cells) in clocks.chunks_exact(8).enumerate() {
        let phandle = read_be32(&cells[0..4]);
        let id = read_be32(&cells[4..8]);
        if phandle == provider_phandle && id == clock_id {
            continue;
        }
        new_clocks.extend_from_slice(cells);
        new_rates.extend_from_slice(&rates[index * 4..index * 4 + 4]);
    }

    let node = tree.node_mut(node_id).ok_or(BootError::DtbPatch)?;
    node.set_property(
        NameRef::Owned("assigned-clocks".into()),
        ValueRef::Owned(new_clocks),
    );
    node.set_property(
        NameRef::Owned("assigned-clock-rates".into()),
        ValueRef::Owned(new_rates),
    );
    Ok(())
}

fn ensure_bootarg(tree: &mut DeviceTreeOwned<'_>, arg: &str) -> Result<(), BootError> {
    let chosen = tree
        .get_or_create_node_by_path("/chosen")
        .map_err(|_| BootError::DtbPatch)?;
    let existing = property_bytes(tree, chosen, "bootargs")
        .map(trim_ascii_nul_newline)
        .unwrap_or(&[]);
    if existing
        .split(|byte| *byte == b' ')
        .any(|token| token == arg.as_bytes())
    {
        return Ok(());
    }

    let mut value = existing.to_vec();
    if !value.is_empty() {
        value.push(b' ');
    }
    value.extend_from_slice(arg.as_bytes());
    value.push(0);
    let node = tree.node_mut(chosen).ok_or(BootError::DtbPatch)?;
    node.set_property(NameRef::Owned("bootargs".into()), ValueRef::Owned(value));
    Ok(())
}

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
