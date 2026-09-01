use anyhow::{bail, Context, Result};
use std::{fs, path::Path, process::Command};

pub fn run_asset_ripper_cli(executable: &Path, unity_data_dir: &Path, output: &Path) -> Result<()> {
    if !unity_data_dir.is_dir() {
        bail!(
            "Unity data directory does not exist: {}",
            unity_data_dir.display()
        );
    }

    if output.exists() && fs::read_dir(output)?.next().transpose()?.is_some() {
        bail!(
            "asset extraction output is not empty: {}",
            output.display()
        );
    }
    fs::create_dir_all(output)?;

    let status = Command::new(executable)
        .arg("--cli")
        .arg("--input")
        .arg(unity_data_dir)
        .arg("--output")
        .arg(output)
        .arg("--mode")
        .arg("primary")
        .arg("--disable-script-import")
        .arg("true")
        .arg("--ignore-streaming-assets")
        .arg("true")
        .status()
        .with_context(|| format!("starting AssetRipper CLI {}", executable.display()))?;

    if !status.success() {
        bail!("AssetRipper CLI exited with status {status}");
    }

    Ok(())
}
