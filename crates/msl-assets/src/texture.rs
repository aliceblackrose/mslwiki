use crate::serialized::{ObjectReader, SerializedFile, SerializedType, UnityVersion};
use anyhow::{Context, Result, bail, ensure};
use image::{ImageFormat, RgbaImage};
use std::{fs, path::Path};

const NO_TARGET_PLATFORM: i32 = -2;
const MAX_TEXTURE_PIXELS: usize = 268_435_456;
type TextureBlockDecoder =
    fn(&[u8], usize, usize, &mut [u32]) -> Result<(), &'static str>;

#[derive(Debug, Clone)]
pub(crate) enum TextureData {
    Inline(Vec<u8>),
    Streamed(StreamData),
}

#[derive(Debug, Clone)]
pub(crate) struct StreamData {
    pub offset: u64,
    pub size: usize,
    pub path: String,
}

#[derive(Debug, Clone)]
pub(crate) struct Texture2D {
    pub name: String,
    pub width: usize,
    pub height: usize,
    pub format: i32,
    pub mip_count: i32,
    pub data: TextureData,
}

pub(crate) fn parse_texture2d(
    file: &SerializedFile<'_>,
    object: &crate::serialized::ObjectInfo,
) -> Result<Texture2D> {
    ensure!(
        file.target_platform != NO_TARGET_PLATFORM,
        "editor-only Texture2D layout is not supported"
    );

    let object_type = file.object_type(object)?;
    let bytes = file.object_bytes(object)?;
    let mut reader = ObjectReader::new(bytes, file.endian);
    let version = &file.unity_version;

    let name = reader.read_aligned_string()?;
    read_texture_base(&mut reader, version)?;

    let width = positive_dimension(reader.read_i32()?, "width")?;
    let height = positive_dimension(reader.read_i32()?, "height")?;
    let _complete_image_size = reader.read_i32()?;
    if version.at_least(2020, 1) {
        let _mips_stripped = reader.read_i32()?;
    }

    let format = reader.read_i32()?;
    let mip_count = reader.read_i32()?;
    let _is_readable = reader.read_bool()?;
    if version.at_least(2020, 1) {
        let _is_preprocessed = reader.read_bool()?;
    }
    if version.at_least(2019, 3) {
        let _ignore_master_texture_limit = reader.read_bool()?;
    }

    if version.at_least(2022, 2)
        && type_tree_field_exists(file, object_type, "m_MipmapLimitGroupName")
    {
        reader.align(4)?;
        let _mipmap_limit_group_name = reader.read_aligned_string()?;
    }

    if version.at_least(2018, 2) {
        let _streaming_mipmaps = reader.read_bool()?;
    }
    reader.align(4)?;
    if version.at_least(2018, 2) {
        let _streaming_mipmaps_priority = reader.read_i32()?;
    }

    let _image_count = reader.read_i32()?;
    let _texture_dimension = reader.read_i32()?;
    read_gl_texture_settings(&mut reader, version)?;
    let _lightmap_format = reader.read_i32()?;
    let _color_space = reader.read_i32()?;

    if version.at_least(2020, 2) && type_tree_field_exists(file, object_type, "m_PlatformBlob") {
        let blob_size = reader.read_i32()?;
        ensure!(
            blob_size >= 0,
            "Texture2D platform blob has a negative size"
        );
        let blob_size = usize::try_from(blob_size).expect("non-negative i32 fits usize");
        ensure!(
            blob_size <= reader.remaining(),
            "Texture2D platform blob exceeds object bounds"
        );
        let _platform_blob = reader.read_bytes(blob_size)?;
        reader.align(4)?;
    }

    let image_data_size = reader.read_i32()?;
    ensure!(
        image_data_size >= 0,
        "Texture2D image data has a negative size"
    );
    let image_data_size = usize::try_from(image_data_size).expect("non-negative i32 fits usize");
    let data = if image_data_size == 0 {
        let offset = reader.read_i64()?;
        ensure!(offset >= 0, "Texture2D stream offset is negative");
        let size = usize::try_from(reader.read_u32()?)
            .context("Texture2D stream size does not fit usize")?;
        let path = reader.read_aligned_string()?;
        TextureData::Streamed(StreamData {
            offset: u64::try_from(offset).expect("non-negative i64 fits u64"),
            size,
            path,
        })
    } else {
        ensure!(
            image_data_size <= reader.remaining(),
            "Texture2D inline image data exceeds object bounds"
        );
        TextureData::Inline(reader.read_bytes(image_data_size)?.to_vec())
    };

    let pixels = width
        .checked_mul(height)
        .context("Texture2D pixel count overflow")?;
    ensure!(
        pixels <= MAX_TEXTURE_PIXELS,
        "Texture2D {width}x{height} exceeds pixel safety limit"
    );

    Ok(Texture2D {
        name,
        width,
        height,
        format,
        mip_count,
        data,
    })
}

