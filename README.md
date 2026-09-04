# MSL Wiki

A data-driven Rust wiki and extraction pipeline for **Monster Super League**.

The project treats game packages as build inputs:

```text
XAPK/APK
   -> relevant Unity + IL2CPP staging
   -> deterministic Unity Data view
   -> sprite/dependency discovery
   -> metadata/type inventory
   -> native Rust UnityFS extraction
   -> Rust sprite cropping
   -> content-addressed asset store
   -> normalized SQLite
   -> Axum wiki + JSON API
```

## What works now

The importer and asset pipeline currently:

1. read XAPK metadata and game version information;
2. open nested split APKs without holding an entire split in RAM;
3. stage only Unity, IL2CPP, sprite, and game-data inputs;
4. record source split + BLAKE3 hash for every staged file;
5. create a deterministic merged Unity `Data` directory with collision detection;
6. parse `CommonSpriteData.txt` into a normalized sprite catalog;
7. scan Unity files for sprite names and cross-file dependencies;
8. follow `_MainTex` evidence to identify likely large `_common` atlas files;
9. inventory `Msg*`, manager, protobuf, monster, skill, item, and gamedata-related strings from `global-metadata.dat`;
10. parse UnityFS bundles directly in Rust, including uncompressed, LZ4/LZ4HC, and LZMA blocks;
11. safely unpack bundle directory entries while preserving standalone Unity files for later serialized-object decoding;
12. crop `CommonSpriteData` rectangles from an atlas PNG entirely in Rust;
13. validate every crop against atlas bounds and support top-left or bottom-left origins;
14. hash extracted/cropped assets and deduplicate identical files into a content-addressed BLAKE3 store;
15. emit manifests mapping original paths and sprite names to stable stored objects;
16. record the package import and staged-file inventory in SQLite;
17. serve a minimal Axum wiki/API process.

Relevant inputs include:

- `assets/bin/Data/**`
- `assets/bin/Data/Managed/Metadata/global-metadata.dat`
- `lib/arm64-v8a/libil2cpp.so`
- `assets/CommonSpriteData.txt`
- `assets/data_list.txt`
- embedded `client_gamedata.pb` / `server_gamedata.pb` when present

The supplied `1.0.260722032` XAPK contains 347 staged files, 344 unique Unity `Data` paths, and 627 common sprite records.

The dependency scanner identifies `8e2205a495a484cf3ae6ba8a2970aaca` as the current build's strong `_common` atlas candidate. This is derived from file references and `_MainTex` evidence rather than hardcoded into the importer.

Raw APK/XAPK files and extracted game assets are intentionally ignored by Git.

## Build

```bash
cargo build --workspace
```

## Import an XAPK

```bash
cargo run -p mslwiki -- import "/path/to/Monster Super League.xapk"
```

This writes:

```text
build/
├── msl.sqlite
├── assets/
│   ├── common-sprites.json
│   └── asset-candidates.json
├── gamedata/
│   └── metadata-inventory.json
├── unity/
│   └── Data/
└── stage/
    ├── import-manifest.json
    ├── source/manifest.json
    └── raw/<split-apk>/...
```

## Re-run asset discovery

```bash
cargo run -p mslwiki -- inspect-assets
```

## Re-run IL2CPP metadata inventory

```bash
cargo run -p mslwiki -- inspect-metadata
```

This creates `build/gamedata/metadata-inventory.json` with candidate protobuf/message and manager type names that can be targeted by the schema recovery stage.

## Extract and index Unity content

No external AssetRipper executable is required. The built-in Rust extractor reads the merged Unity `Data` directory directly:

```bash
cargo run -p mslwiki -- extract-assets
```

The extractor currently owns the UnityFS container layer. It validates bundle bounds, handles block-info-at-end and alignment flags, decompresses uncompressed/LZ4/LZ4HC/LZMA blocks, rejects unsafe bundle paths, and writes:

```text
build/extracted/
├── bundles/<source-bundle>/<bundle-entry>
└── raw/<standalone-unity-file>
```

Every output is then indexed into the BLAKE3 content-addressed store. Serialized Unity object decoding is intentionally a separate layer; Texture2D-to-PNG export is the next native extraction stage rather than another external tool dependency.

If content has already been extracted, indexing can be run independently:

```bash
cargo run -p mslwiki -- index-assets
```

## Crop the common sprite atlas

Once the `_common` Texture2D decoder produces an atlas PNG:

```bash
cargo run -p mslwiki -- crop-sprites \
  --atlas "/path/to/_common.png"
```

The command reads `build/assets/common-sprites.json`, validates every rectangle against the atlas dimensions, writes individual PNG files under `build/assets/sprites`, creates `sprites-manifest.json`, and indexes the cropped images into the shared BLAKE3 store.

If the source atlas uses a bottom-left coordinate origin:

```bash
cargo run -p mslwiki -- crop-sprites \
  --atlas "/path/to/_common.png" \
  --origin bottom-left
```

## Run the wiki

```bash
cargo run -p mslwiki -- serve
```

Then open `http://127.0.0.1:3000`.

API endpoints currently include:

- `GET /health`
- `GET /api/v1/imports`

The site is intentionally minimal at this stage: Astromon/skill/item pages need the game-data schema and asset-to-entity mapping before they can be generated correctly.

## Next extraction stages

1. Validate the native UnityFS backend against the real Unity `6000.3.12f1` `_common` candidate.
2. Add serialized-file/type-tree decoding for the MSL object classes we need first (`Texture2D`, `Sprite`, `TextAsset`, and `MonoBehaviour`).
3. Decode the `_common` Texture2D, including external `.resS` stream data, directly to PNG.
4. Run the Rust cropper against the real 627-sprite atlas and classify the output.
5. Expand metadata inventory into field/tag recovery for the most important `Msg*` types.
6. Decode `client_gamedata.pb` / `server_gamedata.pb` into stable `msl-core` domain types.
7. Associate Astromon/skill/item IDs with extracted image assets.
8. Generate Astromon, skill, item, dungeon, and update pages from normalized data.

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).
