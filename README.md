# MSL Wiki

A data-driven Rust wiki and extraction pipeline for **Monster Super League**.

The project treats game packages as build inputs:

```text
XAPK/APK
   -> relevant Unity + IL2CPP staging
   -> asset/game-data extraction
   -> normalized SQLite
   -> Axum wiki + JSON API
```

## Current milestone

The initial importer inventories and extracts the files needed for later Unity/IL2CPP processing while preserving the split APK they came from. Every staged file is content-hashed and recorded in SQLite.

Relevant inputs currently include:

- `assets/bin/Data/**`
- `assets/bin/Data/Managed/Metadata/global-metadata.dat`
- `lib/arm64-v8a/libil2cpp.so`
- `assets/CommonSpriteData.txt`
- `assets/data_list.txt`
- embedded `client_gamedata.pb` / `server_gamedata.pb` when present

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
└── stage/
    ├── import-manifest.json
    ├── source/manifest.json
    └── raw/<split-apk>/...
```

## Run the wiki

```bash
cargo run -p mslwiki -- serve
```

Then open `http://127.0.0.1:3000`.

API endpoints currently include:

- `GET /health`
- `GET /api/v1/imports`

## Next extraction stages

1. Build a deterministic merged Unity `Data` view from staged split APKs.
2. Add an AssetRipper adapter and content-addressed asset manifest.
3. Parse `CommonSpriteData.txt` and export atlas sprites.
4. Recover the real protobuf schema from IL2CPP metadata.
5. Decode `client_gamedata.pb` / `server_gamedata.pb` into stable `msl-core` domain types.
6. Generate Astromon, skill, item, dungeon, and update pages from normalized data.

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).