fn read_texture_base(reader: &mut ObjectReader<'_>, version: &UnityVersion) -> Result<()> {
    if version.at_least(2017, 3) {
        if version.before(2023, 2) {
            let _forced_fallback_format = reader.read_i32()?;
            let _downscale_fallback = reader.read_bool()?;
        }
        if version.at_least(2020, 2) {
            let _is_alpha_channel_optional = reader.read_bool()?;
        }
        reader.align(4)?;
    }
    Ok(())
}

fn read_gl_texture_settings(reader: &mut ObjectReader<'_>, version: &UnityVersion) -> Result<()> {
    let _filter_mode = reader.read_i32()?;
    let _aniso = reader.read_i32()?;
    let _mip_bias = reader.read_f32()?;
    let _wrap_u = reader.read_i32()?;
    if version.at_least(2017, 1) {
        let _wrap_v = reader.read_i32()?;
        let _wrap_w = reader.read_i32()?;
    }
    Ok(())
}

fn type_tree_field_exists(
    file: &SerializedFile<'_>,
    object_type: &SerializedType,
    field: &str,
) -> bool {
    !file.enable_type_tree || object_type.has_field(field)
}

fn positive_dimension(value: i32, label: &str) -> Result<usize> {
    ensure!(value > 0, "Texture2D {label} must be positive, got {value}");
    usize::try_from(value).context("Texture2D dimension does not fit usize")
}

pub(crate) fn decode_texture_rgba(texture: &Texture2D, encoded: &[u8]) -> Result<Vec<u8>> {
    let pixel_count = texture
        .width
        .checked_mul(texture.height)
        .context("Texture2D pixel count overflow")?;
    let rgba_len = pixel_count
        .checked_mul(4)
        .context("Texture2D decoded byte count overflow")?;

    let mut rgba = match texture.format {
        1 => decode_alpha8(encoded, pixel_count)?,
        3 => decode_rgb24(encoded, pixel_count, false)?,
        4 => copy_four_channel(encoded, pixel_count, ChannelOrder::Rgba)?,
        5 => copy_four_channel(encoded, pixel_count, ChannelOrder::Argb)?,
        7 => decode_rgb565(encoded, pixel_count)?,
        8 => decode_rgb24(encoded, pixel_count, true)?,
        9 | 66 => decode_r16(encoded, pixel_count)?,
        10 => decode_compressed(texture, encoded, texture2ddecoder::decode_bc1)?,
        11 => decode_compressed(texture, encoded, texture2ddecoder::decode_bc2)?,
        12 => decode_compressed(texture, encoded, texture2ddecoder::decode_bc3)?,
        14 => copy_four_channel(encoded, pixel_count, ChannelOrder::Bgra)?,
        24 => decode_compressed(texture, encoded, texture2ddecoder::decode_bc6_unsigned)?,
        25 => decode_compressed(texture, encoded, texture2ddecoder::decode_bc7)?,
        26 => decode_compressed(texture, encoded, texture2ddecoder::decode_bc4)?,
        27 => decode_compressed(texture, encoded, texture2ddecoder::decode_bc5)?,
        28 | 29 | 64 | 65 => decode_crunched(texture, encoded)?,
        30 | 31 => decode_compressed(texture, encoded, texture2ddecoder::decode_pvrtc_2bpp)?,
        32 | 33 => decode_compressed(texture, encoded, texture2ddecoder::decode_pvrtc_4bpp)?,
        34 => decode_compressed(texture, encoded, texture2ddecoder::decode_etc1)?,
        35 => decode_compressed(texture, encoded, texture2ddecoder::decode_atc_rgb4)?,
        36 => decode_compressed(texture, encoded, texture2ddecoder::decode_atc_rgba8)?,
        41 => decode_compressed(texture, encoded, texture2ddecoder::decode_eacr)?,
        42 => decode_compressed(texture, encoded, texture2ddecoder::decode_eacr_signed)?,
        43 => decode_compressed(texture, encoded, texture2ddecoder::decode_eacrg)?,
        44 => decode_compressed(texture, encoded, texture2ddecoder::decode_eacrg_signed)?,
        45 => decode_compressed(texture, encoded, texture2ddecoder::decode_etc2_rgb)?,
        46 => decode_compressed(texture, encoded, texture2ddecoder::decode_etc2_rgba1)?,
        47 => decode_compressed(texture, encoded, texture2ddecoder::decode_etc2_rgba8)?,
        48 | 54 | 67 => decode_astc(texture, encoded, 4, 4)?,
        49 | 55 | 68 => decode_astc(texture, encoded, 5, 5)?,
        50 | 56 | 69 => decode_astc(texture, encoded, 6, 6)?,
        51 | 57 | 70 => decode_astc(texture, encoded, 8, 8)?,
        52 | 58 | 71 => decode_astc(texture, encoded, 10, 10)?,
        53 | 59 | 72 => decode_astc(texture, encoded, 12, 12)?,
        62 => decode_rg16(encoded, pixel_count)?,
        63 => decode_r8(encoded, pixel_count)?,
        73 => decode_rg32(encoded, pixel_count)?,
        74 => decode_rgb48(encoded, pixel_count)?,
        75 => decode_rgba64(encoded, pixel_count)?,
        other => bail!(
            "Texture2D format {} ({other}) is not supported for PNG export",
            texture_format_name(other)
        ),
    };

    ensure!(
        rgba.len() == rgba_len,
        "decoded Texture2D byte count mismatch: expected {rgba_len}, got {}",
        rgba.len()
    );
    flip_rgba_vertical(&mut rgba, texture.width, texture.height)?;
    Ok(rgba)
}

