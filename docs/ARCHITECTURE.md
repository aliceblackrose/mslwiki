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
2. **serialized-file layer** — parse Unity serialized-file headers, object tables, type trees, and external resource references;
3. **object layer** — decode only the object classes MSL needs first (`Texture2D`, `Sprite`, `TextAsset`, and selected `MonoBehaviour` payloads);
4. **conversion layer** — emit deterministic PNG/JSON output for the rest of the pipeline.

The first layer is implemented in-tree and does not invoke AssetRipper or another extraction executable.

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
                              +-----------+-----------+
                              |                       |
                    native Unity extraction      sprite catalog
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

The Unity extraction backend lives inside `msl-assets`; the website never depends on Unity parsing details.

1. Resolve the correct Unity data files from split APKs.
2. Build a collision-checked merged Unity `Data` directory.
3. Detect and unpack UnityFS bundles with the native Rust container parser.
4. Preserve standalone serialized/resource files so no source data is discarded before object decoding exists.
5. Parse serialized files and type trees for the object classes required by MSL.
6. Resolve external resource streams such as `.resS` texture payloads.
7. Decode supported textures and normalize images to PNG.
8. Hash every exported object with BLAKE3.
9. Use `CommonSpriteData.txt` to crop known atlas regions.
10. Store logical asset keys separately from physical content-addressed files.

The UnityFS container parser supports uncompressed, LZ4/LZ4HC, and LZMA blocks, validates ranges and decompressed sizes, handles block-info-at-end/alignment flags, and rejects bundle paths that could escape the extraction root.

The supplied build has 344 unique Unity `Data` paths with no collisions. `CommonSpriteData.txt` contains 627 sprite records; `_common` sprite names are referenced from hashed Unity files, which gives us the next link to resolve to the underlying atlas texture.

## Game-data plan

`client_gamedata.pb` and `server_gamedata.pb` are treated as versioned source data. Once the game's actual protobuf tags and message relationships are recovered, generated `prost` structures belong in a source-format crate and are translated into stable domain structs before entering SQLite.
