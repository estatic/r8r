//! The `r8r` command line (spec §5.1): one binary, role by subcommand.
//! Commands and flags mirror n8n's CLI.

use crate::n8n::config::Config;
use crate::n8n::engine::{self, ExecuteOptions};
use crate::n8n::node::{Mode, Services};
use crate::n8n::nodes::Registry;
use crate::n8n::store::Store;
use crate::n8n::workflow::Workflow;
use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "r8r", version, about = "Workflow automation, compatible with n8n")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Starts r8r: editor, API, webhooks and triggers (the default).
    Start,
    /// Starts a queue-mode worker.
    Worker {
        #[arg(long)]
        concurrency: Option<usize>,
    },
    /// Starts a webhook processor (production URLs only).
    Webhook,
    /// Executes a stored workflow once and prints the result.
    Execute {
        /// Id of the workflow to execute.
        #[arg(long)]
        id: Option<String>,
        /// Deprecated in n8n: execute a workflow file without importing it.
        #[arg(long)]
        file: Option<PathBuf>,
        /// Print only the execution result as JSON.
        #[arg(long = "rawOutput")]
        raw_output: bool,
    },
    /// Imports workflows from a JSON file (one workflow or an array).
    #[command(name = "import:workflow")]
    ImportWorkflow {
        #[arg(long)]
        input: PathBuf,
    },
    /// Exports workflows as a JSON array.
    #[command(name = "export:workflow")]
    ExportWorkflow {
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        pretty: bool,
    },
    /// Imports credentials (encrypted n8n exports or plain data objects).
    #[command(name = "import:credentials")]
    ImportCredentials {
        #[arg(long)]
        input: PathBuf,
    },
    /// Exports credentials; encrypted unless --decrypted.
    #[command(name = "export:credentials")]
    ExportCredentials {
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        decrypted: bool,
        #[arg(long)]
        pretty: bool,
    },
    /// Configuration commands.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Migrates an n8n database so r8r can use it.
    #[command(name = "migrate-from-n8n")]
    MigrateFromN8n {
        #[arg(long)]
        db: String,
    },
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Validates the configuration and prints the effective values.
    Check,
}

/// Runs a non-server command; returns the process exit code.
pub async fn run(command: Command) -> i32 {
    match run_inner(command).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Error: {e:#}");
            1
        }
    }
}

async fn open() -> anyhow::Result<(Config, Store)> {
    let config = Config::load()?;
    let store = Store::open(&config.database_url, &config.encryption_key).await?;
    Ok((config, store))
}

fn read_json(path: &PathBuf) -> anyhow::Result<Value> {
    let text = std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| anyhow::anyhow!("{} is not valid JSON: {e}", path.display()))
}

fn write_output(output: Option<&PathBuf>, value: &Value, pretty: bool) -> anyhow::Result<()> {
    let text = if pretty { serde_json::to_string_pretty(value)? } else { serde_json::to_string(value)? };
    match output {
        Some(path) => std::fs::write(path, text).map_err(|e| anyhow::anyhow!("cannot write {}: {e}", path.display())),
        None => {
            println!("{text}");
            Ok(())
        }
    }
}

fn as_list(value: Value) -> Vec<Value> {
    match value {
        Value::Array(a) => a,
        other => vec![other],
    }
}

