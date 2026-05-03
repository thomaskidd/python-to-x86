use anyhow::{Context, Result};
use clap::Parser;
use rand::{Rng, SeedableRng};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

#[derive(Parser, Debug)]
#[command(name = "pyx86-bench", about = "Differential test bench for the pyx86 compiler")]
struct Cli {
    #[arg(long, default_value_t = 1)]
    tier: u8,
    #[arg(long, default_value_t = num_cpus::get())]
    jobs: usize,
    #[arg(long, default_value = "**")]
    filter: String,
    #[arg(long, default_value = "tests")]
    corpus_root: PathBuf,
    #[arg(long, default_value = "cargo run -q -p pyx86 --release --")]
    compiler: String,
    #[arg(long)]
    json_out: Option<PathBuf>,
    #[arg(long)]
    keep_tmp: bool,
    /// RNG seed for input generation. Defaults to a per-test
    /// deterministic seed so reruns reproduce.
    #[arg(long)]
    seed: Option<u64>,
}

#[derive(Deserialize, Debug)]
struct TierToml {
    tier: u8,
    #[serde(default)]
    iter_at: TierOverrides,
}

#[derive(Deserialize, Debug, Default)]
struct TierOverrides {
    tier1: Option<u32>,
    tier2: Option<u32>,
    tier3: Option<u32>,
    tier4: Option<u32>,
}

impl TierOverrides {
    fn get(&self, tier: u8) -> Option<u32> {
        match tier {
            1 => self.tier1,
            2 => self.tier2,
            3 => self.tier3,
            4 => self.tier4,
            _ => None,
        }
    }
}

fn default_iterations(tier: u8) -> u32 {
    match tier {
        1 => 1,
        2 => 10,
        3 => 1_000,
        4 => 100_000,
        _ => 0,
    }
}

#[derive(Deserialize, Debug)]
struct StrategyToml {
    #[serde(rename = "arg", default)]
    args: Vec<ArgStrategy>,
}

#[derive(Deserialize, Debug, Clone)]
struct ArgStrategy {
    #[serde(rename = "type")]
    ty: String,
    /// Inclusive `[min, max]`. Required for `i64`.
    range: Option<[i64; 2]>,
}

#[derive(Serialize, Debug, Clone)]
struct TestResult {
    name: String,
    passed: bool,
    skipped: bool,
    iterations: u32,
    compile_ms: f64,
    run_ms: f64,
    failure: Option<Failure>,
}

#[derive(Serialize, Debug, Clone)]
struct Failure {
    kind: String,
    message: String,
    input_repr: Option<String>,
    expected: Option<String>,
    actual: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(cli.jobs)
        .build_global();

    let corr_dir = cli.corpus_root.join("correctness");
    if !corr_dir.exists() {
        eprintln!("no correctness corpus at {}", corr_dir.display());
        return Ok(());
    }

    let glob_pat = format!("{}/{}", corr_dir.display(), cli.filter);
    let mut tests: Vec<PathBuf> = glob::glob(&glob_pat)?
        .filter_map(Result::ok)
        .filter(|p| p.is_dir() && p.join("program.py").is_file())
        .collect();
    tests.sort();

    let started = Instant::now();
    let results: Vec<TestResult> = tests
        .par_iter()
        .map(|dir| run_one(dir, &cli))
        .collect();

    let wall = started.elapsed().as_secs_f64();
    print_summary(&results, wall);

    if let Some(path) = &cli.json_out {
        let payload = serde_json::json!({
            "tier": cli.tier,
            "wall_seconds": wall,
            "correctness": results,
        });
        fs::write(path, serde_json::to_string_pretty(&payload)?)?;
    }

    let any_failed = results.iter().any(|r| !r.passed && !r.skipped);
    if any_failed {
        std::process::exit(1);
    }
    Ok(())
}

