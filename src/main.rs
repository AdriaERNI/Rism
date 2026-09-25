//! `rism` binary — thin dispatch shell. No logic lives here: parse, call
//! `rism::tools::*`, render. (rust-bestpractices.md §1/§3)

use anyhow::Result;
use clap::Parser;

use rism::cli::{Cli, Commands, OutputFormat, render};
use rism::iris::IrisClient;
use rism::settings::Settings;
use rism::tools::sql::execute_sql;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if matches!(cli.command, Commands::Mcp) {
        // MCP mode is its own entry: stderr-only logging, stdio transport.
        return rism::mcp::serve(resolve_settings(&cli)?).await;
    }

    // CLI mode: human logs to stderr, results to stdout.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let client = IrisClient::new(resolve_settings(&cli)?)?;

    if let Some(args) = cli.command.to_execute_sql(&cli.namespace) {
        let res = execute_sql(&client, &args).await?;
        render::render_sql(&res, cli.format);
    } else {
        // Commands::Mcp handled above; clap guarantees we can't reach here.
        debug_assert!(false, "unhandled command");
        if cli.format == OutputFormat::Json {
            println!("null");
        }
    }
    Ok(())
}

fn resolve_settings(cli: &Cli) -> Result<Settings> {
    let mut s = Settings::load()?;
    if let Some(url) = &cli.url {
        s.iris_base_url.clone_from(url);
    }
    Ok(s)
}
