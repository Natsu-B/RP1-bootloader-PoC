#!/usr/bin/env python3
"""No Cargo/network needed: actual build guard, R1 predicates, preload ordering."""
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib

repo = Path(__file__).resolve().parents[1]
feature = 'rp1-scmi-linux-preloaded'
manifest = tomllib.loads((repo / 'rp1_chainboot_poc/Cargo.toml').read_text())
features = manifest['features']
allowed = {feature, 'rp1-rtos-record', 'rp1-gdb-debug-stub', 'tftp-boot',
           'tftp-initramfs', 'require-rp1-img', 'log-uart'}
assert features[feature] == ['rp1-rtos-record', 'tftp-initramfs']
env = {k: v for k, v in os.environ.items() if not k.startswith('CARGO_FEATURE_')}
env.update({'CARGO_FEATURE_' + f.upper().replace('-', '_'): '1' for f in allowed})
with tempfile.TemporaryDirectory(prefix='scmi-linux-test-') as scratch:
    guard = Path(scratch) / 'build-guard'
    subprocess.run(['rustc', '--edition=2024', repo / 'rp1_chainboot_poc/build.rs', '-o', guard], check=True)
    subprocess.run([guard], env=env, check=True, capture_output=True)
    for forbidden in features.keys() - allowed:
        trial = dict(env, **{'CARGO_FEATURE_' + forbidden.upper().replace('-', '_'): '1'})
        result = subprocess.run([guard], env=trial, capture_output=True, text=True)
        assert result.returncode != 0 and 'incompatible feature:' in result.stderr, forbidden
    tests = Path(scratch) / 'r1-tests'
    subprocess.run(['rustc', '--edition=2024', '--test', repo / 'rp1_chainboot_poc/src/scmi_linux_admission.rs', '-o', tests], check=True)
    subprocess.run([tests], check=True)
net = (repo / 'rp1_chainboot_poc/src/net_boot.rs').read_text()
preload = net[net.index('if cfg!(any(feature = "rp1-linux-observe-failure"'):net.index('    if skip_rp1_reload {')]
assert preload.index('download_kernel_image_from_tftp') < preload.index('download_initramfs') < preload.index('download_rp1_policy_and_reload_if_needed') < preload.index('return handoff_preloaded_kernel')
assert 'initramfs_len == 0' in preload and preload.count('init_tftp_gem(') == 1
assert preload.index('"scmi_linux.dtb"') < preload.index('download_rp1_policy_and_reload_if_needed')
assert preload.index('console_clock_hz(dtb)?') < preload.index('download_rp1_policy_and_reload_if_needed')
assert preload.index('validate_firmware_board(dtb, handoff_dtb)?') < preload.index('download_rp1_policy_and_reload_if_needed')
assert 'console_clock_matches(firmware_hz, linux_hz, ibrd, fbrd)' in preload
assert 'failure=uart10-clock-contract' in preload
assert 'digest != crate::scmi_linux_admission::LINUX_DTB_SHA256' in preload
assert 'download_rp1_policy_and_reload_if_needed(dtb,' in preload
assert 'return handoff_preloaded_kernel(\n            handoff_dtb,' in preload
main = (repo / 'rp1_chainboot_poc/src/main.rs').read_text()
admission = main[main.index('let admitted = transport.log_rtos_samples'):main.index('crate::timer::delay_millis(500);', main.index('let admitted = transport.log_rtos_samples'))]
assert 'if !admitted' in admission and 'return Err(BootError::Rp1ImageInvalid)' in admission
assert '#[cfg(not(feature = "rp1-scmi-linux-preloaded"))]\n                                    halt();' in admission
assert admission.index('if !admitted') < admission.index('return Ok(())')
fallback = main[main.index('failure=post-cold-observer-not-reached'):main.index('let Some((sram_base, sram_size)) = debug_sram else')]
assert 'return Err(BootError::Rp1Pcie)' in fallback
print(f'PASS: R1 predicates, preload order, observer halt; {len(features.keys() - allowed)} incompatible features rejected')
