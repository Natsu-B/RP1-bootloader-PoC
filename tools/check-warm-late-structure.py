#!/usr/bin/env python3
"""Offline source/synthetic regression only; does not build or admit hardware."""
import pathlib
import re
import subprocess
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASELINE = "294ec7cbd4526f96c941b6324cc35733368499e7"
FEATURE = "rp1-rtos-warm-late-record"
STUB = "rp1_chainboot_poc/src/rp1_debug_stub.rs"
APPEND = "// Append only: preserve legacy source line locations and feature-off code."
IDENTITY = [0, 1, 2, 3, 4, 96, 97, 98, 99, 100, 124, 125]


def old(path):
    return subprocess.check_output(["git", "show", f"{BASELINE}:{path}"], cwd=ROOT, text=True)


def check_tail(source):
    tail = source.split("fn log_warm_late_record(", 1)[1]
    assert "[0, 1, 2, 3, 4, 96, 97, 98, 99, 100, 124, 125]" in tail
    for code in ("base.checked_add(1024).is_none()", "base & 3 != 0", "frequency.checked_mul(122)",
                 "[120u64, 122].into_iter().enumerate()", "let ticks = frequency * seconds;",
                 "ack_counter.wrapping_add(ticks)", "read_counter().wrapping_sub(ack_counter) < ticks",
                 "crate::timer::delay_millis(1);", "let mut snapshot = [0u32; 256];",
                 "snapshot.iter_mut().enumerate()", "snapshot.chunks_exact(4).enumerate()",
                 "((base + index * 4) as *const u32).read_volatile()"):
        assert code in tail, code
    assert tail.count("((base + IDENTITY[i] * 4) as *const u32).read_volatile()") == 2
    assert tail.count("read_volatile()") == 3
    assert tail.count('asm!("dsb sy"') == 4
    assert not re.search(r"write_volatile|write_mem|read_mem|serve_with_transport|init_rp1|no-more-rp1-access=1|alloc::", tail)
    order = ["while arch_timer", "let begin_counter", " BEGIN ", "let read_begin_counter",
             "let before:", "let mut snapshot", "let after:", "let read_end_counter",
             " identity-before ", " words {:03}", " identity-after ", " END ", "observer-complete"]
    positions = [tail.index(item) for item in order]
    assert positions == sorted(positions), "capture/log ordering"
    assert tail.count("post-ack-rp1-read=1 post-ack-rp1-write=0 no-reinit=1") == 2


def main():
    source = (ROOT / STUB).read_text()
    prefix, appended = source.split(APPEND, 1)
    original = old(STUB)
    replacement = original.replace(
        "// Deliberately no target read here or any later RP1 operation.",
        f'#[cfg(feature = "{FEATURE}")] let ack_counter = arch_timer::read_counter();').replace(
        'crate::logln!("[WQ{}] observer-quiesced no-more-rp1-access=1",VERSION);',
        f'#[cfg(not(feature = "{FEATURE}"))] crate::logln!("[WQ{{}}] observer-quiesced no-more-rp1-access=1",VERSION); '
        f'#[cfg(feature = "{FEATURE}")] log_warm_late_record(base, ack[6], ack_counter);')
    assert prefix == replacement + "\n", "only two same-line changes before append"
    assert replacement.count("\n") == original.count("\n"), "legacy source offsets"
    ack = source[source.index("for i in 1..8 { target.add(i).write_volatile(ack[i]); }"):source.index("pub fn new(sram_base")]
    assert ack.index("target.write_volatile(ack[0])") < ack.index("let ack_counter") < ack.index("ack-issued") < ack.index("log_warm_late_record") < ack.index("return;")
    assert ack[:ack.index("let ack_counter")].count('asm!("dsb sy"') == 2
    check_tail(source)
    mutants = [("[120u64, 122]", "[119u64, 121]"), ("[0u32; 256]", "[0u32; 255]"),
               ("wrapping_sub(ack_counter) < ticks", "wrapping_sub(begin_counter) < ticks"),
               ("((base + index * 4)", "((base + 1024 + index * 4)"),
               ("let read_end_counter", "let missing_end_counter"),
               ("no-reinit=1", "no-more-rp1-access=1")]
    for before, after in mutants:
        try:
            check_tail(source.replace(before, after))
        except (AssertionError, ValueError):
            continue
        raise AssertionError(f"accepted negative source fixture: {before}")
    cargo_path = "rp1_chainboot_poc/Cargo.toml"
    cargo = (ROOT / cargo_path).read_text()
    assert cargo.replace(f'{FEATURE} = ["rp1-rtos-watchdog-warm-guard"]\n', "") == old(cargo_path)
    features = tomllib.loads(cargo)["features"]
    def closure(name):
        return {name}.union(*(closure(child) for child in features[name]))
    guard = appended.split("compile_error!", 1)[0]
    forbidden = set(re.findall(r'feature = "([^"]+)"', guard)) - {FEATURE}
    assert f'#[cfg(all(feature = "{FEATURE}", any(' in guard
    assert 'compile_error!("rp1-rtos-warm-late-record requires isolated warm-guard observation");' in appended
    allowed = closure(FEATURE) | {"default", "allow-fw-parts-fallback", "require-rp1-img"}
    assert not closure(FEATURE) & forbidden
    assert all(closure(name) & forbidden for name in features.keys() - allowed), "incompatible feature escaped guard"
    helper = "tools/build-rtos-reader.sh"
    assert (ROOT / helper).read_text().replace(f'case "$feature" in {FEATURE}|', 'case "$feature" in ') == old(helper)
    for path in ("Cargo.lock", "rp1_chainboot_poc/src/main.rs", "rp1_chainboot_poc/build.rs", "linker/aarch64.lds", ".cargo/config.toml"):
        assert (ROOT / path).read_text() == old(path), path
    addresses = [0x2000F800 + 4 * i for i in IDENTITY + list(range(256)) + IDENTITY]
    assert len(addresses) == 280 and min(addresses) == 0x2000F800 and max(addresses) == 0x2000FBFC
    # Synthetic absolute schedule: UART time is charged against the next ACK-relative deadline.
    for ack_counter in (100, (1 << 64) - 100):
        for first_output_ms in (400, 3000):
            now = ack_counter + 17
            starts = []
            for seconds in (120, 122):
                now = max(now, ack_counter + seconds * 1000)
                starts.append((now - ack_counter) & ((1 << 64) - 1))
                now += first_output_ms
            assert starts == [120000, max(122000, 120000 + first_output_ms)]
    print("PASS source-only: exact tail/range/order/timing, default/pinned sources, all incompatible features, 6 negative mutations, synthetic wrap/serial schedule; NOT hardware admission")


if __name__ == "__main__":
    main()
