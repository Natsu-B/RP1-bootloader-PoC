#!/usr/bin/env python3
"""No Cargo/network needed: actual build guard, R1 predicates, preload ordering."""
import argparse
import hashlib
import os
from pathlib import Path
import subprocess
import struct
import tempfile
import tomllib

repo = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--dtb', type=Path, help='Actual candidate input DTB; compare against compiled Rust seal')
parser.add_argument('--time-anchor', action='store_true')
parser.add_argument('--amp-service', action='store_true')
parser.add_argument('--amp-proc1', action='store_true')
parser.add_argument('--amp-observation', action='store_true')
parser.add_argument('--amp-proc1-profile', action='store_true')
parser.add_argument('--amp-proc1-local-stack', action='store_true')
parser.add_argument('--amp-proc1-no-hooks', action='store_true')
parser.add_argument('--amp-clk-sys-200', action='store_true')
parser.add_argument('--firmware', type=Path, help='Actual sealed AG01 ELF for its vector/hash test')
args = parser.parse_args()
if args.amp_proc1 and not args.amp_service: parser.error('--amp-proc1 requires --amp-service')
if args.amp_observation and not args.amp_proc1: parser.error('--amp-observation requires --amp-proc1')
if args.amp_proc1_profile and not args.amp_observation: parser.error('--amp-proc1-profile requires --amp-observation')
if args.amp_proc1_local_stack and not args.amp_proc1_profile: parser.error('--amp-proc1-local-stack requires --amp-proc1-profile')
if args.amp_proc1_no_hooks and not args.amp_proc1_local_stack: parser.error('--amp-proc1-no-hooks requires --amp-proc1-local-stack')
if args.amp_clk_sys_200 and not args.amp_observation: parser.error('--amp-clk-sys-200 requires --amp-observation')
if args.amp_service and not args.firmware: parser.error('--amp-service requires --firmware')
feature = 'rp1-scmi-linux-preloaded'
manifest = tomllib.loads((repo / 'rp1_chainboot_poc/Cargo.toml').read_text())
features = manifest['features']
allowed = {feature, 'rp1-rtos-record', 'rp1-gdb-debug-stub', 'tftp-boot',
           'tftp-initramfs', 'require-rp1-img', 'log-uart', 'rp1-time-anchor'}
