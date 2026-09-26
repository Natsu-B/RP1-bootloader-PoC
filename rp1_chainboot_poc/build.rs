fn main() {
    // Commissioning is a sealed read-only cold path, never a debug-mode mix.
    if std::env::var_os("CARGO_FEATURE_RP1_SCMI_LINUX_PRELOADED").is_some() {
        for (key, _) in std::env::vars() {
            if let Some(feature) = key.strip_prefix("CARGO_FEATURE_") {
                assert!(matches!(feature, "RP1_SCMI_LINUX_PRELOADED" | "RP1_RTOS_RECORD"
                    | "RP1_GDB_DEBUG_STUB" | "TFTP_BOOT" | "TFTP_INITRAMFS"
                    | "REQUIRE_RP1_IMG" | "LOG_UART"),
                    "rp1-scmi-linux-preloaded incompatible feature: {feature}");
            }
        }
    }
    println!("cargo:rustc-link-arg=-Tlinker/aarch64.lds");
    println!("cargo:rerun-if-changed=../linker/aarch64.lds");
    println!(
        "cargo:warning=Use `cargo xbuild` or `cargo xrun` to generate ./bin/rp1_chainboot_poc.img via objcopy."
    );
}
