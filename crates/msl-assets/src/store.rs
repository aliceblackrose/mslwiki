use crate::write_json;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetStoreManifest {
    pub source_root: String,
    pub files: Vec<StoredAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAsset {
    pub source_path: String,
    pub content_hash: String,
    pub size: u64,
    pub stored_path: String,
    pub extension: Option<String>,
}

pub fn index_extracted_assets(
    extracted_root: &Path,
    store_root: &Path,
    manifest_path: &Path,
) -> Result<AssetStoreManifest> {
    let files = collect_files(extracted_root)?;
    fs::create_dir_all(store_root)?;

    let mut stored = Vec::with_capacity(files.len());
    for source in files {
        let (content_hash, size) = hash_file(&source)?;
        let extension = source
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase());
        let suffix = extension.as_deref().unwrap_or("bin");
        let stored_relative = format!("{}/{}.{}", &content_hash[..2], content_hash, suffix);
        let target = store_root.join(&stored_relative);

        if !target.exists() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&source, &target).with_context(|| {
                format!(
                    "copying extracted asset {} to {}",
                    source.display(),
                    target.display()
                )
            })?;
        }

        let source_path = source
            .strip_prefix(extracted_root)
            .unwrap_or(&source)
            .to_string_lossy()
            .replace('\\', "/");

        stored.push(StoredAsset {
            source_path,
            content_hash,
            size,
            stored_path: stored_relative,
            extension,
        });
    }

    stored.sort_by(|a, b| a.source_path.cmp(&b.source_path));
    let manifest = AssetStoreManifest {
        source_root: extracted_root.display().to_string(),
        files: stored,
    };
    write_json(manifest_path, &manifest)?;
    Ok(manifest)
}

fn collect_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();

    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .with_context(|| format!("reading extracted asset directory {}", directory.display()))?
        {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                files.push(entry.path());
            }
        }
    }

    files.sort();
    Ok(files)
}

fn hash_file(path: &Path) -> Result<(String, u64)> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("opening extracted asset {}", path.display()))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut size = 0_u64;

    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }

    Ok((hasher.finalize().to_hex().to_string(), size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deduplicates_by_content_hash() {
        let temp = tempfile::tempdir().unwrap();
        let extracted = temp.path().join("extracted");
        let store = temp.path().join("store");
        let manifest_path = temp.path().join("manifest.json");
        fs::create_dir_all(extracted.join("Texture2D")).unwrap();
        fs::create_dir_all(extracted.join("Sprite")).unwrap();
        fs::write(extracted.join("Texture2D/a.png"), b"same-content").unwrap();
        fs::write(extracted.join("Sprite/b.png"), b"same-content").unwrap();

        let manifest = index_extracted_assets(&extracted, &store, &manifest_path).unwrap();
        assert_eq!(manifest.files.len(), 2);
        assert_eq!(
            manifest.files[0].content_hash,
            manifest.files[1].content_hash
        );
        assert_eq!(manifest.files[0].stored_path, manifest.files[1].stored_path);
        assert!(store.join(&manifest.files[0].stored_path).is_file());
    }
}
