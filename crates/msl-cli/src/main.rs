use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use msl_db::Database;
use std::{net::SocketAddr, path::PathBuf};

#[derive(Debug, Parser)]
#[command(name = "mslwiki")]
#[command(about = "Monster Super League wiki extraction and serving tools")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliSpriteOrigin {
    TopLeft,
    BottomLeft,
}

impl From<CliSpriteOrigin> for msl_assets::SpriteOrigin {
    fn from(value: CliSpriteOrigin) -> Self {
        match value {
            CliSpriteOrigin::TopLeft => Self::TopLeft,
            CliSpriteOrigin::BottomLeft => Self::BottomLeft,
        }
    }
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
        #[arg(long, default_value = "build/assets/asset-candidates.json")]
        asset_report: PathBuf,
        #[arg(long, default_value = "build/gamedata/metadata-inventory.json")]
        metadata_inventory: PathBuf,
        #[arg(long, default_value = "build/msl.sqlite")]
        database: PathBuf,
    },

    /// Re-scan merged Unity data and identify likely sprite/texture dependencies.
    InspectAssets {
        #[arg(long, default_value = "build/unity/Data")]
        unity_dir: PathBuf,
        #[arg(long, default_value = "build/assets/common-sprites.json")]
        sprite_catalog: PathBuf,
        #[arg(long, default_value = "build/assets/asset-candidates.json")]
        output: PathBuf,
    },

    /// Inventory protobuf-like types and recover field-number constants from global-metadata.dat.
    InspectMetadata {
        #[arg(
            long,
            default_value = "build/unity/Data/Managed/Metadata/global-metadata.dat"
        )]
        metadata: PathBuf,
        #[arg(long, default_value = "build/gamedata/metadata-inventory.json")]
        output: PathBuf,
    },

    /// Print one recovered protobuf message schema and its exact field tags.
    ShowSchema {
        message: String,
        #[arg(
            long,
            default_value = "build/unity/Data/Managed/Metadata/global-metadata.dat"
        )]
        metadata: PathBuf,
    },

    /// Extract Unity containers and decoded primary objects with the built-in Rust backend.
    ExtractAssets {
        #[arg(long, default_value = "build/unity/Data")]
        unity_dir: PathBuf,
        #[arg(long, default_value = "build/extracted")]
        output: PathBuf,
        #[arg(long, default_value = "build/assets/unity-objects.json")]
        objects_manifest: PathBuf,
        #[arg(long, default_value = "build/assets/store")]
        store: PathBuf,
        #[arg(long, default_value = "build/assets/extracted-manifest.json")]
        manifest: PathBuf,
    },

    /// Index an existing extracted asset directory without running extraction.
    IndexAssets {
        #[arg(long, default_value = "build/extracted")]
        input: PathBuf,
        #[arg(long, default_value = "build/assets/store")]
        store: PathBuf,
        #[arg(long, default_value = "build/assets/extracted-manifest.json")]
        manifest: PathBuf,
    },

    /// Crop every CommonSpriteData region from an exported atlas PNG and index the sprites.
    CropSprites {
        #[arg(long)]
        atlas: PathBuf,
        #[arg(long, default_value = "build/assets/common-sprites.json")]
        sprite_catalog: PathBuf,
        #[arg(long, default_value = "build/assets/sprites")]
        output: PathBuf,
        #[arg(long, default_value = "build/assets/sprites-manifest.json")]
        manifest: PathBuf,
        #[arg(long, value_enum, default_value_t = CliSpriteOrigin::TopLeft)]
        origin: CliSpriteOrigin,
        #[arg(long, default_value = "build/assets/store")]
        store: PathBuf,
        #[arg(long, default_value = "build/assets/sprites-store-manifest.json")]
        store_manifest: PathBuf,
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
            asset_report,
            metadata_inventory,
            database,
        } => {
            let manifest = msl_ingest::stage_xapk(&xapk, &stage_dir)?;
            let unity_files = msl_ingest::merge_unity_data(&manifest, &stage_dir, &unity_dir)?;
            let sprite_count =
                msl_assets::export_common_sprite_catalog(&manifest, &stage_dir, &sprite_catalog)?;
            let discovery = match sprite_count {
                Some(_) => Some(msl_assets::export_asset_discovery_report(
                    &sprite_catalog,
                    &unity_dir,
                    &asset_report,
                )?),
                None => None,
            };

            let global_metadata = unity_dir.join("Managed/Metadata/global-metadata.dat");
            let metadata = if global_metadata.is_file() {
                let inventory = msl_gamedata::inspect_global_metadata(&global_metadata)?;
                msl_gamedata::write_metadata_inventory(&metadata_inventory, &inventory)?;
                Some(inventory)
            } else {
                None
            };

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
            if let Some(discovery) = discovery {
                println!("Asset evidence files: {}", discovery.evidence.len());
                println!(
                    "Large _common candidates: {}",
                    discovery.large_common_candidates.len()
                );
                println!("Asset report: {}", asset_report.display());
            }
            if let Some(metadata) = metadata {
                let recovered_fields = metadata
                    .protobuf_messages
                    .iter()
                    .map(|message| message.fields.len())
                    .sum::<usize>();
                println!("Metadata version: {:?}", metadata.metadata_version);
                println!("Message-like strings: {}", metadata.message_types.len());
                println!(
                    "Recovered protobuf messages: {}",
                    metadata.protobuf_messages.len()
                );
                println!("Recovered protobuf field tags: {recovered_fields}");
                println!("Manager types: {}", metadata.manager_types.len());
                println!("Metadata inventory: {}", metadata_inventory.display());
            }
            println!(
                "Manifest: {}",
                stage_dir.join("import-manifest.json").display()
            );
            println!("Unity Data: {}", unity_dir.display());
        }
        Command::InspectAssets {
            unity_dir,
            sprite_catalog,
            output,
        } => {
            let report =
                msl_assets::export_asset_discovery_report(&sprite_catalog, &unity_dir, &output)?;
            println!("Unity files scanned: {}", report.files_scanned);
            println!("Sprite catalog entries: {}", report.sprite_count);
            println!("Evidence files: {}", report.evidence.len());
            println!(
                "Large _common candidates: {}",
                report.large_common_candidates.len()
            );
            for candidate in &report.large_common_candidates {
                println!("  {candidate}");
            }
            println!("Report: {}", output.display());
        }
        Command::InspectMetadata { metadata, output } => {
            let inventory = msl_gamedata::inspect_global_metadata(&metadata)?;
            msl_gamedata::write_metadata_inventory(&output, &inventory)?;
            let recovered_fields = inventory
                .protobuf_messages
                .iter()
                .map(|message| message.fields.len())
                .sum::<usize>();
            println!("Metadata version: {:?}", inventory.metadata_version);
            println!("Message-like strings: {}", inventory.message_types.len());
            println!(
                "Recovered protobuf messages: {}",
                inventory.protobuf_messages.len()
            );
            println!("Recovered protobuf field tags: {recovered_fields}");
            println!("Manager types: {}", inventory.manager_types.len());
            println!(
                "Interesting strings: {}",
                inventory.interesting_strings.len()
            );
            println!("Metadata inventory: {}", output.display());
        }
        Command::ShowSchema { message, metadata } => {
            let inventory = msl_gamedata::inspect_global_metadata(&metadata)?;
            let requested = if message.starts_with("Msg") {
                message
            } else {
                format!("Msg{message}")
            };
            let schema = inventory
                .protobuf_messages
                .iter()
                .find(|schema| schema.name.eq_ignore_ascii_case(&requested))
                .with_context(|| format!("protobuf message {requested} was not recovered"))?;

            println!(
                "{} (type #{}, {} protobuf fields)",
                schema.name,
                schema.type_definition_index,
                schema.fields.len()
            );
            for field in &schema.fields {
                match (&field.backing_field, field.backing_type_index) {
                    (Some(backing), Some(type_index)) => {
                        println!(
                            "{:4} {:<8} {:<48} backing={} typeIndex={}",
                            field.tag, field.cardinality, field.name, backing, type_index
                        );
                    }
                    _ => println!("{:4} {:<8} {}", field.tag, field.cardinality, field.name),
                }
            }
        }
        Command::ExtractAssets {
            unity_dir,
            output,
            objects_manifest,
            store,
            manifest,
        } => {
            let extraction = msl_assets::extract_unity_content(&unity_dir, &output)?;
            let objects = msl_assets::export_serialized_assets(&output, &objects_manifest)?;
            let indexed = msl_assets::index_extracted_assets(&output, &store, &manifest)?;
            println!("Unity files scanned: {}", extraction.files_scanned);
            println!(
                "UnityFS bundles extracted: {}",
                extraction.bundles_extracted
            );
            println!("UnityFS entries extracted: {}", extraction.bundle_entries);
            println!(
                "Standalone Unity files preserved: {}",
                extraction.passthrough_files
            );
            println!("Extracted bytes: {}", extraction.extracted_bytes);
            println!("Serialized files: {}", objects.serialized_files);
            println!("Serialized objects scanned: {}", objects.objects_scanned);
            println!("Texture2D objects: {}", objects.texture_objects);
            println!("Texture2D PNGs exported: {}", objects.textures_exported);
            println!("TextAsset objects: {}", objects.text_asset_objects);
            println!("TextAssets exported: {}", objects.text_assets_exported);
            println!("Object export issues: {}", objects.issues.len());
            println!("Object manifest: {}", objects_manifest.display());
            println!("Extracted content: {}", output.display());
            println!("Indexed assets: {}", indexed.files.len());
            println!("Asset store: {}", store.display());
            println!("Asset manifest: {}", manifest.display());
        }
        Command::IndexAssets {
            input,
            store,
            manifest,
        } => {
            let indexed = msl_assets::index_extracted_assets(&input, &store, &manifest)?;
            println!("Indexed assets: {}", indexed.files.len());
            println!("Asset store: {}", store.display());
            println!("Asset manifest: {}", manifest.display());
        }
        Command::CropSprites {
            atlas,
            sprite_catalog,
            output,
            manifest,
            origin,
            store,
            store_manifest,
        } => {
            let (sprites, indexed) = msl_assets::crop_and_index_common_sprites(
                &atlas,
                &sprite_catalog,
                &output,
                &manifest,
                origin.into(),
                &store,
                &store_manifest,
            )?;
            println!("Atlas: {}x{}", sprites.atlas_width, sprites.atlas_height);
            println!("Cropped sprites: {}", sprites.sprites.len());
            println!("Indexed sprite files: {}", indexed.files.len());
            println!("Sprite output: {}", output.display());
            println!("Sprite manifest: {}", manifest.display());
            println!("Asset store: {}", store.display());
        }
        Command::Serve { database, bind } => {
            let db = Database::connect(&database).await?;
            println!("MSL Wiki listening on http://{bind}");
            msl_web::serve(db, bind).await?;
        }
    }

    Ok(())
}
