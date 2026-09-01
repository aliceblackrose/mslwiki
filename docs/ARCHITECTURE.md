# Architecture

`mslwiki` is split into four layers:

1. **Ingest**: accepts XAPK/APK inputs and stages only files relevant to Unity, IL2CPP metadata, streaming assets, and game data.
2. **Domain**: stable Rust types owned by this project. Game-internal protobuf/Unity structures must be translated into these types.
3. **Storage**: normalized SQLite database and asset manifest.
4. **Presentation**: Axum HTML pages and JSON API.

Raw extracted game files are build inputs and are not committed to Git.
