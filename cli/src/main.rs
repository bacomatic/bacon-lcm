// cli/src/main.rs
use clap::{Parser, Subcommand};
use bacon_lcm_cli::{
    bench::BenchCommand,
    dev::DevCommand,
    error::CliError,
    migrate::MigrateCommand,
    test::TestCommand,
};

#[derive(Debug, Parser)]
#[command(
    name = "bacon-lcm-cli",
    about = "Lossless Context Memory — development and operations CLI",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Start a local development session
    Dev(DevCommand),
    /// Run test suites
    Test(TestCommand),
    /// Run performance benchmarks
    Bench(BenchCommand),
    /// Migrate data between databases
    Migrate(MigrateCommand),
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("bacon_lcm=info".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();
    let result: Result<(), CliError> = match cli.command {
        Commands::Dev(cmd)     => cmd.run().await,
        Commands::Test(cmd)    => cmd.run().await,
        Commands::Bench(cmd)   => cmd.run().await,
        Commands::Migrate(cmd) => cmd.run().await,
    };
    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
