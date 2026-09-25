//! `rism` binary — thin dispatch shell. No logic lives here: parse, call
//! `rism::tools::*`, render. (rust-bestpractices.md §1/§3)

use anyhow::{Context, Result};
use clap::Parser;

use rism::cli::{Cli, Commands, DocCall, OutputFormat, TestCommands, render};
use rism::iris::IrisClient;
use rism::settings::Settings;
use rism::tools::command::{ExecuteCommandArgs, execute_command};
use rism::tools::compile::{CompileDocumentsArgs, compile_documents};
use rism::tools::documents::{
    delete_document, get_document, list_documents, put_and_compile, put_document,
};
use rism::tools::monitor::{MonitorArgs, monitor_system};
use rism::tools::serverinfo::{GetServerInfoArgs, get_server_info};
use rism::tools::sql::execute_sql;
use rism::tools::testing::{
    GetTestResultsArgs, ListTestsArgs, RunTestsArgs, get_test_results, list_tests, run_tests,
};

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
    client.negotiate_version().await;

    if let Some(args) = cli.command.to_execute_sql(&cli.namespace) {
        let res = execute_sql(&client, &args).await?;
        render::render_sql(&res, cli.format);
    } else if matches!(cli.command, Commands::Exec { .. }) {
        let Commands::Exec { command, timeout } = &cli.command else {
            unreachable!("matched above")
        };
        let args = ExecuteCommandArgs {
            command: command.clone(),
            namespace: cli.namespace.clone(),
            timeout_secs: *timeout,
        };
        render::render_command(&execute_command(&client, &args).await?, cli.format);
    } else if let Commands::Test(t) = &cli.command {
        match t {
            TestCommands::Run {
                class,
                method,
                timeout,
            } => {
                let args = RunTestsArgs {
                    test_class: class.clone(),
                    test_method: method.clone(),
                    namespace: cli.namespace.clone(),
                    timeout_secs: *timeout,
                };
                let res = run_tests(&client, &args).await?;
                render::render_run_tests(&res, cli.format);
                if res.status == "failed" {
                    // CI-friendly: `rism test run` exits non-zero on failures.
                    std::process::exit(1);
                }
            }
            TestCommands::List { filter } => {
                let args = ListTestsArgs {
                    filter: filter.clone(),
                    namespace: cli.namespace.clone(),
                };
                render::render_list_tests(&list_tests(&client, &args).await?, cli.format);
            }
            TestCommands::Results { class, limit } => {
                let args = GetTestResultsArgs {
                    test_class: class.clone(),
                    max_runs: Some(*limit),
                    namespace: cli.namespace.clone(),
                };
                render::render_results(&get_test_results(&client, &args).await?, cli.format);
            }
        }
    } else if let Commands::Monitor { raw } = &cli.command {
        let res = monitor_system(
            &client,
            &MonitorArgs {
                include_raw_metrics: *raw,
            },
        )
        .await?;
        render::render_monitor(&res, cli.format);
    } else if matches!(cli.command, Commands::Compile { .. }) {
        let Commands::Compile { names, flags } = &cli.command else {
            unreachable!("matched above")
        };
        let args = CompileDocumentsArgs {
            names: names.clone(),
            flags: Some(flags.clone()),
            namespace: cli.namespace.clone(),
        };
        render::render_compile(&compile_documents(&client, &args).await?, cli.format);
    } else if matches!(cli.command, Commands::Info) {
        let info = get_server_info(&client, &GetServerInfoArgs {}).await?;
        render::render_info(&info, cli.format);
    } else if let Some(call) = cli.command.to_doc(&cli.namespace) {
        dispatch_doc(&client, call, cli.format).await?;
    } else {
        // Commands::Mcp handled above; clap guarantees we can't reach here.
        debug_assert!(false, "unhandled command");
    }
    Ok(())
}

async fn dispatch_doc(client: &IrisClient, call: DocCall, format: OutputFormat) -> Result<()> {
    match call {
        DocCall::List(args) => {
            render::render_doc_list(&list_documents(client, &args).await?, format);
            Ok(())
        }
        DocCall::Get(args) => {
            render::render_doc_get(&get_document(client, &args).await?, format);
            Ok(())
        }
        DocCall::Put {
            mut args,
            file,
            compile,
        } => {
            args.content = read_source(&file).await.context("reading source file")?;
            let res = if compile {
                put_and_compile(client, &args).await?
            } else {
                put_document(client, &args).await?
            };
            render::render_doc_put(&res, format);
            Ok(())
        }
        DocCall::Delete(args) => {
            let v = delete_document(client, &args).await?;
            render::render_json(&v, format, || println!("deleted {}", args.name));
            Ok(())
        }
    }
}

async fn read_source(path: &str) -> Result<Vec<String>> {
    if path == "-" {
        use tokio::io::AsyncReadExt;
        let mut buf = String::new();
        tokio::io::stdin().read_to_string(&mut buf).await?;
        Ok(buf.lines().map(str::to_string).collect())
    } else {
        let text = tokio::fs::read_to_string(path).await?;
        Ok(text.lines().map(str::to_string).collect())
    }
}

fn resolve_settings(cli: &Cli) -> Result<Settings> {
    let mut s = Settings::load()?;
    if let Some(url) = &cli.url {
        s.iris_base_url.clone_from(url);
    }
    Ok(s)
}