async fn run_inner(command: Command) -> anyhow::Result<i32> {
    match command {
        Command::Start => unreachable!("the server is started by main"),
        Command::Worker { .. } | Command::Webhook => {
            anyhow::bail!("queue mode (worker and webhook processes) is not implemented yet (roadmap Phase 4)")
        }
        Command::Config { action: ConfigAction::Check } => {
            let config = Config::load()?;
            for (key, value) in config.entries() {
                println!("{key}={value}");
            }
            println!("Configuration is valid.");
            Ok(0)
        }
        Command::ImportWorkflow { input } => {
            let (_, store) = open().await?;
            let workflows = as_list(read_json(&input)?);
            for wf in &workflows {
                Workflow::from_json(wf)?;
                store.save_workflow(wf).await?;
            }
            println!("Successfully imported {} workflow{}.", workflows.len(), if workflows.len() == 1 { "" } else { "s" });
            Ok(0)
        }
        Command::ExportWorkflow { id, all, output, pretty } => {
            let (_, store) = open().await?;
            let list = match (id, all) {
                (Some(id), _) => vec![store.get_workflow(&id).await?.ok_or_else(|| anyhow::anyhow!("no workflow with the id \"{id}\""))?],
                (None, true) => store.list_workflows().await?,
                (None, false) => anyhow::bail!("pass --all or --id=<id>"),
            };
            write_output(output.as_ref(), &Value::Array(list), pretty)?;
            Ok(0)
        }
        Command::ImportCredentials { input } => {
            let (_, store) = open().await?;
            let creds = as_list(read_json(&input)?);
            for c in &creds {
                let name = c["name"].as_str().ok_or_else(|| anyhow::anyhow!("a credential has no name"))?;
                let ty = c["type"].as_str().ok_or_else(|| anyhow::anyhow!("credential \"{name}\" has no type"))?;
                let id = c["id"].as_str().map(String::from).or_else(|| c["id"].as_i64().map(|i| i.to_string()));
                store.save_credential(id.as_deref(), name, ty, &c["data"]).await?;
            }
            println!("Successfully imported {} credential{}.", creds.len(), if creds.len() == 1 { "" } else { "s" });
            Ok(0)
        }
        Command::ExportCredentials { id, all, output, decrypted, pretty } => {
            let (_, store) = open().await?;
            let records = match (id, all) {
                (Some(id), _) => vec![store.get_credential(&id).await?.ok_or_else(|| anyhow::anyhow!("no credential with the id \"{id}\""))?],
                (None, true) => store.list_credentials().await?,
                (None, false) => anyhow::bail!("pass --all or --id=<id>"),
            };
            let mut list = Vec::new();
            for r in records {
                let data = if decrypted { store.decrypt_credential(&r).await? } else { json!(r.data) };
                list.push(json!({"id": r.id, "name": r.name, "type": r.cred_type, "data": data, "createdAt": r.created_at, "updatedAt": r.updated_at}));
            }
            write_output(output.as_ref(), &Value::Array(list), pretty)?;
            Ok(0)
        }
        Command::MigrateFromN8n { db } => {
            let path = db.trim_start_matches("sqlite://").trim_start_matches("sqlite:");
            let bytes = std::fs::read(path).map_err(|e| anyhow::anyhow!("cannot read the n8n database {path}: {e}"))?;
            if !bytes.starts_with(b"SQLite format 3\0") {
                anyhow::bail!("{path} is not an n8n SQLite database");
            }
            anyhow::bail!("migrating {path} from n8n is not implemented yet (roadmap Phase 3)")
        }
        Command::Execute { id, file, raw_output } => execute(id, file, raw_output).await,
    }
}

async fn execute(id: Option<String>, file: Option<PathBuf>, raw_output: bool) -> anyhow::Result<i32> {
    let (config, store) = open().await?;
    let workflow_json = match (id, file) {
        (Some(id), _) => store.get_workflow(&id).await?.ok_or_else(|| anyhow::anyhow!("no workflow with the id \"{id}\""))?,
        (None, Some(path)) => read_json(&path)?,
        (None, None) => anyhow::bail!("\"--id\" has to be set"),
    };
    let workflow = Workflow::from_json(&workflow_json)?;
    let registry = Registry::default();
    let execution_id = store.create_execution(&workflow_json, "cli").await?;
    let services = Services::new(config, Some(store.clone()));
    let options = ExecuteOptions::new(Mode::Cli, execution_id.clone());
    let result = match engine::execute(&workflow, &registry, &services, options).await {
        Ok(r) => r,
        Err(e) => {
            let _ = store.finish_execution(&execution_id, "error", &json!({"error": e.to_string()}), None).await;
            anyhow::bail!("{e}");
        }
    };
    let irun = result.to_irun();
    store.finish_execution(&execution_id, &result.status, &irun, None).await?;
    if !raw_output {
        println!("Execution was {}:", if result.status == "success" { "successful" } else { "NOT successful" });
    }
    println!("{}", serde_json::to_string_pretty(&irun)?);
    Ok(if result.status == "success" { 0 } else { 1 })
}
