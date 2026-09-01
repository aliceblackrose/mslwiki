use crate::{CommonSpriteData, write_json};
use aho_corasick::AhoCorasick;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

const COMMON_ATLAS_MIN_BYTES: u64 = 1_000_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnityAssetEvidence {
    pub path: String,
    pub size: u64,
    pub common_marker: bool,
    pub main_tex_marker: bool,
    pub matched_sprites: Vec<String>,
    pub referenced_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetDiscoveryReport {
    pub sprite_count: usize,
    pub files_scanned: usize,
    pub evidence: Vec<UnityAssetEvidence>,
    pub large_common_candidates: Vec<String>,
}

pub fn export_asset_discovery_report(
    sprite_catalog: &Path,
    unity_data_dir: &Path,
    output: &Path,
) -> Result<AssetDiscoveryReport> {
    let catalog = crate::load_common_sprite_data(sprite_catalog)?;
    let report = discover_asset_evidence(&catalog, unity_data_dir)?;
    write_json(output, &report)?;
    Ok(report)
}

pub fn discover_asset_evidence(
    catalog: &CommonSpriteData,
    unity_data_dir: &Path,
) -> Result<AssetDiscoveryReport> {
    let files = collect_files(unity_data_dir)?;
    let known_names = files
        .iter()
        .filter_map(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect::<BTreeSet<_>>();

    let sprite_names = catalog
        .sprites
        .iter()
        .map(|sprite| sprite.name.clone())
        .collect::<Vec<_>>();
    let sprite_search = build_search(&sprite_names, "sprite-name")?;

    let reference_names = known_names.iter().cloned().collect::<Vec<_>>();
    let reference_search = build_search(&reference_names, "Unity-file reference")?;

    let mut evidence = Vec::new();

    for path in &files {
        let bytes = fs::read(path)
            .with_context(|| format!("reading Unity data file {}", path.display()))?;
        let current_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();

        let matched_sprites = find_patterns(sprite_search.as_ref(), &sprite_names, &bytes, None);
        let referenced_files = find_patterns(
            reference_search.as_ref(),
            &reference_names,
            &bytes,
            Some(&current_name),
        );
        let common_marker = contains_bytes(&bytes, b"_common");
        let main_tex_marker = contains_bytes(&bytes, b"_MainTex");

        if matched_sprites.is_empty()
            && referenced_files.is_empty()
            && !common_marker
            && !main_tex_marker
        {
            continue;
        }

        let relative = path
            .strip_prefix(unity_data_dir)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");

        evidence.push(UnityAssetEvidence {
            path: relative,
            size: bytes.len() as u64,
            common_marker,
            main_tex_marker,
            matched_sprites,
            referenced_files,
        });
    }

    evidence.sort_by(|a, b| {
        b.matched_sprites
            .len()
            .cmp(&a.matched_sprites.len())
            .then_with(|| b.size.cmp(&a.size))
            .then_with(|| a.path.cmp(&b.path))
    });

    let main_tex_targets = evidence
        .iter()
        .filter(|item| item.main_tex_marker)
        .flat_map(|item| item.referenced_files.iter().cloned())
        .collect::<BTreeSet<_>>();

    let large_common_candidates = evidence
        .iter()
        .filter(|item| {
            let file_name = Path::new(&item.path)
                .file_name()
                .map(|name| name.to_string_lossy());
            item.common_marker
                && item.matched_sprites.is_empty()
                && item.size >= COMMON_ATLAS_MIN_BYTES
                && file_name
                    .as_deref()
                    .is_some_and(|name| main_tex_targets.contains(name))
        })
        .map(|item| item.path.clone())
        .collect();

    Ok(AssetDiscoveryReport {
        sprite_count: catalog.sprites.len(),
        files_scanned: files.len(),
        evidence,
        large_common_candidates,
    })
}

fn build_search(patterns: &[String], label: &str) -> Result<Option<AhoCorasick>> {
    if patterns.is_empty() {
        return Ok(None);
    }

    let search = AhoCorasick::new(patterns.iter().map(|pattern| pattern.as_bytes()))
        .with_context(|| format!("building {label} search automaton"))?;
    Ok(Some(search))
}

fn find_patterns(
    search: Option<&AhoCorasick>,
    patterns: &[String],
    bytes: &[u8],
    excluded: Option<&str>,
) -> Vec<String> {
    let Some(search) = search else {
        return Vec::new();
    };

    let mut ids = BTreeSet::new();
    for found in search.find_overlapping_iter(bytes) {
        let id = found.pattern().as_usize();
        if excluded != Some(patterns[id].as_str()) {
            ids.insert(id);
        }
    }

    ids.into_iter().map(|id| patterns[id].clone()).collect()
}

fn collect_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();

    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .with_context(|| format!("reading Unity directory {}", directory.display()))?
        {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() && should_scan_file(root, &entry.path()) {
                files.push(entry.path());
            }
        }
    }

    files.sort();
    Ok(files)
}

fn should_scan_file(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return true;
    };

    relative
        .components()
        .next()
        .is_none_or(|component| component.as_os_str() != "Managed")
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CommonSprite;
    use serde_json::json;

    fn sprite(name: &str) -> CommonSprite {
        CommonSprite {
            name: name.to_owned(),
            x: 0,
            y: 0,
            width: 32,
            height: 32,
            border_left: 0,
            border_right: 0,
            border_top: 0,
            border_bottom: 0,
            padding_left: 0,
            padding_right: 0,
            padding_top: 0,
            padding_bottom: 0,
            has_border: false,
            has_padding: false,
        }
    }

    #[test]
    fn discovers_sprite_and_texture_candidates() {
        let temp = tempfile::tempdir().unwrap();
        let metadata_path = temp.path().join("metadata");
        let link_path = temp.path().join("link");
        let texture_path = temp.path().join("textureblob");

        fs::write(
            &metadata_path,
            b"6000.3.12f1\0_common\0arena_rank_01\0link\0",
        )
        .unwrap();
        fs::write(&link_path, b"_common\0_MainTex\0textureblob\0").unwrap();

        let mut texture = vec![0_u8; (COMMON_ATLAS_MIN_BYTES + 1) as usize];
        texture[..7].copy_from_slice(b"_common");
        fs::write(&texture_path, texture).unwrap();

        let catalog = CommonSpriteData {
            sprites: vec![sprite("arena_rank_01")],
            sprite_indices: json!({}),
        };

        let report = discover_asset_evidence(&catalog, temp.path()).unwrap();
        assert_eq!(report.sprite_count, 1);
        assert_eq!(report.files_scanned, 3);
        assert_eq!(report.large_common_candidates, vec!["textureblob"]);

        let metadata = report
            .evidence
            .iter()
            .find(|item| item.path == "metadata")
            .unwrap();
        assert_eq!(metadata.matched_sprites, vec!["arena_rank_01"]);
        assert!(metadata.referenced_files.contains(&"link".to_owned()));
    }
}
