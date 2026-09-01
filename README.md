# MSL Wiki

A data-driven Rust wiki and extraction pipeline for **Monster Super League**.

The project treats game packages as build inputs:

```text
XAPK/APK
   -> relevant Unity + IL2CPP staging
   -> deterministic Unity Data view
   -> asset/game-data extraction
   -> normalized SQLite
   -> Axum wiki + JSON API
```

## Current milestone

The importer now:

1. reads the XAPK metadata;
2. opens nested split APKs without holding an entire split in RAM;
3. stages only Unity, IL2CPP, sprite, and game-data inputs;
4. records source split + BLAKE3 hash for every staged file;
5. creates a deterministic merged Unity `Data` directory;
6. parses `CommonSpriteData.txt` into a normalized sprite catalog;
7. records the import in SQLite.

Relevant inputs include:

- `assets/bin/Data/**`
- `assets/bin/Data/Managed/Metadata/global-metadata.dat`
- `lib/arm64-v8a/libil2cpp.so`
- `assets/CommonSpriteData.txt`
- `assets/data_list.txt`
- embedded `client_gamedata.pb` / `server_gamedata.pb` when present

The supplied `1.0.260722032` XAPK contains 347 staged files, 344 unique Unity `Data` paths, and 627 common sprite records.

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
│   └── common-sprites.json
├── unity/
│   └── Data/
└── stage/
    ├── import-manifest.json
    ├── source/manifest.json
    └── raw/<split-apk>/...
```

The merged `build/unity/Data` directory is the input for the Unity asset extraction backend.

## Run the wiki

```bash
cargo run -p mslwiki -- serve
```

Then open `http://127.0.0.1:3000`.

API endpoints currently include:

- `GET /health`
- `GET /api/v1/imports`

## Next extraction stages

1. Add an AssetRipper adapter and content-addressed extracted-asset manifest.
2. Resolve the `_common` sprite atlas and crop the 627 catalogued sprite regions.
3. Recover the real protobuf schema from IL2CPP metadata.
4. Decode `client_gamedata.pb` / `server_gamedata.pb` into stable `msl-core` domain types.
5. Generate Astromon, skill, item, dungeon, and update pages from normalized data.

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).
