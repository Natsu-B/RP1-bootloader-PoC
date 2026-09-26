#!/usr/bin/env python3
"""Source admission for the bounded cold-link delta; not a hardware timing test."""
from pathlib import Path
import sys

repo = Path(__file__).resolve().parents[1]
main = (repo/'rp1_chainboot_poc/src/main.rs').read_text()
brcm = (Path(sys.argv[1])/'arch_hal/aarch64_hal/soc/src/bcm2712/brcmstb.rs').read_text()


def check(source, dependency):
    start = dependency.index('pub fn init_bcm2712_root_complex(')
    end = dependency.index('fn probe_bar_size(', start)
    init = dependency[start:end]
    assert init.count('self.start_link(1_000)?;') == 1
    wait = init.index('self.start_link(1_000)?;')
    assert wait < init.rindex('self.configure_root_bridge(cfg)?;') < init.index('self.verify_root_bridge_command()')
    assert dependency.count('self.start_link(') == 1
    for begin, end, stop in (
        ('pub(crate) fn start_rp1_image_with_debug_sram(', 'fn stock_read_address(', 'return Err(BootError::Rp1Pcie);'),
        ('fn stock_spi0_wrapper_readonly(', 'fn log_rp1_pcie_audit(', 'halt();'),
    ):
        block = source[source.index(begin):source.index(end)]
        guard = 'if matches!(err, arch_hal::soc::bcm2712::Bcm2712Error::LinkTimeout) {'
        assert block.count(guard) == 1
        branch = block.split(guard)[1].split('}')[0]
        assert 'fatal=incomplete-link-init retry=forbidden' in branch and stop in branch


check(main, brcm)
negative = 0
for bad_main, bad_brcm in (
    (main, brcm.replace('self.start_link(1_000)?;', 'self.start_link(100)?;')),
    (main.replace('fatal=incomplete-link-init retry=forbidden', 'missing', 1), brcm),
    (main.replace('[RP1STOCKSPI] fatal=incomplete-link-init retry=forbidden', 'missing'), brcm),
    (main.replace('return Err(BootError::Rp1Pcie);', 'break;'), brcm),
):
    try:
        check(bad_main, bad_brcm)
    except (AssertionError, ValueError):
        negative += 1
    else:
        raise SystemExit('invalid mutation accepted')
print(f'PASS source admission, {negative} rejected mutations; hardware NOT_RUN')
