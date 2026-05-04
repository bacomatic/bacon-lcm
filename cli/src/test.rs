// cli/src/test.rs
use clap::Args;
use std::process::Command;
use crate::error::CliError;

#[derive(Debug, Args)]
pub struct TestCommand {
    /// Run integration tests (requires Docker / Postgres)
    #[arg(short, long)]
    pub integration: bool,

    /// Run property-based tests (proptest)
    #[arg(short, long)]
    pub property: bool,

    /// Compile and run benchmarks in test mode
    #[arg(short, long)]
    pub benchmark: bool,
}

impl TestCommand {
    pub async fn run(self) -> Result<(), CliError> {
        let mut ran_something = false;

        if self.integration {
            run_cargo(&["test", "-p", "bacon-lcm-daemon", "--test", "*"])?;
            ran_something = true;
        }
        if self.property {
            run_cargo(&["test", "-p", "bacon-lcm-core", "--test", "property_tests"])?;
            ran_something = true;
        }
        if self.benchmark {
            run_cargo(&["test", "--benches", "-p", "bacon-lcm-core"])?;
            ran_something = true;
        }
        if !ran_something {
            // Default: run all unit tests
            run_cargo(&["test", "--workspace", "--lib"])?;
        }
        Ok(())
    }
}

fn run_cargo(args: &[&str]) -> Result<(), CliError> {
    let status = Command::new("cargo")
        .args(args)
        .status()
        .map_err(CliError::Io)?;
    if !status.success() {
        return Err(CliError::Other(format!(
            "`cargo {}` exited with {}",
            args.join(" "),
            status
        )));
    }
    Ok(())
}