fn decode_alpha8(encoded: &[u8], pixel_count: usize) -> Result<Vec<u8>> {
    let source = encoded
        .get(..pixel_count)
        .context("Alpha8 texture data is truncated")?;
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for alpha in source {
        rgba.extend_from_slice(&[255, 255, 255, *alpha]);
    }
    Ok(rgba)
}

fn decode_r8(encoded: &[u8], pixel_count: usize) -> Result<Vec<u8>> {
    let source = encoded
        .get(..pixel_count)
        .context("R8 texture data is truncated")?;
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for red in source {
        rgba.extend_from_slice(&[*red, *red, *red, 255]);
    }
    Ok(rgba)
}

fn decode_r16(encoded: &[u8], pixel_count: usize) -> Result<Vec<u8>> {
    let byte_count = pixel_count
        .checked_mul(2)
        .context("R16 texture byte count overflow")?;
    let source = encoded
        .get(..byte_count)
        .context("R16 texture data is truncated")?;
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for pixel in source.as_chunks::<2>().0 {
        let red = normalize_u16(pixel[0], pixel[1]);
        rgba.extend_from_slice(&[red, red, red, 255]);
    }
    Ok(rgba)
}

fn decode_rg16(encoded: &[u8], pixel_count: usize) -> Result<Vec<u8>> {
    let byte_count = pixel_count
        .checked_mul(2)
        .context("RG16 texture byte count overflow")?;
    let source = encoded
        .get(..byte_count)
        .context("RG16 texture data is truncated")?;
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for pixel in source.as_chunks::<2>().0 {
        rgba.extend_from_slice(&[pixel[0], pixel[1], 0, 255]);
    }
    Ok(rgba)
}

fn decode_rg32(encoded: &[u8], pixel_count: usize) -> Result<Vec<u8>> {
    let byte_count = pixel_count
        .checked_mul(4)
        .context("RG32 texture byte count overflow")?;
    let source = encoded
        .get(..byte_count)
        .context("RG32 texture data is truncated")?;
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for pixel in source.as_chunks::<4>().0 {
        rgba.extend_from_slice(&[
            normalize_u16(pixel[0], pixel[1]),
            normalize_u16(pixel[2], pixel[3]),
            0,
            255,
        ]);
    }
    Ok(rgba)
}

fn decode_rgb48(encoded: &[u8], pixel_count: usize) -> Result<Vec<u8>> {
    let byte_count = pixel_count
        .checked_mul(6)
        .context("RGB48 texture byte count overflow")?;
    let source = encoded
        .get(..byte_count)
        .context("RGB48 texture data is truncated")?;
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for pixel in source.as_chunks::<6>().0 {
        rgba.extend_from_slice(&[
            normalize_u16(pixel[0], pixel[1]),
            normalize_u16(pixel[2], pixel[3]),
            normalize_u16(pixel[4], pixel[5]),
            255,
        ]);
    }
    Ok(rgba)
}

