use anyhow::Result;
use msl_core::ImportManifest;
use serde::Serialize;
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::path::Path;

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportRecord {
    pub id: i64,
    pub package_name: String,
    pub name: String,
    pub version_code: String,
    pub version_name: String,
    pub source_hash: String,
    pub file_count: i64,
    pub imported_at: String,
}

impl Database {
    pub async fn connect(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }

        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;

        let database = Self { pool };
        database.ensure_schema().await?;
        Ok(database)
    }

    async fn ensure_schema(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS imports (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                package_name TEXT NOT NULL,
                name TEXT NOT NULL,
                version_code TEXT NOT NULL,
                version_name TEXT NOT NULL,
                source_path TEXT NOT NULL,
                source_hash TEXT NOT NULL UNIQUE,
                file_count INTEGER NOT NULL,
                imported_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS staged_files (
                import_id INTEGER NOT NULL,
                source_apk TEXT NOT NULL,
                archive_path TEXT NOT NULL,
                staged_path TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                size INTEGER NOT NULL,
                kind TEXT NOT NULL,
                PRIMARY KEY (import_id, source_apk, archive_path),
                FOREIGN KEY (import_id) REFERENCES imports(id) ON DELETE CASCADE
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn record_import(&self, manifest: &ImportManifest) -> Result<i64> {
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            r#"
            INSERT INTO imports (
                package_name, name, version_code, version_name,
                source_path, source_hash, file_count
            ) VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(source_hash) DO UPDATE SET
                source_path = excluded.source_path,
                file_count = excluded.file_count,
                imported_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(&manifest.package.package_name)
        .bind(&manifest.package.name)
        .bind(&manifest.package.version_code)
        .bind(&manifest.package.version_name)
        .bind(&manifest.source_path)
        .bind(&manifest.source_hash)
        .bind(manifest.file_count() as i64)
        .execute(&mut *tx)
        .await?;

        let import_id: i64 = sqlx::query_scalar("SELECT id FROM imports WHERE source_hash = ?")
            .bind(&manifest.source_hash)
            .fetch_one(&mut *tx)
            .await?;

        sqlx::query("DELETE FROM staged_files WHERE import_id = ?")
            .bind(import_id)
            .execute(&mut *tx)
            .await?;

        for file in &manifest.files {
            sqlx::query(
                r#"
                INSERT INTO staged_files (
                    import_id, source_apk, archive_path, staged_path,
                    content_hash, size, kind
                ) VALUES (?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(import_id)
            .bind(&file.source_apk)
            .bind(&file.archive_path)
            .bind(&file.staged_path)
            .bind(&file.content_hash)
            .bind(file.size as i64)
            .bind(file.kind.as_str())
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(import_id)
    }

    pub async fn recent_imports(&self, limit: i64) -> Result<Vec<ImportRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, package_name, name, version_code, version_name,
                   source_hash, file_count, imported_at
            FROM imports
            ORDER BY imported_at DESC, id DESC
            LIMIT ?
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| ImportRecord {
                id: row.get("id"),
                package_name: row.get("package_name"),
                name: row.get("name"),
                version_code: row.get("version_code"),
                version_name: row.get("version_name"),
                source_hash: row.get("source_hash"),
                file_count: row.get("file_count"),
                imported_at: row.get("imported_at"),
            })
            .collect())
    }
}
