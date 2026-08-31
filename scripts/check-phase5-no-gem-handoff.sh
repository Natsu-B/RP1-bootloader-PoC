#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
net="$root/rp1_chainboot_poc/src/net_boot.rs"
main="$root/rp1_chainboot_poc/src/main.rs"
dtb="$root/rp1_chainboot_poc/src/dtb_patch.rs"
cargo="$root/rp1_chainboot_poc/Cargo.toml"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

grep -Fq 'rp1-linux-handoff-no-gem = ["tftp-boot"]' "$cargo"
for feature in skip-rp1-reload continue-on-rp1-bootstrap-failure rp1-gdb-debug-stub rp1-bar2-rpc-proof; do
    grep -Fq "feature = \"$feature\"" "$main"
done

sed -n '/fn boot_from_tftp_no_gem_handoff/,/fn mask_daif_and_readback/p' "$net" >"$tmp/path"
test "$(grep -Fc 'gem.release_after_quiesce()' "$tmp/path")" -eq 1
quiesce=$(grep -nF 'gem.quiesce();' "$tmp/path" | cut -d: -f1)
readback=$(grep -nF 'let ncr = gem.diagnostic_snapshot().ncr;' "$tmp/path" | cut -d: -f1)
release=$(grep -nF 'gem.release_after_quiesce()' "$tmp/path" | cut -d: -f1)
test "$quiesce" -lt "$readback"
test "$readback" -lt "$release"

sed -n '/post-reset no-GEM path begin/,/jump_to_linux_el2(kernel_entry/p' "$net" >"$tmp/post-reset"
if grep -Eq 'Rp1Gem|init_tftp_gem|dhcp_|tftp::|NetworkBootLease|release_after_|gem\.' "$tmp/post-reset"; then
    echo 'post-reset GEM/network reference found' >&2
    exit 1
fi
for required in start_rp1_image 'delay_millis(10)' audit_rp1_pcie_after_reload \
    'Rp1InitMode::Auto' run_pre_linux_probe jump_to_linux_el2; do
    grep -Fq "$required" "$tmp/post-reset"
done

test "$(grep -Fc '/ethernet@100000' "$dtb")" -eq 3
grep -Fq 'verify_serialized_rp1_ethernet_disabled' "$dtb"
grep -Fq 'status != Some(b"disabled\0".as_slice())' "$dtb"
sed -n '/pub fn patch_dtb_for_linux/,/fn disable_rp1_ethernet/p' "$dtb" >"$tmp/dtb-patch"
copy=$(grep -nF 'core::ptr::copy_nonoverlapping' "$tmp/dtb-patch" | cut -d: -f1)
verify=$(grep -nF 'verify_serialized_rp1_ethernet_disabled(handoff_dtb' "$tmp/dtb-patch" | cut -d: -f1)
test "$copy" -lt "$verify"

echo 'phase5 no-GEM static guards: PASS'
