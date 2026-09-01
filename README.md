# MSL Wiki

A data-driven Rust wiki and extraction pipeline for **Monster Super League**.

The project treats game packages as build inputs:

```text
XAPK/APK
   -> relevant Unity + IL2CPP staging
   -> deterministic Unity Data view
   -> sprite/dependency discovery
   -> Unity primary-content extraction
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
9. invoke a CLI-compatible AssetRipper build in primary/raw extraction mode;
10. hash AssetRipper output and deduplicate identical files into a content-addressed BLAKE3 store;
11. emit an extracted-asset manifest mapping original AssetRipper paths to stable stored objects;
12. record the package import and staged-file inventory in SQLite;
13. serve a minimal Axum wiki/API process.

Relevant inputs include:

- `assets/bin/Data/**`
- `assets/bin/Data/Managed/Metadata/global-metadata.dat`
- `lib/arm64-v8a/libil2cpp.so`
- `assets/CommonSpriteData.txt`
- `assets/data_list.txt`
- embedded `client_gamedata.pb` / `server_gamedata.pb` when present

The supplied `1.0.260722032` XAPK contains 347 staged files, 344 unique Unity `Data` paths, and 627 common sprite records.

The dependency scanner identifies `8e2205a495a484cf3ae6ba8a2970aaca` as the current build's large `_common` atlas candidate. This is derived from file references and `_MainTex` evidence rather than hardcoded into the importer.

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
├── unity/
│   └── Data/
└── stage/
    ├── import-manifest.json
    ├── source/manifest.json
    └── raw/<split-apk>/...
```

`asset-candidates.json` records sprite-name matches, `_common`/`_MainTex` markers, and references between merged Unity files.

## Re-run asset discovery

```bash
cargo run -p mslwiki -- inspect-assets
```

This scans the already merged `build/unity/Data` tree without unpacking the XAPK again.

## Extract and index Unity primary content

Point `--asset-ripper` at a CLI-compatible AssetRipper executable:

```bash
cargo run -p mslwiki -- extract-assets \
  --asset-ripper "/path/to/AssetRipper"
```

The adapter passes the merged Unity data directory as input, requests primary/raw content, disables script import, and writes into `build/extracted` by default. It then indexes every extracted file into:

```text
build/assets/
├── extracted-manifest.json
└── store/
    ├── 00/
    ├── 01/
    ├── ...
    └── ff/
```

Objects are addressed by BLAKE3 hash, so duplicate or unchanged assets share one physical stored file. The extraction command refuses to overwrite a non-empty raw extraction directory.

If primary content has already been extracted, indexing can be run independently:

```bash
cargo run -p mslwiki -- index-assets
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

1. Export the `_common` atlas and crop the 627 catalogued sprite regions.
2. Classify extracted portraits, skill icons, item icons, backgrounds, audio, and UI assets.
3. Recover the real protobuf schema from IL2CPP metadata.
4. Decode `client_gamedata.pb` / `server_gamedata.pb` into stable `msl-core` domain types.
5. Associate Astromon/skill/item IDs with extracted image assets.
6. Generate Astromon, skill, item, dungeon, and update pages from normalized data.

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).