fn run_one(dir: &Path, cli: &Cli) -> TestResult {
    let name = dir.file_name().unwrap().to_string_lossy().to_string();
    match run_correctness(dir, cli) {
        Ok(r) => r,
        Err(e) => TestResult {
            name,
            passed: false,
            skipped: false,
            iterations: 0,
            compile_ms: 0.0,
            run_ms: 0.0,
            failure: Some(Failure {
                kind: "harness".into(),
                message: format!("{:#}", e),
                input_repr: None,
                expected: None,
                actual: None,
            }),
        },
    }
}

fn run_correctness(dir: &Path, cli: &Cli) -> Result<TestResult> {
    let name = dir.file_name().unwrap().to_string_lossy().to_string();
    let tier_toml: TierToml = {
        let s = fs::read_to_string(dir.join("tier.toml"))
            .with_context(|| format!("read tier.toml in {}", dir.display()))?;
        toml::from_str(&s)?
    };

    if tier_toml.tier > cli.tier {
        return Ok(skipped(&name));
    }

    let strategy = if dir.join("strategy.toml").exists() {
        let s = fs::read_to_string(dir.join("strategy.toml"))
            .with_context(|| format!("read strategy.toml in {}", dir.display()))?;
        let parsed: StrategyToml =
            toml::from_str(&s).with_context(|| format!("parse strategy.toml in {}", dir.display()))?;
        Some(parsed.args)
    } else {
        None
    };

    let has_inputs = strategy.as_ref().map(|a| !a.is_empty()).unwrap_or(false);
    let iterations = if has_inputs {
        tier_toml.iter_at.get(cli.tier).unwrap_or_else(|| default_iterations(cli.tier))
    } else {
        1
    };

    let tmp = tempfile::Builder::new()
        .prefix(&format!("pyx86bench-{}-", name))
        .tempdir()?;
    let elf_path = tmp.path().join(format!("{}.elf", name));

    let compile_start = Instant::now();
    let comp = run_compiler(&cli.compiler, &dir.join("program.py"), &elf_path);
    let compile_ms = compile_start.elapsed().as_secs_f64() * 1000.0;
    if !comp.success {
        return Ok(TestResult {
            name,
            passed: false,
            skipped: false,
            iterations: 0,
            compile_ms,
            run_ms: 0.0,
            failure: Some(Failure {
                kind: "compile".into(),
                message: comp.stderr,
                input_repr: None,
                expected: None,
                actual: None,
            }),
        });
    }

    // Per-test deterministic seed (test name hash) so reruns reproduce.
    let seed = cli.seed.unwrap_or_else(|| stable_seed(&name));
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);

    let run_start = Instant::now();
    let mut failure: Option<Failure> = None;
    for _ in 0..iterations {
        let args = generate_inputs(strategy.as_deref().unwrap_or(&[]), &mut rng)?;
        let py = run_cpython(&dir.join("program.py"), &args)?;
        let su = run_subject(&elf_path, &args)?;
        if py.exit_code != su.exit_code || py.stdout != su.stdout {
            failure = Some(Failure {
                kind: "diff".into(),
                message: format!("py exit={}, subject exit={}", py.exit_code, su.exit_code),
                input_repr: Some(format_args_python(&args)),
                expected: Some(py.stdout),
                actual: Some(su.stdout),
            });
            break;
        }
    }
    let run_ms = run_start.elapsed().as_secs_f64() * 1000.0;

    if cli.keep_tmp {
        let _ = tmp.keep();
    }

    Ok(TestResult {
        name,
        passed: failure.is_none(),
        skipped: false,
        iterations,
        compile_ms,
        run_ms,
        failure,
    })
}

fn skipped(name: &str) -> TestResult {
    TestResult {
        name: name.into(),
        passed: true,
        skipped: true,
        iterations: 0,
        compile_ms: 0.0,
        run_ms: 0.0,
        failure: None,
    }
}

/// A single generated input: one value per arg in the strategy.
#[derive(Debug, Clone)]
enum ArgValue {
    I64(i64),
}

