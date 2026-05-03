use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, ValueEnum};

mod check;
mod codegen;
mod hir;
mod link;
mod parser;

#[derive(Parser, Debug)]
#[command(name = "pyx86", about = "Compile a Python source file to a native x86-64 binary")]
struct Cli {
    /// Input .py file.
    input: PathBuf,
    /// Output path. The shape of this output depends on --emit.
    #[arg(short = 'o', long = "output")]
    output: PathBuf,
    /// What to emit.
    #[arg(long, value_enum, default_value_t = Emit::Elf)]
    emit: Emit,
    /// LLVM optimization level passed through to clang as -O<n>.
    #[arg(long, default_value_t = 2)]
    opt_level: u8,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum Emit {
    /// LLVM IR text (.ll)
    Ll,
    /// Native assembly (.s)
    Asm,
    /// Linked ELF executable
    Elf,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("pyx86 error: {:#}", e);
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(&cli.input)
        .map_err(|e| anyhow::anyhow!("read {}: {}", cli.input.display(), e))?;
    let module = parser::parse(&source, &cli.input)?;
    let program = check::lower(&module)?;

    let basename = cli
        .input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "module".to_string());

    let ll_text = codegen::emit_ll(&program, &basename);

    match cli.emit {
        Emit::Ll => {
            std::fs::write(&cli.output, ll_text)?;
        }
        Emit::Asm => {
            let tmp = tempfile::Builder::new().suffix(".ll").tempfile()?;
            std::fs::write(tmp.path(), &ll_text)?;
            link::clang_emit_asm(tmp.path(), &cli.output, cli.opt_level)?;
        }
        Emit::Elf => {
            let tmp = tempfile::Builder::new().suffix(".ll").tempfile()?;
            std::fs::write(tmp.path(), &ll_text)?;
            link::clang_emit_elf(tmp.path(), &cli.output, cli.opt_level)?;
        }
    }
    Ok(())
}
