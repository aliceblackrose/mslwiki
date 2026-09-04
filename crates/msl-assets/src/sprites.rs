use crate::{CommonSpriteData, index_extracted_assets, load_common_sprite_data, write_json};
use anyhow::{Context, Result, bail};
use image::GenericImageView;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs, path::Path};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpriteOrigin {
    TopLeft,
    BottomLeft,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpriteExportManifest {
    pub atlas: String,
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub origin: SpriteOrigin,
    pub sprites: Vec<ExportedSprite>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedSprite {
    pub name: String,
    pub source_x: u32,
    pub source_y: u32,
    pub source_width: u32,
    pub source_height: u32,
    pub output_path: String,
}

pub fn crop_common_sprites(
    atlas_path: &Path,
    sprite_catalog_path: &Path,
    output_dir: &Path,
    manifest_path: &Path,
    origin: SpriteOrigin,
) -> Result<SpriteExportManifest> {
    let catalog = load_common_sprite_data(sprite_catalog_path)?;
    crop_catalog_sprites(atlas_path, &catalog, output_dir, manifest_path, origin)
}

pub fn crop_catalog_sprites(
    atlas_path: &Path,
    catalog: &CommonSpriteData,
    output_dir: &Path,
    manifest_path: &Path,
    origin: SpriteOrigin,
) -> Result<SpriteExportManifest> {
    let atlas = image::open(atlas_path)
        .with_context(|| format!("opening sprite atlas {}", atlas_path.display()))?;
    let (atlas_width, atlas_height) = atlas.dimensions();

    if output_dir.exists() {
        fs::remove_dir_all(output_dir)
            .with_context(|| format!("clearing sprite output {}", output_dir.display()))?;
    }
    fs::create_dir_all(output_dir)
        .with_context(|| format!("creating sprite output {}", output_dir.display()))?;

    let mut exported = Vec::with_capacity(catalog.sprites.len());
    let mut output_names = BTreeSet::new();
    for sprite in &catalog.sprites {
        validate_rect(
            sprite.name.as_str(),
            sprite.x,
            sprite.y,
            sprite.width,
            sprite.height,
            atlas_width,
            atlas_height,
        )?;

        let crop_y = match origin {
            SpriteOrigin::TopLeft => sprite.y,
            SpriteOrigin::BottomLeft => atlas_height
                .checked_sub(sprite.y)
                .and_then(|value| value.checked_sub(sprite.height))
                .with_context(|| {
                    format!(
                        "sprite {} has invalid bottom-left y coordinate",
                        sprite.name
                    )
                })?,
        };

        validate_rect(
            sprite.name.as_str(),
            sprite.x,
            crop_y,
            sprite.width,
            sprite.height,
            atlas_width,
            atlas_height,
        )?;

        let safe_name = sanitize_filename(&sprite.name);
        let output_name = format!("{safe_name}.png");
        if !output_names.insert(output_name.clone()) {
            bail!(
                "sprite output filename collision after sanitizing {}: {}",
                sprite.name,
                output_name
            );
        }
        let output_path = output_dir.join(&output_name);

        atlas
            .crop_imm(sprite.x, crop_y, sprite.width, sprite.height)
            .save(&output_path)
            .with_context(|| {
                format!("saving sprite {} to {}", sprite.name, output_path.display())
            })?;

        exported.push(ExportedSprite {
            name: sprite.name.clone(),
            source_x: sprite.x,
            source_y: sprite.y,
            source_width: sprite.width,
            source_height: sprite.height,
            output_path: output_name,
        });
    }

    let manifest = SpriteExportManifest {
        atlas: atlas_path.display().to_string(),
        atlas_width,
        atlas_height,
        origin,
        sprites: exported,
    };
    write_json(manifest_path, &manifest)?;
    Ok(manifest)
}

pub fn crop_and_index_common_sprites(
    atlas_path: &Path,
    sprite_catalog_path: &Path,
    output_dir: &Path,
    sprite_manifest_path: &Path,
    origin: SpriteOrigin,
    store_root: &Path,
    store_manifest_path: &Path,
) -> Result<(SpriteExportManifest, crate::AssetStoreManifest)> {
    let sprite_manifest = crop_common_sprites(
        atlas_path,
        sprite_catalog_path,
        output_dir,
        sprite_manifest_path,
        origin,
    )?;
    let store_manifest = index_extracted_assets(output_dir, store_root, store_manifest_path)?;
    Ok((sprite_manifest, store_manifest))
}

fn validate_rect(
    name: &str,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    atlas_width: u32,
    atlas_height: u32,
) -> Result<()> {
    if width == 0 || height == 0 {
        bail!("sprite {name} has zero-sized rectangle {width}x{height}");
    }

    let right = x
        .checked_add(width)
        .with_context(|| format!("sprite {name} x coordinate overflow"))?;
    let bottom = y
        .checked_add(height)
        .with_context(|| format!("sprite {name} y coordinate overflow"))?;

    if right > atlas_width || bottom > atlas_height {
        bail!(
            "sprite {name} rectangle ({x},{y},{width},{height}) exceeds atlas {atlas_width}x{atlas_height}"
        );
    }

    Ok(())
}

fn sanitize_filename(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            result.push(ch);
        } else {
            result.push('_');
        }
    }

    if result.is_empty() {
        "sprite".to_owned()
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CommonSprite;
    use image::{ImageBuffer, Rgba};
    use serde_json::json;

    fn sprite(name: &str, x: u32, y: u32, width: u32, height: u32) -> CommonSprite {
        CommonSprite {
            name: name.to_owned(),
            x,
            y,
            width,
            height,
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
    fn crops_top_left_sprite() {
        let temp = tempfile::tempdir().unwrap();
        let atlas_path = temp.path().join("atlas.png");
        let output_dir = temp.path().join("sprites");
        let manifest_path = temp.path().join("sprites.json");

        let image = ImageBuffer::from_fn(4, 4, |x, y| {
            if x < 2 && y < 2 {
                Rgba([255_u8, 0, 0, 255])
            } else {
                Rgba([0_u8, 0, 0, 255])
            }
        });
        image.save(&atlas_path).unwrap();

        let catalog = CommonSpriteData {
            sprites: vec![sprite("top-left", 0, 0, 2, 2)],
            sprite_indices: json!({}),
        };

        let manifest = crop_catalog_sprites(
            &atlas_path,
            &catalog,
            &output_dir,
            &manifest_path,
            SpriteOrigin::TopLeft,
        )
        .unwrap();

        assert_eq!(manifest.sprites.len(), 1);
        let cropped = image::open(output_dir.join("top-left.png")).unwrap();
        assert_eq!(cropped.dimensions(), (2, 2));
        assert_eq!(cropped.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn clears_stale_sprite_output() {
        let temp = tempfile::tempdir().unwrap();
        let atlas_path = temp.path().join("atlas.png");
        let output_dir = temp.path().join("sprites");
        let manifest_path = temp.path().join("sprites.json");
        fs::create_dir_all(&output_dir).unwrap();
        fs::write(output_dir.join("stale.png"), b"stale").unwrap();
        ImageBuffer::<Rgba<u8>, Vec<u8>>::new(4, 4)
            .save(&atlas_path)
            .unwrap();

        let catalog = CommonSpriteData {
            sprites: vec![sprite("fresh", 0, 0, 1, 1)],
            sprite_indices: json!({}),
        };

        crop_catalog_sprites(
            &atlas_path,
            &catalog,
            &output_dir,
            &manifest_path,
            SpriteOrigin::TopLeft,
        )
        .unwrap();

        assert!(!output_dir.join("stale.png").exists());
        assert!(output_dir.join("fresh.png").is_file());
    }

    #[test]
    fn rejects_sanitized_filename_collision() {
        let temp = tempfile::tempdir().unwrap();
        let atlas_path = temp.path().join("atlas.png");
        let output_dir = temp.path().join("sprites");
        let manifest_path = temp.path().join("sprites.json");
        ImageBuffer::<Rgba<u8>, Vec<u8>>::new(4, 4)
            .save(&atlas_path)
            .unwrap();

        let catalog = CommonSpriteData {
            sprites: vec![sprite("same/name", 0, 0, 1, 1), sprite("same?name", 1, 0, 1, 1)],
            sprite_indices: json!({}),
        };

        let error = crop_catalog_sprites(
            &atlas_path,
            &catalog,
            &output_dir,
            &manifest_path,
            SpriteOrigin::TopLeft,
        )
        .unwrap_err();

        assert!(error.to_string().contains("filename collision"));
    }

    #[test]
    fn rejects_out_of_bounds_sprite() {
        let temp = tempfile::tempdir().unwrap();
        let atlas_path = temp.path().join("atlas.png");
        let output_dir = temp.path().join("sprites");
        let manifest_path = temp.path().join("sprites.json");
        ImageBuffer::<Rgba<u8>, Vec<u8>>::new(4, 4)
            .save(&atlas_path)
            .unwrap();

        let catalog = CommonSpriteData {
            sprites: vec![sprite("bad", 3, 3, 2, 2)],
            sprite_indices: json!({}),
        };

        let error = crop_catalog_sprites(
            &atlas_path,
            &catalog,
            &output_dir,
            &manifest_path,
            SpriteOrigin::TopLeft,
        )
        .unwrap_err();

        assert!(error.to_string().contains("exceeds atlas"));
    }
}
