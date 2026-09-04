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
   -> normalized Astromon/skill/item catalog
   -> SQLite
   -> searchable Axum wiki + JSON API
```

## What works now

The importer, asset pipeline, and wiki currently:

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
11. validate UnityFS ranges, block metadata, output collisions, and archive paths before writing files;
12. parse modern Unity SerializedFile v22+ metadata, type tables, TypeTrees, and object tables;
13. decode MSL's Unity 6 `Texture2D` layout, including inline and external `.resS` image data;
14. export supported textures directly to PNG with pure-Rust decoders for ASTC, ETC/EAC, ATC, BCn, PVRTC, Crunch, and common raw formats;
15. export `TextAsset` objects as text or binary data;
16. record per-object decode failures in `build/assets/unity-objects.json` without discarding successful exports;
17. choose the largest decoded texture named `_common` as the MSL common atlas;
18. automatically crop every `CommonSpriteData` rectangle from that atlas when the sprite catalog is available;
19. hash extracted/cropped assets and deduplicate identical files into a content-addressed BLAKE3 store;
20. emit manifests mapping original paths, Unity objects, and sprite names to stable stored objects;
21. record package imports and staged-file inventories in SQLite;
22. store normalized Astromon, skill, and item records in SQLite;
23. search Astromons by name/ID, element, and rarity;
24. search skills and items by name/ID/text;
25. serve responsive Astromon, skill, item, and global-search pages;
26. expose the same catalog through a JSON API.

Relevant package inputs include:

- `assets/bin/Data/**`
- `assets/bin/Data/Managed/Metadata/global-metadata.dat`
- `lib/arm64-v8a/libil2cpp.so`
- `assets/CommonSpriteData.txt`
- `assets/data_list.txt`
- embedded `client_gamedata.pb` / `server_gamedata.pb` when present

The supplied `1.0.260722032` XAPK contains 347 staged files, 344 unique Unity `Data` paths, and 627 common sprite records.

The dependency scanner identifies `8e2205a495a484cf3ae6ba8a2970aaca` as the supplied build's strong `_common` atlas candidate. This is derived from file references and `_MainTex` evidence rather than hardcoded into the importer. Semantic extraction additionally identifies the atlas from its decoded `Texture2D.m_Name` value.

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

That command performs the MSL asset path end-to-end:

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

## Wiki catalog

The website reads a normalized catalog from SQLite. The stable interchange format is a JSON object with `astromons`, `skills`, and `items` arrays.

A minimal example:

```json
{
  "astromons": [
    {
      "id": 101,
      "slug": "fire-mona",
      "name": "Mona",
      "element": "Fire",
      "rarity": 4,
      "variant": "Variant",
      "role": "Attacker",
      "hp": 1200,
      "attack": 930,
      "defense": 610,
      "recovery": 720,
      "skill_one_id": 201,
      "portrait_asset": "assets/sprites/mona.png"
    }
  ],
  "skills": [
    {
      "id": 201,
      "slug": "flame-strike",
      "name": "Flame Strike",
      "description": "Deals fire damage.",
      "skill_type": "Active",
      "element": "Fire",
      "cooldown": 3,
      "icon_asset": "assets/sprites/flame-strike.png"
    }
  ],
  "items": []
}
```

Asset paths are relative to `build/`. For example, `assets/sprites/mona.png` is served from `build/assets/sprites/mona.png`.

Load a normalized catalog into SQLite with:

```bash
cargo run -p mslwiki -- load-catalog build/gamedata/catalog.json
```

The game-data decoder is intended to emit this same contract, so the web/database layer does not need to change when automatic protobuf decoding is completed.

## Run the wiki

```bash
cargo run -p mslwiki -- serve
```

Then open `http://127.0.0.1:3000`.

Pages include:

- `/` — wiki home and global search;
- `/search?q=...` — search Astromons, skills, and items;
- `/astromons` — Astromon catalog with name/ID, element, and rarity filters;
- `/astromons/<slug-or-id>` — Astromon details, stats, and skill links;
- `/skills` and `/skills/<slug-or-id>`;
- `/items` and `/items/<slug-or-id>`.

JSON API endpoints include:

- `GET /health`
- `GET /api/v1/imports`
- `GET /api/v1/search?q=...&kind=astromon|skill|item`
- `GET /api/v1/astromons?q=...&element=...&rarity=...&page=...`
- `GET /api/v1/astromons/<slug-or-id>`
- `GET /api/v1/skills?q=...&page=...`
- `GET /api/v1/skills/<slug-or-id>`
- `GET /api/v1/items?q=...&page=...`
- `GET /api/v1/items/<slug-or-id>`

## Remaining data work

The website and normalized storage/search contract are implemented. The remaining MSL-specific work is getting all live game records into that contract automatically:

1. validate the native Unity decoder against the current real package and inspect entries in `unity-objects.json`;
2. continue protobuf field/tag recovery for the important `Msg*` types;
3. decode `client_gamedata.pb` / `server_gamedata.pb` into normalized Astromon/skill/item records;
4. associate game entity IDs with exported image assets;
5. extend the same catalog model to dungeons, quests, evolution chains, gems, and update history.

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).
