use crate::{
    serialized::{
        CLASS_ID_TEXT_ASSET, CLASS_ID_TEXTURE2D, ObjectReader, SerializedFile,
        looks_like_serialized_file,
    },
    texture::{
        StreamData, Texture2D, TextureData, decode_texture_rgba, parse_texture2d, save_png,
        texture_format_name,
    },
    write_json,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const MAX_STREAM_READ_BYTES: usize = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedAssetExportReport {
    pub files_scanned: usize,
    pub serialized_files: usize,
    pub objects_scanned: usize,
    pub texture_objects: usize,
    pub textures_exported: usize,
    pub text_asset_objects: usize,
    pub text_assets_exported: usize,
    pub textures: Vec<TextureExport>,
    pub text_assets: Vec<TextAssetExport>,
    pub issues: Vec<AssetExportIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextureExport {
    pub source: String,
    pub path_id: i64,
    pub name: String,
    pub width: usize,
    pub height: usize,
    pub mip_count: i32,
    pub texture_format: i32,
    pub texture_format_name: String,
    pub data_source: String,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextAssetExport {
    pub source: String,
    pub path_id: i64,
    pub name: String,
    pub size: usize,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetExportIssue {
    pub source: String,
    pub path_id: Option<i64>,
    pub class_id: Option<i32>,
    pub error: String,
}

pub fn export_serialized_assets(
    extraction_root: &Path,
    manifest_path: &Path,
) -> Result<SerializedAssetExportReport> {
    let inputs = collect_extracted_inputs(extraction_root)?;
    let resources = ResourceCatalog::new(extraction_root, &inputs);
    let objects_root = extraction_root.join("objects");
    fs::create_dir_all(objects_root.join("textures"))?;
    fs::create_dir_all(objects_root.join("text"))?;

    let mut report = SerializedAssetExportReport {
        files_scanned: inputs.len(),
        serialized_files: 0,
        objects_scanned: 0,
        texture_objects: 0,
        textures_exported: 0,
        text_asset_objects: 0,
        text_assets_exported: 0,
        textures: Vec::new(),
        text_assets: Vec::new(),
        issues: Vec::new(),
    };

    for source in &inputs {
        let bytes = fs::read(source)
            .with_context(|| format!("reading extracted Unity file {}", source.display()))?;
        if !looks_like_serialized_file(&bytes) {
            continue;
        }

        report.serialized_files += 1;
        let relative_source = relative_path(extraction_root, source);
        let file = match SerializedFile::parse(&bytes) {
            Ok(file) => file,
            Err(error) => {
                report.issues.push(AssetExportIssue {
                    source: relative_source,
                    path_id: None,
                    class_id: None,
                    error: format!("parsing SerializedFile: {error:#}"),
                });
                continue;
            }
        };

        report.objects_scanned += file.objects.len();
        for object in &file.objects {
            match object.class_id {
                CLASS_ID_TEXTURE2D => {
                    report.texture_objects += 1;
                    match export_texture(
                        extraction_root,
                        &objects_root,
                        source,
                        &relative_source,
                        &resources,
                        &file,
                        object,
                    ) {
                        Ok(texture) => {
                            report.textures_exported += 1;
                            report.textures.push(texture);
                        }
                        Err(error) => report.issues.push(AssetExportIssue {
                            source: relative_source.clone(),
                            path_id: Some(object.path_id),
                            class_id: Some(object.class_id),
                            error: format!("exporting Texture2D: {error:#}"),
                        }),
                    }
                }
                CLASS_ID_TEXT_ASSET => {
                    report.text_asset_objects += 1;
                    match export_text_asset(
                        extraction_root,
                        &objects_root,
                        &relative_source,
                        &file,
                        object,
                    ) {
                        Ok(text) => {
                            report.text_assets_exported += 1;
                            report.text_assets.push(text);
                        }
                        Err(error) => report.issues.push(AssetExportIssue {
                            source: relative_source.clone(),
                            path_id: Some(object.path_id),
                            class_id: Some(object.class_id),
                            error: format!("exporting TextAsset: {error:#}"),
                        }),
                    }
                }
                _ => {}
            }
        }
    }

    report
        .textures
        .sort_by(|left, right| (&left.name, &left.source, left.path_id).cmp(&(&right.name, &right.source, right.path_id)));
    report
        .text_assets
        .sort_by(|left, right| (&left.name, &left.source, left.path_id).cmp(&(&right.name, &right.source, right.path_id)));
    report.issues.sort_by(|left, right| {
        (&left.source, left.path_id, left.class_id, &left.error).cmp(&(
            &right.source,
            right.path_id,
            right.class_id,
            &right.error,
        ))
    });

    write_json(manifest_path, &report)?;
    Ok(report)
}

fn export_texture(
    extraction_root: &Path,
    objects_root: &Path,
    serialized_path: &Path,
    relative_source: &str,
    resources: &ResourceCatalog,
    file: &SerializedFile<'_>,
    object: &crate::serialized::ObjectInfo,
) -> Result<TextureExport> {
    let texture = parse_texture2d(file, object)?;
    let (encoded, data_source) = load_texture_data(serialized_path, resources, &texture)?;
    let rgba = decode_texture_rgba(&texture, &encoded)?;
    let output = objects_root.join("textures").join(unique_object_name(
        &texture.name,
        relative_source,
        object.path_id,
        "png",
    ));
    save_png(&output, &texture, rgba)?;

    Ok(TextureExport {
        source: relative_source.to_owned(),
        path_id: object.path_id,
        name: texture.name.clone(),
        width: texture.width,
        height: texture.height,
        mip_count: texture.mip_count,
        texture_format: texture.format,
        texture_format_name: texture_format_name(texture.format).to_owned(),
        data_source,
        output: relative_path(extraction_root, &output),
    })
}

fn load_texture_data(
    serialized_path: &Path,
    resources: &ResourceCatalog,
    texture: &Texture2D,
) -> Result<(Vec<u8>, String)> {
    match &texture.data {
        TextureData::Inline(bytes) => Ok((bytes.clone(), "inline".to_owned())),
        TextureData::Streamed(stream) => {
            ensure!(
                stream.size <= MAX_STREAM_READ_BYTES,
                "Texture2D stream is too large: {} bytes",
                stream.size
            );
            let resource = resources.resolve(serialized_path, stream)?;
            let bytes = read_resource_range(&resource, stream.offset, stream.size)?;
            Ok((bytes, resource.display().to_string()))
        }
    }
}

fn read_resource_range(path: &Path, offset: u64, size: usize) -> Result<Vec<u8>> {
    let mut file = File::open(path)
        .with_context(|| format!("opening streamed Unity resource {}", path.display()))?;
    let file_size = file.metadata()?.len();
    let end = offset
        .checked_add(u64::try_from(size).context("resource read size does not fit u64")?)
        .context("streamed Unity resource range overflow")?;
    ensure!(
        end <= file_size,
        "streamed Unity resource range {offset}..{end} exceeds {} bytes in {}",
        file_size,
        path.display()
    );
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0_u8; size];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn export_text_asset(
    extraction_root: &Path,
    objects_root: &Path,
    relative_source: &str,
    file: &SerializedFile<'_>,
    object: &crate::serialized::ObjectInfo,
) -> Result<TextAssetExport> {
    ensure!(
        file.target_platform != -2,
        "editor-only TextAsset layout is not supported"
    );
    let bytes = file.object_bytes(object)?;
    let mut reader = ObjectReader::new(bytes, file.endian);
    let name = reader.read_aligned_string()?;
    let data = reader.read_byte_array()?;
    let extension = if is_probably_text(&data) { "txt" } else { "bytes" };
    let output = objects_root.join("text").join(unique_object_name(
        &name,
        relative_source,
        object.path_id,
        extension,
    ));
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, &data)
        .with_context(|| format!("writing TextAsset {}", output.display()))?;

    Ok(TextAssetExport {
        source: relative_source.to_owned(),
        path_id: object.path_id,
        name,
        size: data.len(),
        output: relative_path(extraction_root, &output),
    })
}

fn is_probably_text(bytes: &[u8]) -> bool {
    if bytes.contains(&0) {
        return false;
    }
    std::str::from_utf8(bytes).is_ok()
}

fn unique_object_name(name: &str, source: &str, path_id: i64, extension: &str) -> String {
    let stem = sanitize_name(name, "unnamed");
    let source_hash = blake3::hash(source.as_bytes()).to_hex().to_string();
    format!("{stem}~{}~{path_id}.{extension}", &source_hash[..10])
}

fn sanitize_name(value: &str, fallback: &str) -> String {
    let mut safe = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    safe = safe.trim_matches('.').to_owned();
    if safe.is_empty() {
        safe = fallback.to_owned();
    }
    safe.truncate(120);
    safe
}

#[derive(Debug)]
struct ResourceCatalog {
    root: PathBuf,
    by_name: BTreeMap<String, Vec<PathBuf>>,
}

impl ResourceCatalog {
    fn new(root: &Path, files: &[PathBuf]) -> Self {
        let mut by_name = BTreeMap::<String, Vec<PathBuf>>::new();
        for path in files {
            if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                by_name.entry(name.to_owned()).or_default().push(path.clone());
            }
        }
        for paths in by_name.values_mut() {
            paths.sort();
        }
        Self {
            root: root.to_path_buf(),
            by_name,
        }
    }

    fn resolve(&self, serialized_path: &Path, stream: &StreamData) -> Result<PathBuf> {
        ensure!(!stream.path.is_empty(), "Texture2D stream path is empty");
        let normalized = stream.path.replace('\\', "/");
        let file_name = normalized
            .rsplit('/')
            .find(|part| !part.is_empty())
            .context("Texture2D stream path has no file name")?;

        if let Some(parent) = serialized_path.parent() {
            let sibling = parent.join(file_name);
            if sibling.is_file() {
                return Ok(sibling);
            }
        }

        if let Some(paths) = self.by_name.get(file_name) {
            if paths.len() == 1 {
                return Ok(paths[0].clone());
            }
            if let Some(parent) = serialized_path.parent()
                && let Some(found) = paths.iter().find(|candidate| candidate.parent() == Some(parent))
            {
                return Ok(found.clone());
            }
            if !paths.is_empty() {
                anyhow::bail!(
                    "Texture2D stream resource {file_name} is ambiguous across {} extracted files",
                    paths.len()
                );
            }
        }

        let case_insensitive = self
            .by_name
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case(file_name))
            .flat_map(|(_, paths)| paths.iter())
            .collect::<Vec<_>>();
        if case_insensitive.len() == 1 {
            return Ok(case_insensitive[0].clone());
        }

        anyhow::bail!(
            "Texture2D stream resource {file_name} was not found under {}",
            self.root.display()
        )
    }
}

fn collect_extracted_inputs(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for directory in [root.join("bundles"), root.join("raw")] {
        if !directory.is_dir() {
            continue;
        }
        let mut pending = vec![directory];
        while let Some(current) = pending.pop() {
            for entry in fs::read_dir(&current)
                .with_context(|| format!("reading extracted Unity directory {}", current.display()))?
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
    }
    files.sort();
    Ok(files)
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_stream_resource_next_to_cab() {
        let temp = tempfile::tempdir().unwrap();
        let bundle = temp.path().join("bundles/source");
        fs::create_dir_all(&bundle).unwrap();
        let cab = bundle.join("CAB-test");
        let resource = bundle.join("CAB-test.resS");
        fs::write(&cab, b"cab").unwrap();
        fs::write(&resource, b"resource").unwrap();
        let catalog = ResourceCatalog::new(temp.path(), &[cab.clone(), resource.clone()]);
        let stream = StreamData {
            offset: 0,
            size: 8,
            path: "archive:/CAB-test/CAB-test.resS".to_owned(),
        };
        assert_eq!(catalog.resolve(&cab, &stream).unwrap(), resource);
    }

    #[test]
    fn reads_bounded_stream_range() {
        let temp = tempfile::tempdir().unwrap();
        let resource = temp.path().join("data.resS");
        fs::write(&resource, b"0123456789").unwrap();
        assert_eq!(read_resource_range(&resource, 3, 4).unwrap(), b"3456");
        assert!(read_resource_range(&resource, 9, 4).is_err());
    }

    #[test]
    fn object_names_are_collision_resistant() {
        let first = unique_object_name("_common", "bundles/a/CAB", 7, "png");
        let second = unique_object_name("_common", "bundles/b/CAB", 7, "png");
        assert_ne!(first, second);
        assert!(first.starts_with("_common~"));
    }
}
