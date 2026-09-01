use anyhow::Result;
use clap::{Parser, Subcommand};
use msl_db::Database;
use std::{net::SocketAddr, path::PathBuf};

#[derive(Debug, Parser)]
#[command(name = "mslwiki")]
#[command(about = "Monster Super League wiki extraction and serving tools")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Stage relevant Unity and IL2CPP files from an XAPK and index the import.
    Import {
        xapk: PathBuf,

        #[arg(long, default_value = "build/stage")]
        stage_dir: PathBuf,

        #[arg(long, default_value = "build/msl.sqlite")]
        database: PathBuf,
    },

    /// Run the local wiki/API server.
    Serve {
        #[arg(long, default_value = "build/msl.sqlite")]
        database: PathBuf,

        #[arg(long, default_value = "127.0.0.1:3000")]
        bind: SocketAddr,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mslwiki=info".into()),
        )
        .init();

    match Cli::parse().command {
        Command::Import {
            xapk,
            stage_dir,
            database,
        } => {
            let manifest = msl_ingest::stage_xapk(&xapk, &stage_dir)?;
            let db = Database::connect(&database).await?;
            let import_id = db.record_import(&manifest).await?;

            println!(
                "Imported {} {}",
                manifest.package.name, manifest.package.version_name
            );
            println!("Import ID: {import_id}");
            println!("Staged files: {}", manifest.file_count());
            println!(
                "Manifest: {}",
                stage_dir.join("import-manifest.json").display()
            );
        }
        Command::Serve { database, bind } => {
            let db = Database::connect(&database).await?;
            println!("MSL Wiki listening on http://{bind}");
            msl_web::serve(db, bind).await?;
        }
    }

    Ok(())
}
