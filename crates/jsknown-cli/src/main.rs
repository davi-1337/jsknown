use anyhow::Result;
use clap::{Parser, Subcommand};
use jsknown_core::config::Config;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "jsknown")]
#[command(
    about = "JavaScript asset recovery, chunk fetching, source map reversal, and AST analysis"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Serve(ServeArgs),
}

#[derive(Parser, Clone)]
struct ServeArgs {
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 3333)]
    port: u16,
    #[arg(long, default_value = "default")]
    project: String,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long = "scope")]
    scopes: Vec<String>,
    #[arg(long, default_value_t = 2)]
    rate_per_second: u32,
    #[arg(long, default_value_t = 0)]
    rate_per_minute: u32,
    #[arg(long, default_value_t = 5)]
    fetch_concurrency: usize,
    #[arg(long, default_value_t = 25_000_000)]
    max_body_bytes: usize,
}

impl From<ServeArgs> for Config {
    fn from(args: ServeArgs) -> Self {
        Config {
            host: args.host,
            port: args.port,
            project: args.project,
            output_root: args.output,
            scope_patterns: args.scopes,
            rate_per_second: args.rate_per_second,
            rate_per_minute: args.rate_per_minute,
            fetch_concurrency: args.fetch_concurrency,
            max_body_bytes: args.max_body_bytes,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    match cli.command {
        Commands::Serve(args) => jsknown_server::serve(args.into()).await,
    }
}
