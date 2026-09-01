use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, collections::BTreeSet, fs, path::Path};

const METADATA_MAGIC: u32 = 0xFAB1_1BAF;
const SECTION_STRING: usize = 2;
const SECTION_FIELD_DEFAULT_VALUES: usize = 7;
const SECTION_DEFAULT_VALUE_DATA: usize = 8;
const SECTION_FIELDS: usize = 11;
const SECTION_TYPE_DEFINITIONS: usize = 19;
const V39_TYPE_DEFINITION_SIZE: usize = 82;
const V39_FIELD_DEFINITION_SIZE: usize = 12;
const FIELD_DEFAULT_VALUE_SIZE: usize = 12;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataInventory {
    pub source: String,
    pub metadata_version: Option<i32>,
    pub message_types: Vec<String>,
    pub manager_types: Vec<String>,
    pub interesting_strings: Vec<String>,
    pub protobuf_messages: Vec<ProtobufMessageSchema>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtobufMessageSchema {
    pub name: String,
    pub namespace: String,
    pub type_definition_index: usize,
    pub declared_field_count: u16,
    pub fields: Vec<ProtobufFieldSchema>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtobufFieldSchema {
    pub name: String,
    pub tag: i32,
    pub field_number_constant: String,
    pub field_definition_index: usize,
    pub constant_type_index: i32,
    pub backing_field: Option<String>,
    pub backing_type_index: Option<i32>,
}

#[derive(Debug, Clone, Copy)]
struct Section {
    offset: usize,
    size: usize,
    count: usize,
}

#[derive(Debug, Clone)]
struct FieldDefinition {
    name: String,
    type_index: i32,
}

pub fn inspect_global_metadata(path: &Path) -> Result<MetadataInventory> {
    let bytes =
        fs::read(path).with_context(|| format!("reading global metadata {}", path.display()))?;

    let strings = extract_ascii_strings(&bytes, 4);
    let message_types = strings
        .iter()
        .filter(|value| is_message_type(value))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let manager_types = strings
        .iter()
        .filter(|value| value.ends_with("Manager") || value.ends_with("Manager`1"))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let interesting_strings = strings
        .into_iter()
        .filter(|value| {
            let lower = value.to_ascii_lowercase();
            lower.contains("gamedata")
                || lower.contains("protobuf")
                || lower.contains("monster")
                || lower.contains("skill")
                || lower.contains("item")
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let metadata_version = metadata_version(&bytes);
    let protobuf_messages = match metadata_version {
        Some(38 | 39) => recover_protobuf_schemas_v39(&bytes)?,
        _ => Vec::new(),
    };

    Ok(MetadataInventory {
        source: path.display().to_string(),
        metadata_version,
        message_types,
        manager_types,
        interesting_strings,
        protobuf_messages,
    })
}

pub fn write_metadata_inventory(path: &Path, inventory: &MetadataInventory) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(inventory)?)
        .with_context(|| format!("writing metadata inventory {}", path.display()))?;
    Ok(())
}

pub fn recover_protobuf_schemas_v39(bytes: &[u8]) -> Result<Vec<ProtobufMessageSchema>> {
    let version = metadata_version(bytes).context("missing IL2CPP metadata header")?;
    if !matches!(version, 38 | 39) {
        bail!("protobuf field recovery currently supports IL2CPP metadata v38-v39, got v{version}");
    }

    let strings = section(bytes, SECTION_STRING)?;
    let field_defaults = section(bytes, SECTION_FIELD_DEFAULT_VALUES)?;
    let default_data = section(bytes, SECTION_DEFAULT_VALUE_DATA)?;
    let fields = section(bytes, SECTION_FIELDS)?;
    let type_definitions = section(bytes, SECTION_TYPE_DEFINITIONS)?;

    require_element_size(fields, V39_FIELD_DEFINITION_SIZE, "field definitions")?;
    require_element_size(
        field_defaults,
        FIELD_DEFAULT_VALUE_SIZE,
        "field default values",
    )?;
    require_element_size(
        type_definitions,
        V39_TYPE_DEFINITION_SIZE,
        "type definitions",
    )?;

    let field_definitions = (0..fields.count)
        .map(|index| parse_field_definition(bytes, strings, fields, index))
        .collect::<Result<Vec<_>>>()?;
    let defaults = parse_field_defaults(bytes, field_defaults)?;

    let mut messages = Vec::new();
    for type_index in 0..type_definitions.count {
        let offset = type_definitions.offset + type_index * V39_TYPE_DEFINITION_SIZE;
        let name_index = read_i32(bytes, offset)?;
        let namespace_index = read_i32(bytes, offset + 4)?;
        let name = metadata_string(bytes, strings, name_index)?;
        if !is_message_type(&name) {
            continue;
        }

        // Unity metadata v38/v39 uses packed dynamic-width indices. For this build,
        // the first field index begins at byte 26 and FieldCount at byte 62.
        let first_field = read_i32(bytes, offset + 26)?;
        let declared_field_count = read_u16(bytes, offset + 62)?;
        if first_field < 0 {
            continue;
        }

        let first_field = first_field as usize;
        let field_end = first_field
            .checked_add(declared_field_count as usize)
            .context("type field range overflow")?;
        if field_end > field_definitions.len() {
            bail!(
                "type {name} references fields {first_field}..{field_end}, but only {} exist",
                field_definitions.len()
            );
        }

        let type_fields = &field_definitions[first_field..field_end];
        let mut protobuf_fields = Vec::new();
        for (relative_index, field) in type_fields.iter().enumerate() {
            let Some(base_name) = field.name.strip_suffix("FieldNumber") else {
                continue;
            };
            let field_index = first_field + relative_index;
            let Some((default_type_index, data_index)) = defaults.get(&field_index).copied() else {
                continue;
            };
            if data_index < 0 {
                continue;
            }

            let tag = read_compressed_int32_from_section(bytes, default_data, data_index as usize)?;
            if tag <= 0 {
                continue;
            }

            let backing = find_backing_field(base_name, type_fields);
            protobuf_fields.push(ProtobufFieldSchema {
                name: base_name.to_owned(),
                tag,
                field_number_constant: field.name.clone(),
                field_definition_index: field_index,
                constant_type_index: default_type_index,
                backing_field: backing.map(|value| value.name.clone()),
                backing_type_index: backing.map(|value| value.type_index),
            });
        }
        protobuf_fields.sort_by_key(|field| field.tag);

        messages.push(ProtobufMessageSchema {
            name,
            namespace: metadata_string(bytes, strings, namespace_index)?,
            type_definition_index: type_index,
            declared_field_count,
            fields: protobuf_fields,
        });
    }

    Ok(messages)
}

pub fn extract_ascii_strings(bytes: &[u8], minimum_len: usize) -> Vec<String> {
    let mut strings = Vec::new();
    let mut start = None;

    for (index, byte) in bytes.iter().copied().enumerate() {
        if is_ascii_text(byte) {
            start.get_or_insert(index);
            continue;
        }

        if let Some(begin) = start.take()
            && index.saturating_sub(begin) >= minimum_len
            && let Ok(value) = std::str::from_utf8(&bytes[begin..index])
        {
            strings.push(value.to_owned());
        }
    }

    if let Some(begin) = start
        && bytes.len().saturating_sub(begin) >= minimum_len
        && let Ok(value) = std::str::from_utf8(&bytes[begin..])
    {
        strings.push(value.to_owned());
    }

    strings
}

fn metadata_version(bytes: &[u8]) -> Option<i32> {
    if bytes.len() < 8 || u32::from_le_bytes(bytes[0..4].try_into().ok()?) != METADATA_MAGIC {
        return None;
    }
    Some(i32::from_le_bytes(bytes[4..8].try_into().ok()?))
}

fn section(bytes: &[u8], index: usize) -> Result<Section> {
    let base = 8 + index * 12;
    let offset = read_i32(bytes, base)?;
    let size = read_i32(bytes, base + 4)?;
    let count = read_i32(bytes, base + 8)?;
    if offset < 0 || size < 0 || count < 0 {
        bail!("metadata section {index} contains a negative offset, size, or count");
    }

    let section = Section {
        offset: offset as usize,
        size: size as usize,
        count: count as usize,
    };
    let end = section
        .offset
        .checked_add(section.size)
        .context("metadata section end overflow")?;
    if end > bytes.len() {
        bail!(
            "metadata section {index} extends to {end}, beyond file length {}",
            bytes.len()
        );
    }
    Ok(section)
}

fn require_element_size(section: Section, expected: usize, label: &str) -> Result<()> {
    if section.count == 0 {
        return Ok(());
    }
    if section.size / section.count != expected || !section.size.is_multiple_of(section.count) {
        bail!(
            "unexpected {label} element size: {}/{} (expected {expected})",
            section.size,
            section.count
        );
    }
    Ok(())
}

fn parse_field_definition(
    bytes: &[u8],
    strings: Section,
    fields: Section,
    index: usize,
) -> Result<FieldDefinition> {
    let offset = fields.offset + index * V39_FIELD_DEFINITION_SIZE;
    let name_index = read_i32(bytes, offset)?;
    Ok(FieldDefinition {
        name: metadata_string(bytes, strings, name_index)?,
        type_index: read_i32(bytes, offset + 4)?,
    })
}

fn parse_field_defaults(bytes: &[u8], section: Section) -> Result<BTreeMap<usize, (i32, i32)>> {
    let mut defaults = BTreeMap::new();
    for index in 0..section.count {
        let offset = section.offset + index * FIELD_DEFAULT_VALUE_SIZE;
        let field_index = read_i32(bytes, offset)?;
        if field_index < 0 {
            continue;
        }
        defaults.insert(
            field_index as usize,
            (read_i32(bytes, offset + 4)?, read_i32(bytes, offset + 8)?),
        );
    }
    Ok(defaults)
}

fn metadata_string(bytes: &[u8], strings: Section, index: i32) -> Result<String> {
    if index < 0 || index as usize >= strings.size {
        bail!("metadata string index {index} is outside string section");
    }
    let start = strings.offset + index as usize;
    let end_limit = strings.offset + strings.size;
    let end = bytes[start..end_limit]
        .iter()
        .position(|byte| *byte == 0)
        .map(|relative| start + relative)
        .context("unterminated metadata string")?;
    Ok(String::from_utf8_lossy(&bytes[start..end]).into_owned())
}

fn find_backing_field<'a>(
    base_name: &str,
    fields: &'a [FieldDefinition],
) -> Option<&'a FieldDefinition> {
    let wanted = normalize_identifier(base_name);
    fields.iter().find(|field| {
        field.name.ends_with('_')
            && !field.name.starts_with("_repeated_")
            && normalize_identifier(field.name.trim_end_matches('_')) == wanted
    })
}

