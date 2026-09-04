use anyhow::Result;
use msl_core::ImportManifest;
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CatalogSnapshot {
    #[serde(default)]
    pub astromons: Vec<AstromonRecord>,
    #[serde(default)]
    pub skills: Vec<SkillRecord>,
    #[serde(default)]
    pub items: Vec<ItemRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AstromonRecord {
    pub id: i64,
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub element: String,
    #[serde(default)]
    pub rarity: Option<i64>,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub hp: Option<i64>,
    #[serde(default)]
    pub attack: Option<i64>,
    #[serde(default)]
    pub defense: Option<i64>,
    #[serde(default)]
    pub recovery: Option<i64>,
    #[serde(default)]
    pub skill_one_id: Option<i64>,
    #[serde(default)]
    pub skill_two_id: Option<i64>,
    #[serde(default)]
    pub leader_skill_id: Option<i64>,
    #[serde(default)]
    pub portrait_asset: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRecord {
    pub id: i64,
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub skill_type: Option<String>,
    #[serde(default)]
    pub element: Option<String>,
    #[serde(default)]
    pub cooldown: Option<i64>,
    #[serde(default)]
    pub icon_asset: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemRecord {
    pub id: i64,
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub item_type: Option<String>,
    #[serde(default)]
    pub rarity: Option<i64>,
    #[serde(default)]
    pub icon_asset: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogCounts {
    pub astromons: i64,
    pub skills: i64,
    pub items: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub kind: String,
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub subtitle: String,
    pub asset_path: Option<String>,
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

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS skills (
                id INTEGER PRIMARY KEY,
                slug TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                skill_type TEXT,
                element TEXT,
                cooldown INTEGER,
                icon_asset TEXT,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS astromons (
                id INTEGER PRIMARY KEY,
                slug TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                element TEXT NOT NULL DEFAULT '',
                rarity INTEGER,
                variant TEXT,
                role TEXT,
                hp INTEGER,
                attack INTEGER,
                defense INTEGER,
                recovery INTEGER,
                skill_one_id INTEGER,
                skill_two_id INTEGER,
                leader_skill_id INTEGER,
                portrait_asset TEXT,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (skill_one_id) REFERENCES skills(id),
                FOREIGN KEY (skill_two_id) REFERENCES skills(id),
                FOREIGN KEY (leader_skill_id) REFERENCES skills(id)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS items (
                id INTEGER PRIMARY KEY,
                slug TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                item_type TEXT,
                rarity INTEGER,
                icon_asset TEXT,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        for statement in [
            "CREATE INDEX IF NOT EXISTS idx_astromons_name ON astromons(name COLLATE NOCASE)",
            "CREATE INDEX IF NOT EXISTS idx_astromons_element ON astromons(element COLLATE NOCASE)",
            "CREATE INDEX IF NOT EXISTS idx_astromons_rarity ON astromons(rarity)",
            "CREATE INDEX IF NOT EXISTS idx_skills_name ON skills(name COLLATE NOCASE)",
            "CREATE INDEX IF NOT EXISTS idx_items_name ON items(name COLLATE NOCASE)",
        ] {
            sqlx::query(statement).execute(&self.pool).await?;
        }

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

    pub async fn import_catalog(&self, catalog: &CatalogSnapshot) -> Result<CatalogCounts> {
        let mut tx = self.pool.begin().await?;

        for skill in &catalog.skills {
            sqlx::query(
                r#"
                INSERT INTO skills (
                    id, slug, name, description, skill_type, element, cooldown, icon_asset
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(id) DO UPDATE SET
                    slug = excluded.slug,
                    name = excluded.name,
                    description = excluded.description,
                    skill_type = excluded.skill_type,
                    element = excluded.element,
                    cooldown = excluded.cooldown,
                    icon_asset = excluded.icon_asset,
                    updated_at = CURRENT_TIMESTAMP
                "#,
            )
            .bind(skill.id)
            .bind(&skill.slug)
            .bind(&skill.name)
            .bind(&skill.description)
            .bind(&skill.skill_type)
            .bind(&skill.element)
            .bind(skill.cooldown)
            .bind(&skill.icon_asset)
            .execute(&mut *tx)
            .await?;
        }

        for astromon in &catalog.astromons {
            sqlx::query(
                r#"
                INSERT INTO astromons (
                    id, slug, name, element, rarity, variant, role,
                    hp, attack, defense, recovery,
                    skill_one_id, skill_two_id, leader_skill_id, portrait_asset
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(id) DO UPDATE SET
                    slug = excluded.slug,
                    name = excluded.name,
                    element = excluded.element,
                    rarity = excluded.rarity,
                    variant = excluded.variant,
                    role = excluded.role,
                    hp = excluded.hp,
                    attack = excluded.attack,
                    defense = excluded.defense,
                    recovery = excluded.recovery,
                    skill_one_id = excluded.skill_one_id,
                    skill_two_id = excluded.skill_two_id,
                    leader_skill_id = excluded.leader_skill_id,
                    portrait_asset = excluded.portrait_asset,
                    updated_at = CURRENT_TIMESTAMP
                "#,
            )
            .bind(astromon.id)
            .bind(&astromon.slug)
            .bind(&astromon.name)
            .bind(&astromon.element)
            .bind(astromon.rarity)
            .bind(&astromon.variant)
            .bind(&astromon.role)
            .bind(astromon.hp)
            .bind(astromon.attack)
            .bind(astromon.defense)
            .bind(astromon.recovery)
            .bind(astromon.skill_one_id)
            .bind(astromon.skill_two_id)
            .bind(astromon.leader_skill_id)
            .bind(&astromon.portrait_asset)
            .execute(&mut *tx)
            .await?;
        }

        for item in &catalog.items {
            sqlx::query(
                r#"
                INSERT INTO items (id, slug, name, description, item_type, rarity, icon_asset)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(id) DO UPDATE SET
                    slug = excluded.slug,
                    name = excluded.name,
                    description = excluded.description,
                    item_type = excluded.item_type,
                    rarity = excluded.rarity,
                    icon_asset = excluded.icon_asset,
                    updated_at = CURRENT_TIMESTAMP
                "#,
            )
            .bind(item.id)
            .bind(&item.slug)
            .bind(&item.name)
            .bind(&item.description)
            .bind(&item.item_type)
            .bind(item.rarity)
            .bind(&item.icon_asset)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        self.catalog_counts().await
    }

    pub async fn catalog_counts(&self) -> Result<CatalogCounts> {
        Ok(CatalogCounts {
            astromons: sqlx::query_scalar("SELECT COUNT(*) FROM astromons")
                .fetch_one(&self.pool)
                .await?,
            skills: sqlx::query_scalar("SELECT COUNT(*) FROM skills")
                .fetch_one(&self.pool)
                .await?,
            items: sqlx::query_scalar("SELECT COUNT(*) FROM items")
                .fetch_one(&self.pool)
                .await?,
        })
    }

    pub async fn astromons(
        &self,
        query: &str,
        element: &str,
        rarity: Option<i64>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<AstromonRecord>> {
        let rarity = rarity.unwrap_or(-1);
        let rows = sqlx::query(
            r#"
            SELECT id, slug, name, element, rarity, variant, role,
                   hp, attack, defense, recovery,
                   skill_one_id, skill_two_id, leader_skill_id, portrait_asset
            FROM astromons
            WHERE (
                ? = '' OR name LIKE '%' || ? || '%' COLLATE NOCASE
                OR slug LIKE '%' || ? || '%' COLLATE NOCASE
                OR CAST(id AS TEXT) = ?
            )
              AND (? = '' OR element = ? COLLATE NOCASE)
              AND (? < 0 OR rarity = ?)
            ORDER BY COALESCE(rarity, 0) DESC, name COLLATE NOCASE ASC, id ASC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(query)
        .bind(query)
        .bind(query)
        .bind(query)
        .bind(element)
        .bind(element)
        .bind(rarity)
        .bind(rarity)
        .bind(limit.clamp(1, 500))
        .bind(offset.max(0))
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(astromon_from_row).collect())
    }

    pub async fn astromon(&self, slug_or_id: &str) -> Result<Option<AstromonRecord>> {
        let row = sqlx::query(
            r#"
            SELECT id, slug, name, element, rarity, variant, role,
                   hp, attack, defense, recovery,
                   skill_one_id, skill_two_id, leader_skill_id, portrait_asset
            FROM astromons
            WHERE slug = ? COLLATE NOCASE OR CAST(id AS TEXT) = ?
            LIMIT 1
            "#,
        )
        .bind(slug_or_id)
        .bind(slug_or_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(astromon_from_row))
    }

    pub async fn skills(&self, query: &str, limit: i64, offset: i64) -> Result<Vec<SkillRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, slug, name, description, skill_type, element, cooldown, icon_asset
            FROM skills
            WHERE ? = ''
               OR name LIKE '%' || ? || '%' COLLATE NOCASE
               OR slug LIKE '%' || ? || '%' COLLATE NOCASE
               OR description LIKE '%' || ? || '%' COLLATE NOCASE
               OR CAST(id AS TEXT) = ?
            ORDER BY name COLLATE NOCASE ASC, id ASC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(query)
        .bind(query)
        .bind(query)
        .bind(query)
        .bind(query)
        .bind(limit.clamp(1, 500))
        .bind(offset.max(0))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(skill_from_row).collect())
    }

    pub async fn skill(&self, slug_or_id: &str) -> Result<Option<SkillRecord>> {
        let row = sqlx::query(
            r#"
            SELECT id, slug, name, description, skill_type, element, cooldown, icon_asset
            FROM skills
            WHERE slug = ? COLLATE NOCASE OR CAST(id AS TEXT) = ?
            LIMIT 1
            "#,
        )
        .bind(slug_or_id)
        .bind(slug_or_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(skill_from_row))
    }

    pub async fn items(&self, query: &str, limit: i64, offset: i64) -> Result<Vec<ItemRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, slug, name, description, item_type, rarity, icon_asset
            FROM items
            WHERE ? = ''
               OR name LIKE '%' || ? || '%' COLLATE NOCASE
               OR slug LIKE '%' || ? || '%' COLLATE NOCASE
               OR description LIKE '%' || ? || '%' COLLATE NOCASE
               OR CAST(id AS TEXT) = ?
            ORDER BY name COLLATE NOCASE ASC, id ASC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(query)
        .bind(query)
        .bind(query)
        .bind(query)
        .bind(query)
        .bind(limit.clamp(1, 500))
        .bind(offset.max(0))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(item_from_row).collect())
    }

    pub async fn item(&self, slug_or_id: &str) -> Result<Option<ItemRecord>> {
        let row = sqlx::query(
            r#"
            SELECT id, slug, name, description, item_type, rarity, icon_asset
            FROM items
            WHERE slug = ? COLLATE NOCASE OR CAST(id AS TEXT) = ?
            LIMIT 1
            "#,
        )
        .bind(slug_or_id)
        .bind(slug_or_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(item_from_row))
    }

    pub async fn search_catalog(
        &self,
        query: &str,
        kind: Option<&str>,
        limit: i64,
    ) -> Result<Vec<SearchResult>> {
        let limit = limit.clamp(1, 100);
        let mut results = Vec::new();
        let kind = kind.unwrap_or("").to_ascii_lowercase();

        if kind.is_empty() || kind == "astromon" || kind == "astromons" {
            for value in self.astromons(query, "", None, limit, 0).await? {
                let subtitle = [
                    (!value.element.is_empty()).then_some(value.element.clone()),
                    value.rarity.map(|rarity| format!("{rarity}★")),
                    value.role.clone(),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
                results.push(SearchResult {
                    kind: "astromon".to_owned(),
                    id: value.id,
                    slug: value.slug,
                    name: value.name,
                    subtitle,
                    asset_path: value.portrait_asset,
                });
            }
        }

        if kind.is_empty() || kind == "skill" || kind == "skills" {
            for value in self.skills(query, limit, 0).await? {
                let subtitle = value
                    .skill_type
                    .clone()
                    .or(value.element.clone())
                    .unwrap_or_else(|| "Skill".to_owned());
                results.push(SearchResult {
                    kind: "skill".to_owned(),
                    id: value.id,
                    slug: value.slug,
                    name: value.name,
                    subtitle,
                    asset_path: value.icon_asset,
                });
            }
        }

        if kind.is_empty() || kind == "item" || kind == "items" {
            for value in self.items(query, limit, 0).await? {
                let subtitle = value.item_type.clone().unwrap_or_else(|| "Item".to_owned());
                results.push(SearchResult {
                    kind: "item".to_owned(),
                    id: value.id,
                    slug: value.slug,
                    name: value.name,
                    subtitle,
                    asset_path: value.icon_asset,
                });
            }
        }

        results.sort_by(|left, right| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.id.cmp(&right.id))
        });
        results.truncate(limit as usize);
        Ok(results)
    }
}

fn astromon_from_row(row: sqlx::sqlite::SqliteRow) -> AstromonRecord {
    AstromonRecord {
        id: row.get("id"),
        slug: row.get("slug"),
        name: row.get("name"),
        element: row.get("element"),
        rarity: row.get("rarity"),
        variant: row.get("variant"),
        role: row.get("role"),
        hp: row.get("hp"),
        attack: row.get("attack"),
        defense: row.get("defense"),
        recovery: row.get("recovery"),
        skill_one_id: row.get("skill_one_id"),
        skill_two_id: row.get("skill_two_id"),
        leader_skill_id: row.get("leader_skill_id"),
        portrait_asset: row.get("portrait_asset"),
    }
}

fn skill_from_row(row: sqlx::sqlite::SqliteRow) -> SkillRecord {
    SkillRecord {
        id: row.get("id"),
        slug: row.get("slug"),
        name: row.get("name"),
        description: row.get("description"),
        skill_type: row.get("skill_type"),
        element: row.get("element"),
        cooldown: row.get("cooldown"),
        icon_asset: row.get("icon_asset"),
    }
}

fn item_from_row(row: sqlx::sqlite::SqliteRow) -> ItemRecord {
    ItemRecord {
        id: row.get("id"),
        slug: row.get("slug"),
        name: row.get("name"),
        description: row.get("description"),
        item_type: row.get("item_type"),
        rarity: row.get("rarity"),
        icon_asset: row.get("icon_asset"),
    }
}
