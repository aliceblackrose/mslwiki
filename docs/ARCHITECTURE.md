# Architecture

`mslwiki` keeps source-format reverse engineering out of the website layer.

## Layers

### `msl-ingest`

Accepts XAPK inputs, reads `manifest.json`, opens nested split APKs, and stages only files relevant to Unity, IL2CPP, sprite metadata, and game data. The original split APK name is preserved in the staging path so duplicate Unity paths never silently overwrite one another.

### `msl-core`

Stable project-owned domain types. Future protobuf-generated types and Unity-specific structures must be translated into these types before storage or presentation.

### `msl-db`

SQLite persistence for imported game versions and staged-file inventory. This becomes the normalized Astromon/skill/item/localization database as decoders are added.

### `msl-web`

Axum HTML and JSON API. It only consumes normalized database data; it does not parse APKs, protobuf messages, or Unity files.

### `msl-cli`

Composition root for commands such as `import` and `serve`.

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
  +-- ABI split APK -> libil2cpp.so -> staging manifest
  |                                       |
  +-- Unity asset packs -> Data ----------+
  |                                       |
  +-- StreamingAssets metadata -----------+
                                          v
                                    normalized data
                                          |
                              +-----------+-----------+
                              |                       |
                           SQLite                asset store
                              |                       |
                              +-----------+-----------+
                                          v
                                       Axum wiki
```

## Asset extraction plan

The first AssetRipper integration should be an adapter, not a hard dependency in website code. Rust owns orchestration and post-processing:

1. Resolve the correct Unity data files from split APKs.
2. Run the configured Unity extraction backend.
3. Hash every exported object with BLAKE3.
4. Normalize images to web-friendly output.
5. Parse `CommonSpriteData.txt` and crop known atlas regions.
6. Store logical asset keys separately from physical content-addressed files.

This lets an unchanged texture be reused across multiple game releases while game-version metadata remains independent.

## Game-data plan

`client_gamedata.pb` and `server_gamedata.pb` are treated as versioned source data. Once the game's actual protobuf tags and message relationships are recovered, generated `prost` structures belong in a source-format crate and are translated into stable domain structs before entering SQLite.
