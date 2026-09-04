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
   -> native SerializedFile/Object decoding
   -> Texture2D/TextAsset export
   -> automatic _common atlas sprite cropping
   -> content-addressed asset store
   -> normalized SQLite
   -> Axum wiki + JSON API
```

## What works now

The importer and asset pipeline currently:

1. reads XAPK metadata and game version information;
2. opens nested split APKs without holding an entire split in RAM;
3. stages only Unity, IL2CPP, sprite, and game-data inputs;
4. records source split + BLAKE3 hash for every staged file;
5. creates a deterministic merged Unity `Data` directory with collision detection;
6. parses `CommonSpriteData.txt` into a normalized sprite catalog;
7. scans Unity files for sprite names and cross-file dependencies;
8. follows `_MainTex` evidence to identify likely large `_common` atlas files;
9. inventories `Msg*`, manager, protobuf, monster, skill, item, and gamedata-related strings from `global-metadata.dat`;
10. parses UnityFS bundles directly in Rust, including uncompressed, LZ4/LZ4HC, and LZMA blocks;
11. validates UnityFS ranges, block metadata, output collisions, and archive paths before writing files;
12. parses modern Unity SerializedFile v22+ metadata, type tables, TypeTrees, and object tables;
13. decodes MSL's Unity 6 `Texture2D` layout, including inline and external `.resS` image data;
14. exports supported textures directly to PNG with pure-Rust decoders for ASTC, ETC/EAC, ATC, BCn, PVRTC, Crunch, and common raw formats;
15. exports `TextAsset` objects as text or binary data;
16. records per-object decode failures in `build/assets/unity-objects.json` without discarding successful exports;
17. chooses the largest decoded texture named `_common` as the MSL common atlas;
18. automatically crops every `CommonSpriteData` rectangle from that atlas when the sprite catalog is available;
19. hashes extracted/cropped assets and deduplicates identical files into a content-addressed BLAKE3 store;
20. emits manifests mapping original paths, Unity objects, and sprite names to stable stored objects;
21. records the package import and staged-file inventory in SQLite;
22. serves a minimal Axum wiki/API process.

Relevant inputs include:

- `assets/bin/Data/**`
- `assets/bin/Data/Managed/Metadata/global-metadata.dat`
- `lib/arm64-v8a/libil2cpp.so`
- `assets/CommonSpriteData.txt`
- `assets/data_list.txt`
- embedded `client_gamedata.pb` / `server_gamedata.pb` when present

The supplied `1.0.260722032` XAPK contains 347 staged files, 344 unique Unity `Data` paths, and 627 common sprite records.

The dependency scanner identifies `8e2205a495a484cf3ae6ba8a2970aaca` as the supplied build's strong `_common` atlas candidate. This is derived from file references and `_MainTex` evidence rather than hardcoded into the importer. Semantic extraction now additionally identifies the atlas from its decoded `Texture2D.m_Name` value.

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

## Extract Unity assets

No AssetRipper executable or other external Unity extraction program is required:

```bash
cargo run -p mslwiki -- extract-assets
```

That single command performs the MSL asset path end-to-end:

```text
build/unity/Data
  -> UnityFS unpacking
  -> SerializedFile parsing
  -> Texture2D/TextAsset object decoding
  -> .resS stream resolution
  -> Texture2D PNG export
  -> _common atlas selection
  -> CommonSpriteData cropping
  -> BLAKE3 content-addressed indexing
```

Primary outputs are:

```text
build/
├── extracted/
│   ├── bundles/<source-bundle>/<bundle-entry>
│   ├── raw/<standalone-unity-file>
│   └── objects/
│       ├── textures/*.png
│       └── text/*
└── assets/
    ├── unity-objects.json
    ├── extracted-manifest.json
    ├── sprites/
    ├── sprites-manifest.json
    ├── sprites-store-manifest.json
    └── store/
```

`unity-objects.json` is also the diagnostic surface for unsupported or malformed individual objects. A failed texture decode is recorded there while other valid objects continue exporting.

The default automatic sprite crop uses the same top-left origin as the standalone `crop-sprites` command. If the current game build's atlas metadata uses bottom-left coordinates, run extraction with:

```bash
cargo run -p mslwiki -- extract-assets --sprite-origin bottom-left
```

If content has already been extracted and only the content-addressed store needs rebuilding:

```bash
cargo run -p mslwiki -- index-assets
```

## Crop a sprite atlas manually

The standalone cropper remains available for debugging or non-default atlases:

```bash
cargo run -p mslwiki -- crop-sprites \
  --atlas "/path/to/atlas.png"
```

To use bottom-left coordinates:

```bash
cargo run -p mslwiki -- crop-sprites \
  --atlas "/path/to/atlas.png" \
  --origin bottom-left
```

## Native extractor scope

The native extractor is deliberately **MSL-focused**, not a clone of every AssetRipper feature. The supported semantic object classes currently needed by this repository are:

- `Texture2D` — decoded to PNG;
- `TextAsset` — exported as text/binary data.

Unity container and SerializedFile parsing are generic enough to discover other object classes, but unsupported classes are left untouched rather than approximated. Add another object decoder when MSL data actually requires it.

The current SerializedFile parser targets modern Unity format v22+ and the supplied Unity `6000.3.12f1` build. It rejects editor-only layouts and invalid ranges instead of attempting unsafe recovery.

## Run the wiki

```bash
cargo run -p mslwiki -- serve
```

Then open `http://127.0.0.1:3000`.

API endpoints currently include:

- `GET /health`
- `GET /api/v1/imports`

The site is intentionally minimal at this stage: Astromon/skill/item pages need the game-data schema and asset-to-entity mapping before they can be generated correctly.

## Next data stages

Asset extraction no longer depends on an external Unity ripper. The remaining project work is data interpretation rather than basic Unity asset access:

1. validate the native decoder against the real supplied Unity `6000.3.12f1` package and inspect any entries in `unity-objects.json`;
2. expand metadata inventory into field/tag recovery for the most important `Msg*` types;
3. decode `client_gamedata.pb` / `server_gamedata.pb` into stable `msl-core` domain types;
4. associate Astromon/skill/item IDs with exported image assets;
5. generate Astromon, skill, item, dungeon, and update pages from normalized data.

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).