fn normalize_identifier(value: &str) -> String {
    value
        .bytes()
        .filter(|byte| byte.is_ascii_alphanumeric())
        .map(|byte| byte.to_ascii_lowercase() as char)
        .collect()
}

fn read_compressed_int32_from_section(
    bytes: &[u8],
    section: Section,
    relative_offset: usize,
) -> Result<i32> {
    if relative_offset >= section.size {
        bail!("compressed integer offset {relative_offset} is outside default-value data");
    }
    read_compressed_int32(&bytes[section.offset + relative_offset..section.offset + section.size])
}

fn read_compressed_int32(bytes: &[u8]) -> Result<i32> {
    let encoded = read_compressed_uint32(bytes)?;
    if encoded == u32::MAX {
        return Ok(i32::MIN);
    }
    let negative = encoded & 1 != 0;
    let value = encoded >> 1;
    if negative {
        Ok(-((value as i64 + 1) as i32))
    } else {
        Ok(value as i32)
    }
}

fn read_compressed_uint32(bytes: &[u8]) -> Result<u32> {
    let first = *bytes.first().context("empty compressed integer")?;
    let value = if first & 0x80 == 0 {
        first as u32
    } else if first & 0xC0 == 0x80 {
        ensure_len(bytes, 2)?;
        ((first as u32 & !0x80) << 8) | bytes[1] as u32
    } else if first & 0xE0 == 0xC0 {
        ensure_len(bytes, 4)?;
        ((first as u32 & !0xC0) << 24)
            | ((bytes[1] as u32) << 16)
            | ((bytes[2] as u32) << 8)
            | bytes[3] as u32
    } else if first == 0xF0 {
        ensure_len(bytes, 5)?;
        u32::from_le_bytes(bytes[1..5].try_into().expect("slice length checked"))
    } else if first == 0xFE {
        u32::MAX - 1
    } else if first == 0xFF {
        u32::MAX
    } else {
        bail!("invalid IL2CPP compressed integer prefix 0x{first:02x}");
    };
    Ok(value)
}

