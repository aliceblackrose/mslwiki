# Architecture

`mslwiki` keeps source-format reverse engineering out of the website layer.

## Layers

### `msl-ingest`

Accepts XAPK inputs, reads `manifest.json`, opens nested split APKs, and stages only files relevant to Unity, IL2CPP, sprite metadata, and game data. The original split APK name is preserved in the staging path so duplicate Unity paths never silently overwrite one another.

It also creates the deterministic merged Unity `Data` view. A conflicting duplicate relative path is an error rather than an implicit overwrite.

### `msl-assets`

Owns MSL/Unity asset metadata processing after staging. It currently parses `CommonSpriteData.txt` and exports a normalized JSON catalog. The extraction backend and web asset conversion will live here rather than in the server.

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
  +-- ABI split APK -> libil2cpp.so -> staged split inventory
  |                                       |
  +-- Unity asset packs -> Data ----------+
  |                                       |
  +-- CommonSpriteData.txt ---------------+
                                          v
                                merged Unity Data view
                                          |
                              +-----------+-----------+
                              |                       |
                        Unity extraction        sprite catalog
                              |                       |
                              +-----------+-----------+
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

The Unity extraction integration is an adapter, not a dependency of the website. Rust owns orchestration and post-processing:

1. Resolve the correct Unity data files from split APKs.
2. Build a collision-checked merged Unity `Data` directory.
3. Run the configured Unity extraction backend.
4. Hash every exported object with BLAKE3.
5. Normalize images to web-friendly output.
6. Use `CommonSpriteData.txt` to crop known atlas regions.
7. Store logical asset keys separately from physical content-addressed files.

The supplied build has 344 unique Unity `Data` paths with no collisions. `CommonSpriteData.txt` contains 627 sprite records; `_common` sprite names are referenced from hashed Unity files, which gives us the next link to resolve to the underlying atlas texture.

## Game-data plan

`client_gamedata.pb` and `server_gamedata.pb` are treated as versioned source data. Once the game's actual protobuf tags and message relationships are recovered, generated `prost` structures belong in a source-format crate and are translated into stable domain structs before entering SQLite.