fn generate_inputs(strategy: &[ArgStrategy], rng: &mut impl Rng) -> Result<Vec<ArgValue>> {
    strategy
        .iter()
        .map(|a| match a.ty.as_str() {
            "i64" => {
                let [lo, hi] = a
                    .range
                    .ok_or_else(|| anyhow::anyhow!("i64 arg missing required `range = [lo, hi]`"))?;
                anyhow::ensure!(lo <= hi, "i64 arg range has lo > hi: [{}, {}]", lo, hi);
                Ok(ArgValue::I64(rng.gen_range(lo..=hi)))
            }
            other => anyhow::bail!("strategy arg type `{}` is not supported in v0.3", other),
        })
        .collect()
}

fn format_args_python(args: &[ArgValue]) -> String {
    args.iter()
        .map(|a| match a {
            ArgValue::I64(v) => v.to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_args_argv(args: &[ArgValue]) -> Vec<String> {
    args.iter()
        .map(|a| match a {
            ArgValue::I64(v) => v.to_string(),
        })
        .collect()
}

/// FNV-1a-ish hash so the same test name gives the same default seed.
fn stable_seed(name: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in name.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

struct CompileStatus {
    success: bool,
    stderr: String,
}

fn run_compiler(compiler_cmd: &str, src: &Path, out: &Path) -> CompileStatus {
    let parts: Vec<&str> = compiler_cmd.split_whitespace().collect();
    let Some((program, args)) = parts.split_first() else {
        return CompileStatus {
            success: false,
            stderr: "empty --compiler command".into(),
        };
    };
    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd.arg(src);
    cmd.arg("-o").arg(out);
    match cmd.output() {
        Ok(o) => CompileStatus {
            success: o.status.success(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        },
        Err(e) => CompileStatus {
            success: false,
            stderr: format!("failed to spawn compiler `{}`: {}", program, e),
        },
    }
}

struct RunOutput {
    exit_code: i32,
    stdout: String,
}

fn run_cpython(src: &Path, args: &[ArgValue]) -> Result<RunOutput> {
    let dir = src.parent().context("source has no parent")?;
    let stem = src.file_stem().context("source has no stem")?.to_string_lossy();
    let py_args = format_args_python(args);
    let snippet = format!(
        "import sys; sys.path.insert(0, r'{}'); from {} import main; print(repr(main({})))",
        dir.display(),
        stem,
        py_args,
    );
    let output = Command::new("python3").arg("-c").arg(&snippet).output()?;
    Ok(RunOutput {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
    })
}

fn run_subject(elf: &Path, args: &[ArgValue]) -> Result<RunOutput> {
    let argv = format_args_argv(args);
    let output = Command::new(elf).args(&argv).output()?;
    Ok(RunOutput {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
    })
}

fn print_summary(results: &[TestResult], wall: f64) {
    println!("correctness:");
    if results.is_empty() {
        println!("  (no tests discovered)");
    }
    for r in results {
        if r.skipped {
            continue;
        }
        let mark = if r.passed { "ok  " } else { "FAIL" };
        if r.passed {
            println!(
                "  {} {:30}  {:>4} iter  compile {:>5.1} ms  run {:>5.1} ms",
                mark, r.name, r.iterations, r.compile_ms, r.run_ms
            );
        } else if let Some(f) = &r.failure {
            println!("  {} {:30}  [{}] {}", mark, r.name, f.kind, f.message.lines().next().unwrap_or(""));
            if let Some(input) = &f.input_repr {
                println!("       input:    main({})", input);
            }
            if let (Some(exp), Some(act)) = (&f.expected, &f.actual) {
                println!("       expected: {}", exp.trim_end());
                println!("       actual  : {}", act.trim_end());
            } else if f.message.lines().count() > 1 {
                for line in f.message.lines().take(5) {
                    println!("       {}", line);
                }
            }
        }
    }
    let pass = results.iter().filter(|r| r.passed && !r.skipped).count();
    let fail = results.iter().filter(|r| !r.passed && !r.skipped).count();
    let skip = results.iter().filter(|r| r.skipped).count();
    let total = pass + fail;
    println!();
    println!(
        "{}/{} passed, {} failed, {} skipped. {:.2}s wall.",
        pass, total, fail, skip, wall
    );
}