fn decode_rgba64(encoded: &[u8], pixel_count: usize) -> Result<Vec<u8>> {
    let byte_count = pixel_count
        .checked_mul(8)
        .context("RGBA64 texture byte count overflow")?;
    let source = encoded
        .get(..byte_count)
        .context("RGBA64 texture data is truncated")?;
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for pixel in source.as_chunks::<8>().0 {
        rgba.extend_from_slice(&[
            normalize_u16(pixel[0], pixel[1]),
            normalize_u16(pixel[2], pixel[3]),
            normalize_u16(pixel[4], pixel[5]),
            normalize_u16(pixel[6], pixel[7]),
        ]);
    }
    Ok(rgba)
}

fn normalize_u16(low: u8, high: u8) -> u8 {
    let value = u16::from_le_bytes([low, high]);
    ((u32::from(value) * 255 + 32_767) / 65_535) as u8
}

fn decode_rgb24(encoded: &[u8], pixel_count: usize, bgr: bool) -> Result<Vec<u8>> {
    let byte_count = pixel_count
        .checked_mul(3)
        .context("RGB24 texture byte count overflow")?;
    let source = encoded
        .get(..byte_count)
        .context("RGB24 texture data is truncated")?;
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for pixel in source.as_chunks::<3>().0 {
        if bgr {
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
        } else {
            rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
        }
    }
    Ok(rgba)
}

#[derive(Debug, Clone, Copy)]
enum ChannelOrder {
    Rgba,
    Argb,
    Bgra,
}

fn copy_four_channel(encoded: &[u8], pixel_count: usize, order: ChannelOrder) -> Result<Vec<u8>> {
    let byte_count = pixel_count
        .checked_mul(4)
        .context("RGBA texture byte count overflow")?;
    let source = encoded
        .get(..byte_count)
        .context("RGBA texture data is truncated")?;
    let mut rgba = Vec::with_capacity(byte_count);
    for pixel in source.as_chunks::<4>().0 {
        match order {
            ChannelOrder::Rgba => rgba.extend_from_slice(pixel),
            ChannelOrder::Argb => rgba.extend_from_slice(&[pixel[1], pixel[2], pixel[3], pixel[0]]),
            ChannelOrder::Bgra => rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]),
        }
    }
    Ok(rgba)
}

fn decode_rgb565(encoded: &[u8], pixel_count: usize) -> Result<Vec<u8>> {
    let byte_count = pixel_count
        .checked_mul(2)
        .context("RGB565 texture byte count overflow")?;
    let source = encoded
        .get(..byte_count)
        .context("RGB565 texture data is truncated")?;
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for pixel in source.as_chunks::<2>().0 {
        let value = u16::from_le_bytes([pixel[0], pixel[1]]);
        let red = (((value >> 11) & 0x1f) * 255 / 31) as u8;
        let green = (((value >> 5) & 0x3f) * 255 / 63) as u8;
        let blue = ((value & 0x1f) * 255 / 31) as u8;
        rgba.extend_from_slice(&[red, green, blue, 255]);
    }
    Ok(rgba)
}

fn decode_compressed(
    texture: &Texture2D,
    encoded: &[u8],
    decoder: TextureBlockDecoder,
) -> Result<Vec<u8>> {
    let mut pixels = vec![0_u32; texture.width * texture.height];
    decoder(encoded, texture.width, texture.height, &mut pixels)
        .map_err(|message| anyhow::anyhow!(message))?;
    Ok(bgra_words_to_rgba(&pixels))
}

fn decode_astc(
    texture: &Texture2D,
    encoded: &[u8],
    block_width: usize,
    block_height: usize,
) -> Result<Vec<u8>> {
    let mut pixels = vec![0_u32; texture.width * texture.height];
    texture2ddecoder::decode_astc(
        encoded,
        texture.width,
        texture.height,
        block_width,
        block_height,
        &mut pixels,
    )
    .map_err(|message| anyhow::anyhow!(message))?;
    Ok(bgra_words_to_rgba(&pixels))
}

fn decode_crunched(texture: &Texture2D, encoded: &[u8]) -> Result<Vec<u8>> {
    let mut pixels = vec![0_u32; texture.width * texture.height];
    if let Err(unity_error) =
        texture2ddecoder::decode_unity_crunch(encoded, texture.width, texture.height, &mut pixels)
    {
        texture2ddecoder::decode_crunch(encoded, texture.width, texture.height, &mut pixels)
            .map_err(|crunch_error| {
                anyhow::anyhow!(
                    "Unity Crunch decode failed ({unity_error}); Crunch decode failed ({crunch_error})"
                )
            })?;
    }
    Ok(bgra_words_to_rgba(&pixels))
}

