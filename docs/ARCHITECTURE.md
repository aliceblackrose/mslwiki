# Architecture

`mslwiki` keeps source-format reverse engineering out of the website layer.

## Layers

### `msl-ingest`

Accepts XAPK inputs, reads `manifest.json`, opens nested split APKs, and stages only files relevant to Unity, IL2CPP, sprite metadata, and game data. The original split APK name is preserved in the staging path so duplicate Unity paths never silently overwrite one another.

It also creates the deterministic merged Unity `Data` view. A conflicting duplicate relative path is an error rather than an implicit overwrite.

### `msl-assets`

Owns MSL/Unity asset processing after staging. It parses `CommonSpriteData.txt`, performs dependency discovery, owns the native Unity extraction backend, converts supported Unity objects into web assets, and manages the content-addressed asset store.

The Unity backend is intentionally layered:

1. **container layer** — detect UnityFS bundles, parse block/directory metadata, decompress blocks, and expose safe bundle entries;
2. **serialized-file layer** — parse modern SerializedFile headers, object tables, TypeTrees, and object byte ranges;
3. **resource layer** — resolve external streamed payloads such as `.resS` files with bounded offset/size reads;
4. **object layer** — decode the object classes MSL currently needs (`Texture2D` and `TextAsset`);
5. **conversion layer** — convert supported texture encodings to RGBA/PNG and text assets to deterministic files;
6. **MSL atlas layer** — select the largest decoded `Texture2D` named `_common` and crop `CommonSpriteData` regions;
7. **store layer** — hash exported files with BLAKE3 and deduplicate identical physical content.

All of those layers are in-tree Rust code. The extraction path does not invoke AssetRipper or another Unity extraction executable.

Additional Unity object classes should only be added when MSL data requires them. This keeps source-format parsing narrow and testable instead of turning the repository into a general-purpose editor/project exporter.

### `msl-core`

Stable project-owned domain types. Future protobuf-generated types and Unity-specific structures must be translated into these types before storage or presentation.

### `msl-db`

SQLite persistence for imported game versions and staged-file inventory. This becomes the normalized Astromon/skill/item/localization database as decoders are added.

### `msl-web`

Axum HTML and JSON API. It only consumes normalized database data; it does not parse APKs, protobuf messages, or Unity files.

### `msl-cli`

Composition root for commands such as `import`, `extract-assets`, and `serve`.

## Extraction pipeline

```text
XAPK
  |
  +-- manifest.json ----------------------+
  |                                       |
  +-- base APK                            |
  |     +-- global-metadata.dat           |
  |     +-- Unity Data                    |
  |                                       v
  +-- ABI split APK -> libil2cpp.so -> staged split inventory
  |                                       |
  +-- Unity asset packs -> Data ----------+
  |                                       |
  +-- CommonSpriteData.txt ---------------+
                                          v
                                merged Unity Data view
                                          |
                                          v
                                  UnityFS/raw files
                                          |
                                          v
                                  SerializedFile
                                          |
                            +-------------+-------------+
                            |                           |
                       Texture2D                   TextAsset
                            |                           |
                       inline/.resS                      |
                            |                           |
                       texture decode                   |
                            |                           |
                            +-------------+-------------+
                                          |
                                          v
                                   exported objects
                                          |
                               +----------+----------+
                               |                     |
                         _common atlas          other assets
                               |
                     CommonSpriteData crop
                               |
                               +----------+----------+
                                          |
                                          v
                               BLAKE3 asset store
                                          |
                              +-----------+-----------+
                              |                       |
                           SQLite                  Axum wiki
```

## Native Unity extraction

The extraction backend is MSL-focused and has explicit failure boundaries.

### UnityFS container parsing

The container layer:

- supports uncompressed, LZ4/LZ4HC, and LZMA blocks;
- validates compressed/decompressed ranges and declared sizes;
- handles block-info-at-end and block-alignment flags;
- rejects path traversal and extraction collisions;
- preserves non-bundle Unity files under `build/extracted/raw`.

### SerializedFile parsing

The semantic layer targets modern Unity SerializedFile v22+ files, including the supplied Unity `6000.3.12f1` build. It parses:

- the extended v22 header;
- metadata endianness;
- Unity version and target platform;
- serialized type records;
- blob TypeTrees and their local/common string references;
- aligned object tables and path IDs;
- object byte ranges with overflow/bounds checks.

Editor-only object layouts are rejected rather than guessed.

### Texture2D decoding

The Texture2D reader follows the modern Unity 6 layout, including post-2023.2 `Texture` base fields, conditional `m_MipmapLimitGroupName`, conditional `m_PlatformBlob`, and 64-bit streamed-resource offsets.

The conversion layer uses pure-Rust decoding for the Android-relevant compression families:

- ASTC;
- ETC1/ETC2 and EAC;
- ATC;
- BC1/2/3/4/5/6/7;
- PVRTC;
- Crunch and Unity Crunch;
- common uncompressed channel layouts.

Texture decode errors are isolated to an object and recorded in `build/assets/unity-objects.json`; they do not discard successfully decoded neighboring objects.

### Streamed resource resolution

External texture payloads are resolved by resource basename with bundle-local files preferred over global matches. Ambiguous resource names fail explicitly. Offset + size ranges are checked against the resource file before allocating or reading.

### MSL common atlas

After semantic export, the pipeline chooses the largest decoded `Texture2D` whose `m_Name` is `_common`. If `build/assets/common-sprites.json` exists, `extract-assets` feeds that PNG directly into the existing Rust cropper and indexes the resulting sprite files.

This semantic name check replaces the need to hardcode the current hashed atlas bundle filename. The earlier dependency scanner remains useful as evidence/debugging when a future game update changes the asset graph.

## Output/manifests

The extraction layers preserve source provenance rather than flattening everything into one directory:

```text
build/extracted/
├── bundles/<source-bundle>/<bundle-entry>
├── raw/<standalone-unity-file>
└── objects/
    ├── textures/*.png
    └── text/*
```

Manifest roles:

- `asset-candidates.json` — byte-level dependency evidence from the merged Unity view;
- `unity-objects.json` — semantic object exports plus per-object failures;
- `extracted-manifest.json` — content-addressed index of extracted files;
- `sprites-manifest.json` — logical CommonSprite crop records;
- `sprites-store-manifest.json` — content-addressed index of cropped sprites.

## Game-data plan

`client_gamedata.pb` and `server_gamedata.pb` are treated as versioned source data. Once the game's actual protobuf tags and message relationships are recovered, generated `prost` structures belong in a source-format crate and are translated into stable domain structs before entering SQLite.

With native Unity asset access in-tree, the remaining extraction work is primarily schema/data interpretation and asset-to-entity mapping rather than dependence on an external Unity ripper.
