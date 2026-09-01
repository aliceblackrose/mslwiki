use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GamePackageMetadata {
    pub xapk_version: Option<String>,
    pub package_name: String,
    pub name: String,
    pub version_code: String,
    pub version_name: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StagedKind {
    UnityData,
    Il2CppBinary,
    Il2CppMetadata,
    SpriteMetadata,
    DataList,
    GameData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedFile {
    pub source_apk: String,
    pub archive_path: String,
    pub staged_path: String,
    pub content_hash: String,
    pub size: u64,
    pub kind: StagedKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportManifest {
    pub source_path: String,
    pub source_hash: String,
    pub package: GamePackageMetadata,
    pub files: Vec<StagedFile>,
}

impl ImportManifest {
    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}
