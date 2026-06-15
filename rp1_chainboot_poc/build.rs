fn main() {
    println!("cargo:rustc-link-arg=-Tlinker/aarch64.lds");
    println!("cargo:rerun-if-changed=../linker/aarch64.lds");
    println!(
        "cargo:warning=Use `cargo xbuild` or `cargo xrun` to generate ./bin/rp1_chainboot_poc.img via objcopy."
    );
}
