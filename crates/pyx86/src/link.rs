use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, bail, Result};

pub fn clang_emit_elf(ll: &Path, out: &Path, opt_level: u8) -> Result<()> {
    run_clang(&[
        &format!("-O{}", opt_level),
        "-x",
        "ir",
        ll.to_str().ok_or_else(|| anyhow!("non-utf8 ll path"))?,
        "-o",
        out.to_str().ok_or_else(|| anyhow!("non-utf8 out path"))?,
        // Link libm so `floor`, `ceil`, `tan`, `pow`, etc. resolve.
        "-lm",
    ])
}

pub fn clang_emit_asm(ll: &Path, out: &Path, opt_level: u8) -> Result<()> {
    run_clang(&[
        &format!("-O{}", opt_level),
        "-S",
        "-x",
        "ir",
        ll.to_str().ok_or_else(|| anyhow!("non-utf8 ll path"))?,
        "-o",
        out.to_str().ok_or_else(|| anyhow!("non-utf8 out path"))?,
    ])
}

fn run_clang(args: &[&str]) -> Result<()> {
    let output = Command::new("clang").args(args).output().map_err(|e| {
        anyhow!(
            "internal: failed to spawn `clang`: {}\n  = note: install LLVM (e.g. `sudo apt install clang`) and retry",
            e
        )
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("clang failed with status {}:\n{}", output.status, stderr);
    }
    Ok(())
}