fn ensure_len(bytes: &[u8], required: usize) -> Result<()> {
    if bytes.len() < required {
        bail!(
            "compressed integer needs {required} bytes, only {} remain",
            bytes.len()
        );
    }
    Ok(())
}

fn read_i32(bytes: &[u8], offset: usize) -> Result<i32> {
    let end = offset.checked_add(4).context("i32 offset overflow")?;
    let slice = bytes
        .get(offset..end)
        .with_context(|| format!("i32 read {offset}..{end} exceeds metadata length"))?;
    Ok(i32::from_le_bytes(
        slice.try_into().expect("slice length checked"),
    ))
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16> {
    let end = offset.checked_add(2).context("u16 offset overflow")?;
    let slice = bytes
        .get(offset..end)
        .with_context(|| format!("u16 read {offset}..{end} exceeds metadata length"))?;
    Ok(u16::from_le_bytes(
        slice.try_into().expect("slice length checked"),
    ))
}

fn is_ascii_text(byte: u8) -> bool {
    matches!(byte, 0x20..=0x7e)
}

fn is_message_type(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("Msg") else {
        return false;
    };
    !rest.is_empty()
        && rest.len() <= 128
        && rest
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_message_inventory() {
        let bytes = b"xx\0MsgMonster\0MsgMonsterSkill\0GameDataManager\0Google.Protobuf\0junk";
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), bytes).unwrap();

        let inventory = inspect_global_metadata(temp.path()).unwrap();
        assert_eq!(
            inventory.message_types,
            vec!["MsgMonster".to_owned(), "MsgMonsterSkill".to_owned()]
        );
        assert!(inventory.protobuf_messages.is_empty());
        assert!(
            inventory
                .manager_types
                .contains(&"GameDataManager".to_owned())
        );
        assert!(
            inventory
                .interesting_strings
                .contains(&"Google.Protobuf".to_owned())
        );
    }

    #[test]
    fn decodes_il2cpp_compressed_field_numbers() {
        assert_eq!(read_compressed_int32(&[0x02]).unwrap(), 1);
        assert_eq!(read_compressed_int32(&[0x04]).unwrap(), 2);
        assert_eq!(read_compressed_int32(&[0x14]).unwrap(), 10);
        assert_eq!(read_compressed_int32(&[0x87, 0xE2]).unwrap(), 1009);
    }

    #[test]
    fn matches_generated_backing_fields() {
        let fields = vec![
            FieldDefinition {
                name: "_repeated_monsters_codec".to_owned(),
                type_index: 1,
            },
            FieldDefinition {
                name: "monsters_".to_owned(),
                type_index: 2,
            },
        ];
        let field = find_backing_field("Monsters", &fields).unwrap();
        assert_eq!(field.name, "monsters_");
        assert_eq!(field.type_index, 2);
    }
}
