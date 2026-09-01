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

impl StagedKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnityData => "unity_data",
            Self::Il2CppBinary => "il2cpp_binary",
            Self::Il2CppMetadata => "il2cpp_metadata",
            Self::SpriteMetadata => "sprite_metadata",
            Self::DataList => "data_list",
            Self::GameData => "game_data",
        }
    }
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
