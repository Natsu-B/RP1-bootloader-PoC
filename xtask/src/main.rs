use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const TARGET: &str = "aarch64-unknown-none-softfloat";
const PACKAGE: &str = "rp1_chainboot_poc";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("xtask: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let cmd = args.next().unwrap_or_else(|| "build".to_owned());
    let cargo_args = normalize_cargo_args(args);
    match cmd.as_str() {
        "build" => build(false, &cargo_args),
        "run" => build(true, &cargo_args),
        "-h" | "--help" | "help" => {
            print_help();
            Ok(())
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

fn print_help() {
    println!("usage:");
    println!("  cargo run         # build ELF and raw image");
    println!("  cargo xbuild      # same as build");
    println!("  cargo xrun        # build and print deploy placeholder");
    println!("  cargo xrun --feature log-semihosting");
}

fn normalize_cargo_args<I>(args: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    args.into_iter()
        .map(|arg| {
            if arg == "--feature" {
                "--features".to_owned()
            } else if let Some(features) = arg.strip_prefix("--feature=") {
                format!("--features={features}")
            } else {
                arg
            }
        })
        .collect()
}

fn build(run_placeholder: bool, cargo_args: &[String]) -> Result<(), String> {
    let manifest_dir = workspace_root()?;
    let mut cargo = Command::new("cargo");
    cargo
        .current_dir(&manifest_dir)
        .arg("build")
        .arg("-Z")
        .arg("build-std=core,alloc,compiler_builtins")
        .arg("-Z")
        .arg("build-std-features=compiler-builtins-mem")
        .arg("-p")
        .arg(PACKAGE)
        .arg("--target")
        .arg(TARGET)
        .arg("--release")
        .args(cargo_args);

    run_status(&mut cargo, "cargo build")?;

    let out_dir = manifest_dir.join("bin");
    fs::create_dir_all(&out_dir).map_err(|err| format!("create bin dir: {err}"))?;

    let elf_src = manifest_dir
        .join("target")
        .join(TARGET)
        .join("release")
        .join(PACKAGE);
    let elf_dst = out_dir.join(format!("{PACKAGE}.elf"));
    let img_dst = out_dir.join(format!("{PACKAGE}.img"));

    fs::copy(&elf_src, &elf_dst)
        .map_err(|err| format!("copy {} to {}: {err}", display(&elf_src), display(&elf_dst)))?;

    let objcopy = find_objcopy().ok_or_else(|| {
        "objcopy not found; tried rust-objcopy, llvm-objcopy, aarch64-none-elf-objcopy, aarch64-linux-gnu-objcopy".to_owned()
    })?;
    let mut objcopy_cmd = Command::new(&objcopy);
    objcopy_cmd
        .arg("-O")
        .arg("binary")
        .arg(&elf_src)
        .arg(&img_dst);
    run_status(&mut objcopy_cmd, "objcopy")?;

    println!("ELF: {}", display(&elf_dst));
    println!("IMG: {}", display(&img_dst));
    if run_placeholder {
        println!("xrun: deploy/run is not implemented yet; generated raw image above.");
    }
    Ok(())
}

fn workspace_root() -> Result<PathBuf, String> {
    let manifest_dir = env::var_os("CARGO_MANIFEST_DIR")
        .ok_or_else(|| "CARGO_MANIFEST_DIR is not set".to_owned())?;
    PathBuf::from(manifest_dir)
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "xtask manifest has no parent".to_owned())
}

fn find_objcopy() -> Option<PathBuf> {
    [
        "rust-objcopy",
        "llvm-objcopy",
        "aarch64-none-elf-objcopy",
        "aarch64-linux-gnu-objcopy",
    ]
    .iter()
    .find_map(which)
}

fn which(name: &&str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(OsStr::new(name)))
        .find(|candidate| candidate.is_file())
}

fn run_status(cmd: &mut Command, label: &str) -> Result<(), String> {
    let status = cmd.status().map_err(|err| format!("{label}: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label} failed with {status}"))
    }
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[allow(dead_code)]
fn io_err(context: &str, err: io::Error) -> String {
    format!("{context}: {err}")
}