fn bgra_words_to_rgba(pixels: &[u32]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(pixels.len() * 4);
    for pixel in pixels {
        let [blue, green, red, alpha] = pixel.to_le_bytes();
        rgba.extend_from_slice(&[red, green, blue, alpha]);
    }
    rgba
}

fn flip_rgba_vertical(rgba: &mut [u8], width: usize, height: usize) -> Result<()> {
    let row_bytes = width
        .checked_mul(4)
        .context("Texture2D row byte count overflow")?;
    ensure!(
        rgba.len() == row_bytes.saturating_mul(height),
        "Texture2D buffer size does not match dimensions"
    );

    for top in 0..height / 2 {
        let bottom = height - 1 - top;
        let top_start = top * row_bytes;
        let bottom_start = bottom * row_bytes;
        for byte in 0..row_bytes {
            rgba.swap(top_start + byte, bottom_start + byte);
        }
    }
    Ok(())
}

pub(crate) fn save_png(path: &Path, texture: &Texture2D, rgba: Vec<u8>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let width = u32::try_from(texture.width).context("Texture2D width does not fit u32")?;
    let height = u32::try_from(texture.height).context("Texture2D height does not fit u32")?;
    let image =
        RgbaImage::from_raw(width, height, rgba).context("building decoded Texture2D image")?;
    image
        .save_with_format(path, ImageFormat::Png)
        .with_context(|| format!("writing Texture2D PNG {}", path.display()))
}

