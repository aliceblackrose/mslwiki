use anyhow::{Context, Result, bail};
use msl_core::{GamePackageMetadata, ImportManifest, StagedFile, StagedKind};
use serde_json::Value;
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
};
use zip::ZipArchive;

pub fn stage_xapk(input: &Path, output: &Path) -> Result<ImportManifest> {
    fs::create_dir_all(output)
        .with_context(|| format!("creating staging directory {}", output.display()))?;

    let source_hash = hash_file(input)?;
    let source_file =
        File::open(input).with_context(|| format!("opening XAPK {}", input.display()))?;
    let mut xapk = ZipArchive::new(source_file).context("opening XAPK as ZIP archive")?;

    let package = {
        let mut manifest_file = xapk
            .by_name("manifest.json")
            .context("XAPK does not contain manifest.json")?;
        let mut bytes = Vec::new();
        manifest_file.read_to_end(&mut bytes)?;

        let source_dir = output.join("source");
        fs::create_dir_all(&source_dir)?;
        fs::write(source_dir.join("manifest.json"), &bytes)?;

        parse_package_metadata(&bytes)?
    };

    let mut files = Vec::new();

    for index in 0..xapk.len() {
        let mut entry = xapk.by_index(index)?;
        let entry_name = entry.name().to_owned();

        if !entry_name.ends_with(".apk") {
            continue;
        }

        // Nested APKs can be hundreds of MiB. Spool one split at a time to a
        // seekable temporary file rather than retaining the whole APK in RAM.
        let mut temporary_apk = tempfile::tempfile()?;
        std::io::copy(&mut entry, &mut temporary_apk)
            .with_context(|| format!("reading split APK {entry_name}"))?;
        temporary_apk.seek(SeekFrom::Start(0))?;

        stage_apk(&entry_name, temporary_apk, output, &mut files)
            .with_context(|| format!("staging split APK {entry_name}"))?;
    }

    files.sort_by(|a, b| (&a.source_apk, &a.archive_path).cmp(&(&b.source_apk, &b.archive_path)));

    let manifest = ImportManifest {
        source_path: input.display().to_string(),
        source_hash,
        package,
        files,
    };

    fs::write(
        output.join("import-manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;

    Ok(manifest)
}

fn stage_apk<R: Read + Seek>(
    source_apk: &str,
    reader: R,
    output: &Path,
    staged_files: &mut Vec<StagedFile>,
) -> Result<()> {
    let mut apk = ZipArchive::new(reader)?;
    let source_file_name = Path::new(source_apk)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("split.apk");
    let split_name = source_file_name
        .strip_suffix(".apk")
        .unwrap_or(source_file_name);

    for index in 0..apk.len() {
        let mut entry = apk.by_index(index)?;
        let archive_path = entry.name().to_owned();
        let Some(kind) = classify_path(&archive_path) else {
            continue;
        };

        let safe_path = safe_archive_path(&archive_path)?;
        let target = output.join("raw").join(split_name).join(safe_path);

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut out = File::create(&target)?;
        let mut hasher = blake3::Hasher::new();
        let mut buffer = [0_u8; 64 * 1024];
        let mut written = 0_u64;

        loop {
            let count = entry.read(&mut buffer)?;
            if count == 0 {
                break;
            }

            hasher.update(&buffer[..count]);
            out.write_all(&buffer[..count])?;
            written += count as u64;
        }

        let staged_path = target
            .strip_prefix(output)
            .unwrap_or(&target)
            .to_string_lossy()
            .replace('\\', "/");

        staged_files.push(StagedFile {
            source_apk: source_apk.to_owned(),
            archive_path,
            staged_path,
            content_hash: hasher.finalize().to_hex().to_string(),
            size: written,
            kind,
        });
    }

    Ok(())
}

fn classify_path(path: &str) -> Option<StagedKind> {
    match path {
        "lib/arm64-v8a/libil2cpp.so" => Some(StagedKind::Il2CppBinary),
        "assets/bin/Data/Managed/Metadata/global-metadata.dat" => Some(StagedKind::Il2CppMetadata),
        "assets/CommonSpriteData.txt" => Some(StagedKind::SpriteMetadata),
        "assets/data_list.txt" => Some(StagedKind::DataList),
        "assets/client_gamedata.pb" | "assets/server_gamedata.pb" => Some(StagedKind::GameData),
        _ if path.starts_with("assets/bin/Data/") => Some(StagedKind::UnityData),
        _ => None,
    }
}

fn safe_archive_path(path: &str) -> Result<PathBuf> {
    let path = Path::new(path);

    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("unsafe archive path: {}", path.display());
    }

    Ok(path.to_path_buf())
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }

    Ok(hasher.finalize().to_hex().to_string())
}

fn parse_package_metadata(bytes: &[u8]) -> Result<GamePackageMetadata> {
    let value: Value = serde_json::from_slice(bytes)?;

    Ok(GamePackageMetadata {
        xapk_version: json_string(value.get("xapk_version")),
        package_name: required_json_string(&value, "package_name")?,
        name: required_json_string(&value, "name")?,
        version_code: required_json_string(&value, "version_code")?,
        version_name: required_json_string(&value, "version_name")?,
    })
}

fn required_json_string(value: &Value, field: &str) -> Result<String> {
    json_string(value.get(field)).with_context(|| format!("manifest.json is missing {field}"))
}

fn json_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_known_msl_paths() {
        assert_eq!(
            classify_path("lib/arm64-v8a/libil2cpp.so"),
            Some(StagedKind::Il2CppBinary)
        );
        assert_eq!(
            classify_path("assets/bin/Data/Managed/Metadata/global-metadata.dat"),
            Some(StagedKind::Il2CppMetadata)
        );
        assert_eq!(
            classify_path("assets/CommonSpriteData.txt"),
            Some(StagedKind::SpriteMetadata)
        );
        assert_eq!(
            classify_path("assets/bin/Data/sharedassets0.assets"),
            Some(StagedKind::UnityData)
        );
        assert_eq!(classify_path("classes.dex"), None);
    }
}
