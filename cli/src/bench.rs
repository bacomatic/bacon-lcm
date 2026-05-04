// cli/src/bench.rs
use clap::Args;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use crate::error::CliError;

#[derive(Debug, Args)]
pub struct BenchCommand {
    /// Also run TypeScript benchmarks and print comparison (stub)
    #[arg(short, long)]
    pub compare: bool,

    /// Write raw Criterion output to this file
    #[arg(short, long)]
    pub export: Option<PathBuf>,
}

impl BenchCommand {
    pub async fn run(self) -> Result<(), CliError> {
        if self.compare {
            eprintln!("note: TypeScript comparison not yet implemented; running Rust benchmarks only");
        }

        let mut cmd = Command::new("cargo");
        cmd.args(["bench", "-p", "bacon-lcm-core"]);

        if let Some(path) = self.export {
            let output = cmd
                .stdout(Stdio::piped())
                .output()
                .map_err(CliError::Io)?;
            std::fs::write(&path, &output.stdout).map_err(CliError::Io)?;
            println!("Benchmark output written to {}", path.display());
            if !output.status.success() {
                return Err(CliError::Other(format!(
                    "cargo bench exited with {}",
                    output.status
                )));
            }
        } else {
            let status = cmd.status().map_err(CliError::Io)?;
            if !status.success() {
                return Err(CliError::Other(format!(
                    "cargo bench exited with {}",
                    status
                )));
            }
        }

        Ok(())
    }
}