if args.amp_service: allowed.add('rp1-amp-service')
if args.amp_proc1: allowed.add('rp1-amp-proc1')
if args.amp_observation: allowed.add('rp1-amp-observation')
if args.amp_proc1_profile: allowed.add('rp1-amp-proc1-profile')
if args.amp_proc1_local_stack: allowed.add('rp1-amp-proc1-local-stack')
if args.amp_proc1_no_hooks: allowed.add('rp1-amp-proc1-no-hooks')
if args.amp_clk_sys_200: allowed.add('rp1-amp-clk-sys-200')
assert features[feature] == ['rp1-rtos-record', 'tftp-initramfs']
assert features['rp1-time-anchor'] == [feature]
assert features['rp1-amp-service'] == ['rp1-time-anchor']
assert features['rp1-amp-proc1'] == ['rp1-amp-service']
assert features['rp1-amp-observation'] == ['rp1-amp-proc1']
assert features['rp1-amp-proc1-profile'] == ['rp1-amp-observation']
assert features['rp1-amp-proc1-local-stack'] == ['rp1-amp-proc1-profile']
assert features['rp1-amp-proc1-no-hooks'] == ['rp1-amp-proc1-local-stack']
assert features['rp1-amp-clk-sys-200'] == ['rp1-amp-observation']
env = {k: v for k, v in os.environ.items() if not k.startswith('CARGO_FEATURE_')}
env.update({'CARGO_FEATURE_' + f.upper().replace('-', '_'): '1' for f in allowed})
with tempfile.TemporaryDirectory(prefix='scmi-linux-test-') as scratch:
    guard = Path(scratch) / 'build-guard'
    subprocess.run(['rustc', '--edition=2024', repo / 'rp1_chainboot_poc/build.rs', '-o', guard], check=True)
    subprocess.run([guard], env=env, check=True, capture_output=True)
    for forbidden in features.keys() - allowed - {'rp1-amp-service','rp1-amp-proc1','rp1-amp-observation','rp1-amp-proc1-profile','rp1-amp-proc1-local-stack','rp1-amp-proc1-no-hooks','rp1-amp-clk-sys-200'}:
        trial = dict(env, **{'CARGO_FEATURE_' + forbidden.upper().replace('-', '_'): '1'})
        result = subprocess.run([guard], env=trial, capture_output=True, text=True)
        assert result.returncode != 0 and 'incompatible feature:' in result.stderr, forbidden
    tests = Path(scratch) / 'r1-tests'
    cfg = ['--cfg', 'feature="rp1-time-anchor"'] if args.time_anchor else []
    if args.amp_service: cfg += ['--cfg','feature="rp1-amp-service"']
    if args.amp_proc1: cfg += ['--cfg','feature="rp1-amp-proc1"']
    if args.amp_observation: cfg += ['--cfg','feature="rp1-amp-observation"']
    if args.amp_proc1_profile: cfg += ['--cfg','feature="rp1-amp-proc1-profile"']
    if args.amp_proc1_local_stack: cfg += ['--cfg','feature="rp1-amp-proc1-local-stack"']
    if args.amp_proc1_no_hooks: cfg += ['--cfg','feature="rp1-amp-proc1-no-hooks"']
    if args.amp_clk_sys_200: cfg += ['--cfg','feature="rp1-amp-clk-sys-200"']
    subprocess.run(['rustc', '--edition=2024', '--test', repo / 'rp1_chainboot_poc/src/scmi_linux_admission.rs', *cfg, '-o', tests], check=True)
    test_env = {k: v for k, v in os.environ.items() if not k.startswith('SCMI_LINUX_DTB_')}
    if args.dtb:
        data = args.dtb.read_bytes()
        test_env.update(SCMI_LINUX_DTB_PATH=str(args.dtb.resolve()),
                        SCMI_LINUX_DTB_SHA256=hashlib.sha256(data).hexdigest())
    subprocess.run([tests], env=test_env, check=True)
    if args.amp_service:
        service_tests = Path(scratch)/'amp-service-tests'
        source=repo/'rp1_chainboot_poc/src/amp_service_admission.rs'
        if args.amp_proc1:
            source=Path(scratch)/'proc1-host.rs'
            source.write_text(f'#[path="{repo}/rp1_chainboot_poc/src/amp_service_admission.rs"] mod amp_service_admission;\n'
                              f'#[path="{repo}/rp1_chainboot_poc/src/amp_proc1_admission.rs"] mod amp_proc1_admission;\n')
        subprocess.run(['rustc','--edition=2024','--test',*cfg,source,'-o',service_tests],check=True)
        raw=args.firmware.read_bytes(); header=struct.unpack_from('<16sHHIIIIIHHHHHH',raw)
        vector_offset=next(struct.unpack_from('<8I',raw,header[5]+i*header[9])[1]
                           for i in range(header[10]) if struct.unpack_from('<8I',raw,header[5]+i*header[9])[0]==1)
        prefix='AMP_PROC1' if args.amp_proc1 else 'AMP_SERVICE'
        service_env={k:v for k,v in os.environ.items() if not k.startswith(('AMP_SERVICE_FIRMWARE_','AMP_SERVICE_SCMI_','AMP_PROC1_FIRMWARE_','AMP_PROC1_SCMI_'))}
        service_env.update({prefix+'_FIRMWARE_SHA256':hashlib.sha256(raw).hexdigest(),
                           prefix+'_SCMI_HANDLER':f'{struct.unpack_from("<I",raw,vector_offset+73*4)[0]:08x}'})
        subprocess.run([service_tests],env=service_env,check=True)
        bad_env={**service_env,prefix+'_SCMI_HANDLER':'20000291'}
        test_name='amp_proc1_admission::tests::actual_firmware_seal_if_supplied' if args.amp_proc1 else 'tests::actual_firmware_seal_if_supplied'
        rejected=subprocess.run([service_tests,test_name,'--exact'],env=bad_env,capture_output=True)
        assert rejected.returncode!=0,'legacy SCMI handler accepted for AG01'
        bad_env={**service_env,prefix+'_FIRMWARE_SHA256':'00'*32}
        rejected=subprocess.run([service_tests,test_name,'--exact'],env=bad_env,capture_output=True)
        assert rejected.returncode!=0,'unsealed AG01 firmware accepted'
    if args.dtb:
        # A changed actual DT file must fail the same compiled-source test.
        bad = Path(scratch) / 'changed.dtb'
        changed = bytearray(data); changed[-1] ^= 1; bad.write_bytes(changed)
        bad_env = dict(test_env, SCMI_LINUX_DTB_PATH=str(bad),
                       SCMI_LINUX_DTB_SHA256=hashlib.sha256(changed).hexdigest())
        rejected = subprocess.run([tests, 'supplied_dtb_seal', '--exact'], env=bad_env, capture_output=True)
        assert rejected.returncode != 0, 'changed DT accepted'
        print(f'PASS actual DT seal {test_env["SCMI_LINUX_DTB_SHA256"]}; changed bytes rejected')
    tests = Path(scratch) / 'anchor-tests'
    subprocess.run(['rustc', '--edition=2024', '--test', repo / 'rp1_chainboot_poc/src/rp1_time_anchor.rs', '-o', tests], check=True)
    subprocess.run([tests], check=True)
    tests = Path(scratch) / 'pcie-snapshot-tests'
    subprocess.run(['rustc', '--edition=2024', '--test', repo / 'rp1_chainboot_poc/src/rp1_pcie_snapshot.rs', '-o', tests], check=True)
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
assert admission.index('if !admitted') < admission.index('rp1_time_anchor::log(&rp1)') < admission.index('return Ok(())')
assert '#[cfg(feature = "rp1-time-anchor")] rp1_time_anchor::log(&rp1)' in admission
fallback = main[main.index('failure=post-cold-observer-not-reached'):main.index('let Some((sram_base, sram_size)) = debug_sram else')]
assert 'return Err(BootError::Rp1Pcie)' in fallback
if args.amp_proc1:
    observer=(repo/'rp1_chainboot_poc/src/rp1_debug_stub.rs').read_text()
    assert 'map.mmio_base(0x0001_4000,0x1c)' in observer
    assert 'self.translate_rp1_addr(0x4001_4000' not in observer
print(f'PASS: R1 predicates, preload order, observer halt; {len(features.keys() - allowed)} incompatible features rejected; AMP-service tested={args.amp_service}')
