use anyhow::{Context, Result};
use msl_core::{ImportManifest, StagedKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fs, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommonSpriteData {
    #[serde(rename = "SpriteList")]
    pub sprites: Vec<CommonSprite>,

    #[serde(rename = "SpriteIndicies", default)]
    pub sprite_indices: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommonSprite {
    pub name: String,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub border_left: u32,
    pub border_right: u32,
    pub border_top: u32,
    pub border_bottom: u32,
    pub padding_left: u32,
    pub padding_right: u32,
    pub padding_top: u32,
    pub padding_bottom: u32,
    pub has_border: bool,
    pub has_padding: bool,
}

pub fn load_common_sprite_data(path: &Path) -> Result<CommonSpriteData> {
    let bytes = fs::read(path)
        .with_context(|| format!("reading common sprite metadata {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing common sprite metadata {}", path.display()))
}

pub fn export_common_sprite_catalog(
    manifest: &ImportManifest,
    stage_root: &Path,
    output: &Path,
) -> Result<Option<usize>> {
    let Some(staged_file) = manifest
        .files
        .iter()
        .find(|file| file.kind == StagedKind::SpriteMetadata)
    else {
        return Ok(None);
    };

    let source = stage_root.join(&staged_file.staged_path);
    let catalog = load_common_sprite_data(&source)?;

    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }

    fs::write(output, serde_json::to_vec_pretty(&catalog)?)?;
    Ok(Some(catalog.sprites.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_sprite_shape() {
        let json = br#"{
            "SpriteList": [{
                "name": "arena_rank_01",
                "x": 1265,
                "y": 665,
                "width": 63,
                "height": 74,
                "borderLeft": 7,
                "borderRight": 7,
                "borderTop": 7,
                "borderBottom": 7,
                "paddingLeft": 15,
                "paddingRight": 14,
                "paddingTop": 13,
                "paddingBottom": 13,
                "hasBorder": true,
                "hasPadding": true
            }],
            "SpriteIndicies": {}
        }"#;

        let data: CommonSpriteData = serde_json::from_slice(json).unwrap();
        assert_eq!(data.sprites.len(), 1);
        assert_eq!(data.sprites[0].name, "arena_rank_01");
        assert_eq!(data.sprites[0].width, 63);
    }
}
