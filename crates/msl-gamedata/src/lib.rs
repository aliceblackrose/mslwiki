use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataInventory {
    pub source: String,
    pub message_types: Vec<String>,
    pub manager_types: Vec<String>,
    pub interesting_strings: Vec<String>,
}

pub fn inspect_global_metadata(path: &Path) -> Result<MetadataInventory> {
    let bytes = fs::read(path)
        .with_context(|| format!("reading global metadata {}", path.display()))?;

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

    Ok(MetadataInventory {
        source: path.display().to_string(),
        message_types,
        manager_types,
        interesting_strings,
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
}