pub(crate) fn texture_format_name(format: i32) -> &'static str {
    match format {
        1 => "Alpha8",
        2 => "ARGB4444",
        3 => "RGB24",
        4 => "RGBA32",
        5 => "ARGB32",
        6 => "ARGBFloat",
        7 => "RGB565",
        8 => "BGR24",
        9 => "R16",
        10 => "DXT1",
        11 => "DXT3",
        12 => "DXT5",
        13 => "RGBA4444",
        14 => "BGRA32",
        24 => "BC6H",
        25 => "BC7",
        26 => "BC4",
        27 => "BC5",
        28 => "DXT1Crunched",
        29 => "DXT5Crunched",
        30 => "PVRTC_RGB2",
        31 => "PVRTC_RGBA2",
        32 => "PVRTC_RGB4",
        33 => "PVRTC_RGBA4",
        34 => "ETC_RGB4",
        35 => "ATC_RGB4",
        36 => "ATC_RGBA8",
        41 => "EAC_R",
        42 => "EAC_R_SIGNED",
        43 => "EAC_RG",
        44 => "EAC_RG_SIGNED",
        45 => "ETC2_RGB",
        46 => "ETC2_RGBA1",
        47 => "ETC2_RGBA8",
        48 => "ASTC_RGB_4x4",
        49 => "ASTC_RGB_5x5",
        50 => "ASTC_RGB_6x6",
        51 => "ASTC_RGB_8x8",
        52 => "ASTC_RGB_10x10",
        53 => "ASTC_RGB_12x12",
        54 => "ASTC_RGBA_4x4",
        55 => "ASTC_RGBA_5x5",
        56 => "ASTC_RGBA_6x6",
        57 => "ASTC_RGBA_8x8",
        58 => "ASTC_RGBA_10x10",
        59 => "ASTC_RGBA_12x12",
        60 => "ETC_RGB4_3DS",
        61 => "ETC_RGBA8_3DS",
        62 => "RG16",
        63 => "R8",
        64 => "ETC_RGB4Crunched",
        65 => "ETC2_RGBA8Crunched",
        66 => "R16_Alt",
        67 => "ASTC_HDR_4x4",
        68 => "ASTC_HDR_5x5",
        69 => "ASTC_HDR_6x6",
        70 => "ASTC_HDR_8x8",
        71 => "ASTC_HDR_10x10",
        72 => "ASTC_HDR_12x12",
        73 => "RG32",
        74 => "RGB48",
        75 => "RGBA64",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialized::{Endian, ObjectInfo, SerializedType};
    use std::collections::BTreeSet;

    #[test]
    fn decodes_inline_unity6_rgba_texture_and_flips_y() {
        let texture = Texture2D {
            name: "test".to_owned(),
            width: 2,
            height: 2,
            format: 4,
            mip_count: 1,
            data: TextureData::Inline(Vec::new()),
        };
        let encoded = [
            255, 0, 0, 255, 0, 255, 0, 255, // source bottom row
            0, 0, 255, 255, 255, 255, 255, 255, // source top row
        ];
        let rgba = decode_texture_rgba(&texture, &encoded).unwrap();
        assert_eq!(
            rgba,
            vec![
                0, 0, 255, 255, 255, 255, 255, 255, 255, 0, 0, 255, 0, 255, 0, 255
            ]
        );
    }

    #[test]
    fn parses_unity6_texture_layout_with_typetree_fields() {
        let object_bytes = build_unity6_texture_object();
        let mut fields = BTreeSet::new();
        fields.insert("m_MipmapLimitGroupName".to_owned());
        fields.insert("m_PlatformBlob".to_owned());
        let ty = SerializedType {
            class_id: 28,
            fields,
        };
        let file_bytes = object_bytes.clone();
        let file = SerializedFile {
            endian: Endian::Little,
            unity_version: UnityVersion::parse("6000.3.12f1".to_owned()),
            target_platform: 13,
            enable_type_tree: true,
            types: vec![ty],
            objects: vec![ObjectInfo {
                path_id: 7,
                byte_start: 0,
                byte_size: file_bytes.len(),
                type_index: 0,
                class_id: 28,
            }],
            bytes: &file_bytes,
        };

        let texture = parse_texture2d(&file, &file.objects[0]).unwrap();
        assert_eq!(texture.name, "_common");
        assert_eq!((texture.width, texture.height), (2, 2));
        assert_eq!(texture.format, 4);
        match texture.data {
            TextureData::Inline(data) => assert_eq!(data.len(), 16),
            TextureData::Streamed(_) => panic!("expected inline texture"),
        }
    }

    #[test]
    fn normalizes_sixteen_bit_channels() {
        let texture = Texture2D {
            name: "r16".to_owned(),
            width: 2,
            height: 1,
            format: 9,
            mip_count: 1,
            data: TextureData::Inline(Vec::new()),
        };
        let rgba = decode_texture_rgba(&texture, &[0, 0, 255, 255]).unwrap();
        assert_eq!(rgba, vec![0, 0, 0, 255, 255, 255, 255, 255]);
    }

    fn build_unity6_texture_object() -> Vec<u8> {
        let mut bytes = Vec::new();
        push_string(&mut bytes, "_common");
        bytes.push(0); // m_IsAlphaChannelOptional
        align(&mut bytes, 4);
        push_i32(&mut bytes, 2); // width
        push_i32(&mut bytes, 2); // height
        push_i32(&mut bytes, 16); // complete image size
        push_i32(&mut bytes, 0); // mips stripped
        push_i32(&mut bytes, 4); // RGBA32
        push_i32(&mut bytes, 1); // mip count
        bytes.extend_from_slice(&[1, 0, 0]); // readable, preprocessed, ignore limit
        align(&mut bytes, 4);
        push_string(&mut bytes, ""); // mipmap limit group
        bytes.push(0); // streaming mipmaps
        align(&mut bytes, 4);
        push_i32(&mut bytes, 0); // streaming priority
        push_i32(&mut bytes, 1); // image count
        push_i32(&mut bytes, 2); // texture dimension
        push_i32(&mut bytes, 1); // filter
        push_i32(&mut bytes, 1); // aniso
        bytes.extend_from_slice(&0_f32.to_le_bytes());
        push_i32(&mut bytes, 0); // wrap U
        push_i32(&mut bytes, 0); // wrap V
        push_i32(&mut bytes, 0); // wrap W
        push_i32(&mut bytes, 0); // lightmap
        push_i32(&mut bytes, 1); // color space
        push_i32(&mut bytes, 0); // platform blob size
        align(&mut bytes, 4);
        push_i32(&mut bytes, 16); // image data size
        bytes.extend_from_slice(&[0_u8; 16]);
        bytes
    }

    fn push_i32(bytes: &mut Vec<u8>, value: i32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_string(bytes: &mut Vec<u8>, value: &str) {
        push_i32(bytes, value.len() as i32);
        bytes.extend_from_slice(value.as_bytes());
        align(bytes, 4);
    }

    fn align(bytes: &mut Vec<u8>, alignment: usize) {
        let target = (bytes.len() + alignment - 1) & !(alignment - 1);
        bytes.resize(target, 0);
    }
}
