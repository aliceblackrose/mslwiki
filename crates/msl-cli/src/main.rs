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
    /// Stage relevant game files, build the Unity data view, and index the import.
    Import {
        xapk: PathBuf,

        #[arg(long, default_value = "build/stage")]
        stage_dir: PathBuf,

        #[arg(long, default_value = "build/unity/Data")]
        unity_dir: PathBuf,

        #[arg(long, default_value = "build/assets/common-sprites.json")]
        sprite_catalog: PathBuf,

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
            unity_dir,
            sprite_catalog,
            database,
        } => {
            let manifest = msl_ingest::stage_xapk(&xapk, &stage_dir)?;
            let unity_files = msl_ingest::merge_unity_data(&manifest, &stage_dir, &unity_dir)?;
            let sprite_count =
                msl_assets::export_common_sprite_catalog(&manifest, &stage_dir, &sprite_catalog)?;

            let db = Database::connect(&database).await?;
            let import_id = db.record_import(&manifest).await?;

            println!(
                "Imported {} {}",
                manifest.package.name, manifest.package.version_name
            );
            println!("Import ID: {import_id}");
            println!("Staged files: {}", manifest.file_count());
            println!("Merged Unity files: {unity_files}");
            match sprite_count {
                Some(count) => println!("Common sprites: {count}"),
                None => println!("Common sprites: metadata not present"),
            }
            println!(
                "Manifest: {}",
                stage_dir.join("import-manifest.json").display()
            );
            println!("Unity Data: {}", unity_dir.display());
        }
        Command::Serve { database, bind } => {
            let db = Database::connect(&database).await?;
            println!("MSL Wiki listening on http://{bind}");
            msl_web::serve(db, bind).await?;
        }
    }

    Ok(())
}
