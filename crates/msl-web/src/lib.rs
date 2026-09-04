use anyhow::Result;
use axum::{
    Json, Router,
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use msl_db::{AstromonRecord, Database, ImportRecord, ItemRecord, SearchResult, SkillRecord};
use serde::Deserialize;
use std::{
    net::SocketAddr,
    path::{Component, Path as FilePath, PathBuf},
    sync::Arc,
};

const PAGE_SIZE: i64 = 48;

#[derive(Clone)]
struct AppState {
    database: Database,
    media_root: Arc<PathBuf>,
}

#[derive(Debug, Deserialize, Default)]
struct SearchQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    kind: String,
}

#[derive(Debug, Deserialize, Default)]
struct AstromonQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    element: String,
    rarity: Option<i64>,
    page: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
struct ListQuery {
    #[serde(default)]
    q: String,
    page: Option<i64>,
}

pub fn router(database: Database) -> Router {
    router_with_media(database, PathBuf::from("build"))
}

pub fn router_with_media(database: Database, media_root: PathBuf) -> Router {
    let state = AppState {
        database,
        media_root: Arc::new(media_root),
    };

    Router::new()
        .route("/", get(home))
        .route("/search", get(search_page))
        .route("/astromons", get(astromon_index))
        .route("/astromons/{slug}", get(astromon_detail))
        .route("/skills", get(skill_index))
        .route("/skills/{slug}", get(skill_detail))
        .route("/items", get(item_index))
        .route("/items/{slug}", get(item_detail))
        .route("/assets/site.css", get(site_css))
        .route("/media/{*path}", get(media))
        .route("/health", get(health))
        .route("/api/v1/imports", get(imports))
        .route("/api/v1/search", get(api_search))
        .route("/api/v1/astromons", get(api_astromons))
        .route("/api/v1/astromons/{slug}", get(api_astromon))
        .route("/api/v1/skills", get(api_skills))
        .route("/api/v1/skills/{slug}", get(api_skill))
        .route("/api/v1/items", get(api_items))
        .route("/api/v1/items/{slug}", get(api_item))
        .with_state(state)
}

pub async fn serve(database: Database, address: SocketAddr) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, router(database)).await?;
    Ok(())
}

async fn health() -> &'static str {
    "ok"
}

async fn imports(State(state): State<AppState>) -> WebResult<Json<Vec<ImportRecord>>> {
    state
        .database
        .recent_imports(20)
        .await
        .map(Json)
        .map_err(internal_error)
}

async fn api_search(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> WebResult<Json<Vec<SearchResult>>> {
    let kind = (!query.kind.trim().is_empty()).then_some(query.kind.trim());
    state
        .database
        .search_catalog(query.q.trim(), kind, 100)
        .await
        .map(Json)
        .map_err(internal_error)
}

async fn api_astromons(
    State(state): State<AppState>,
    Query(query): Query<AstromonQuery>,
) -> WebResult<Json<Vec<AstromonRecord>>> {
    let page = query.page.unwrap_or(1).max(1);
    state
        .database
        .astromons(
            query.q.trim(),
            query.element.trim(),
            query.rarity,
            PAGE_SIZE,
            (page - 1) * PAGE_SIZE,
        )
        .await
        .map(Json)
        .map_err(internal_error)
}

async fn api_astromon(
    State(state): State<AppState>,
    AxumPath(slug): AxumPath<String>,
) -> WebResult<Response> {
    match state
        .database
        .astromon(&slug)
        .await
        .map_err(internal_error)?
    {
        Some(value) => Ok(Json(value).into_response()),
        None => Ok(StatusCode::NOT_FOUND.into_response()),
    }
}

async fn api_skills(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> WebResult<Json<Vec<SkillRecord>>> {
    let page = query.page.unwrap_or(1).max(1);
    state
        .database
        .skills(query.q.trim(), PAGE_SIZE, (page - 1) * PAGE_SIZE)
        .await
        .map(Json)
        .map_err(internal_error)
}

async fn api_skill(
    State(state): State<AppState>,
    AxumPath(slug): AxumPath<String>,
) -> WebResult<Response> {
    match state.database.skill(&slug).await.map_err(internal_error)? {
        Some(value) => Ok(Json(value).into_response()),
        None => Ok(StatusCode::NOT_FOUND.into_response()),
    }
}

async fn api_items(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> WebResult<Json<Vec<ItemRecord>>> {
    let page = query.page.unwrap_or(1).max(1);
    state
        .database
        .items(query.q.trim(), PAGE_SIZE, (page - 1) * PAGE_SIZE)
        .await
        .map(Json)
        .map_err(internal_error)
}

async fn api_item(
    State(state): State<AppState>,
    AxumPath(slug): AxumPath<String>,
) -> WebResult<Response> {
    match state.database.item(&slug).await.map_err(internal_error)? {
        Some(value) => Ok(Json(value).into_response()),
        None => Ok(StatusCode::NOT_FOUND.into_response()),
    }
}

async fn home(State(state): State<AppState>) -> WebResult<Html<String>> {
    let counts = state
        .database
        .catalog_counts()
        .await
        .map_err(internal_error)?;
    let featured = state
        .database
        .astromons("", "", None, 12, 0)
        .await
        .map_err(internal_error)?;
    let imports = state.database.recent_imports(5).await.unwrap_or_default();

    let featured_html = if featured.is_empty() {
        empty_catalog_panel()
    } else {
        format!(
            "<div class=\"entity-grid\">{}</div>",
            featured
                .iter()
                .map(render_astromon_card)
                .collect::<String>()
        )
    };

    let imports_html = if imports.is_empty() {
        "<p class=\"muted\">No package imports recorded yet.</p>".to_owned()
    } else {
        imports
            .iter()
            .map(|import| {
                format!(
                    "<div class=\"import-row\"><span>{}</span><strong>{}</strong><span>{} files</span></div>",
                    escape_html(&import.name),
                    escape_html(&import.version_name),
                    import.file_count
                )
            })
            .collect::<String>()
    };

    let body = format!(
        r#"
<section class="hero">
  <div class="eyebrow">Monster Super League data wiki</div>
  <h1>Find any Astromon.<br><span>Know what it does.</span></h1>
  <p>Search Astromons, skills, and items from the game data extracted by this project.</p>
  <form class="hero-search" action="/search" method="get">
    <input name="q" aria-label="Search wiki" placeholder="Search by name or game ID…" autocomplete="off">
    <button type="submit">Search</button>
  </form>
  <div class="count-strip">
    <a href="/astromons"><strong>{}</strong><span>Astromons</span></a>
    <a href="/skills"><strong>{}</strong><span>Skills</span></a>
    <a href="/items"><strong>{}</strong><span>Items</span></a>
  </div>
</section>
<section class="section">
  <div class="section-heading"><div><span class="eyebrow">Browse</span><h2>Astromon catalog</h2></div><a class="text-link" href="/astromons">View all →</a></div>
  {}
</section>
<section class="section compact-section">
  <div class="section-heading"><div><span class="eyebrow">Pipeline</span><h2>Recent game imports</h2></div></div>
  <div class="import-list">{}</div>
</section>
"#,
        counts.astromons, counts.skills, counts.items, featured_html, imports_html
    );
    Ok(Html(layout("MSL Wiki", &body)))
}

async fn search_page(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> WebResult<Html<String>> {
    let trimmed = query.q.trim();
    let kind = (!query.kind.trim().is_empty()).then_some(query.kind.trim());
    let results = if trimmed.is_empty() {
        Vec::new()
    } else {
        state
            .database
            .search_catalog(trimmed, kind, 100)
            .await
            .map_err(internal_error)?
    };

    let results_html = if trimmed.is_empty() {
        "<div class=\"empty-state\"><h3>Start with a name or ID</h3><p>Try an Astromon, skill, or item.</p></div>".to_owned()
    } else if results.is_empty() {
        format!(
            "<div class=\"empty-state\"><h3>No results for “{}”</h3><p>The catalog may not be loaded yet, or the name may differ from the in-game data.</p></div>",
            escape_html(trimmed)
        )
    } else {
        format!(
            "<div class=\"search-results\">{}</div>",
            results.iter().map(render_search_result).collect::<String>()
        )
    };

    let body = format!(
        r#"
<section class="page-head">
  <span class="eyebrow">Global search</span>
  <h1>Search the wiki</h1>
  <form class="hero-search inline-search" action="/search" method="get">
    <input name="q" value="{}" placeholder="Astromon, skill, item, or ID…" autofocus>
    <select name="kind" aria-label="Content type">
      <option value="" {}>Everything</option>
      <option value="astromon" {}>Astromons</option>
      <option value="skill" {}>Skills</option>
      <option value="item" {}>Items</option>
    </select>
    <button type="submit">Search</button>
  </form>
</section>
<section class="section search-section">{}</section>
"#,
        escape_html(trimmed),
        selected(query.kind.is_empty()),
        selected(query.kind.eq_ignore_ascii_case("astromon")),
        selected(query.kind.eq_ignore_ascii_case("skill")),
        selected(query.kind.eq_ignore_ascii_case("item")),
        results_html
    );
    Ok(Html(layout("Search · MSL Wiki", &body)))
}

async fn astromon_index(
    State(state): State<AppState>,
    Query(query): Query<AstromonQuery>,
) -> WebResult<Html<String>> {
    let page = query.page.unwrap_or(1).max(1);
    let values = state
        .database
        .astromons(
            query.q.trim(),
            query.element.trim(),
            query.rarity,
            PAGE_SIZE,
            (page - 1) * PAGE_SIZE,
        )
        .await
        .map_err(internal_error)?;
    let rarity_filter = query
        .rarity
        .map(|value| value.to_string())
        .unwrap_or_default();

    let cards = if values.is_empty() {
        empty_catalog_panel()
    } else {
        format!(
            "<div class=\"entity-grid\">{}</div>{}",
            values.iter().map(render_astromon_card).collect::<String>(),
            pagination(
                "/astromons",
                page,
                values.len() as i64 == PAGE_SIZE,
                &[
                    ("q", query.q.as_str()),
                    ("element", query.element.as_str()),
                    ("rarity", rarity_filter.as_str()),
                ]
            )
        )
    };

    let body = format!(
        r#"
<section class="page-head">
  <span class="eyebrow">Bestiary</span>
  <h1>Astromons</h1>
  <p>Browse the normalized Astromon catalog or narrow it by element, rarity, name, or game ID.</p>
</section>
<section class="filter-bar">
<form action="/astromons" method="get">
  <input name="q" value="{}" placeholder="Name or ID…">
  <select name="element">
    <option value="" {}>All elements</option>
    {}
  </select>
  <select name="rarity">
    <option value="" {}>All rarities</option>
    {}
  </select>
  <button type="submit">Filter</button>
  <a class="ghost-button" href="/astromons">Clear</a>
</form>
</section>
<section class="section catalog-section">{}</section>
"#,
        escape_html(&query.q),
        selected(query.element.is_empty()),
        element_options(&query.element),
        selected(query.rarity.is_none()),
        rarity_options(query.rarity),
        cards
    );
    Ok(Html(layout("Astromons · MSL Wiki", &body)))
}

async fn astromon_detail(
    State(state): State<AppState>,
    AxumPath(slug): AxumPath<String>,
) -> WebResult<Response> {
    let Some(value) = state
        .database
        .astromon(&slug)
        .await
        .map_err(internal_error)?
    else {
        return Ok(not_found("Astromon"));
    };

    let rarity = value
        .rarity
        .map(|number| {
            format!(
                "{} <span class=\"stars\">{}</span>",
                number,
                "★".repeat(number.clamp(0, 6) as usize)
            )
        })
        .unwrap_or_else(|| "Unknown".to_owned());
    let variant = value.variant.as_deref().unwrap_or("—");
    let role = value.role.as_deref().unwrap_or("—");
    let media = render_large_media(value.portrait_asset.as_deref(), &value.name);
    let skills = [
        ("Active skill", value.skill_one_id),
        ("Passive skill", value.skill_two_id),
        ("Leader skill", value.leader_skill_id),
    ]
    .into_iter()
    .filter_map(|(label, id)| id.map(|id| (label, id)))
    .map(|(label, id)| {
        format!(
            "<a class=\"detail-link\" href=\"/skills/{id}\"><span>{label}</span><strong>Skill #{id}</strong><b>→</b></a>"
        )
    })
    .collect::<String>();
    let skills = if skills.is_empty() {
        "<p class=\"muted\">Skill relationships have not been mapped for this record yet.</p>"
            .to_owned()
    } else {
        skills
    };

    let body = format!(
        r#"
<section class="detail-hero">
  <div class="detail-media">{}</div>
  <div class="detail-copy">
    <div class="detail-kicker"><span class="element-pill {}">{}</span><span>#{} </span></div>
    <h1>{}</h1>
    <p class="detail-subtitle">{} · {} · rarity {}</p>
    <div class="stat-grid">
      {}
      {}
      {}
      {}
    </div>
  </div>
</section>
<section class="detail-columns">
  <article class="panel"><span class="eyebrow">Identity</span><h2>Profile</h2><dl class="facts"><div><dt>Game ID</dt><dd>{}</dd></div><div><dt>Element</dt><dd>{}</dd></div><div><dt>Variant</dt><dd>{}</dd></div><div><dt>Role</dt><dd>{}</dd></div></dl></article>
  <article class="panel"><span class="eyebrow">Kit</span><h2>Skills</h2><div class="detail-links">{}</div></article>
</section>
"#,
        media,
        element_class(&value.element),
        escape_html(if value.element.is_empty() {
            "Unknown"
        } else {
            &value.element
        }),
        value.id,
        escape_html(&value.name),
        escape_html(variant),
        escape_html(role),
        rarity,
        stat_card("HP", value.hp),
        stat_card("ATK", value.attack),
        stat_card("DEF", value.defense),
        stat_card("REC", value.recovery),
        value.id,
        escape_html(if value.element.is_empty() {
            "Unknown"
        } else {
            &value.element
        }),
        escape_html(variant),
        escape_html(role),
        skills
    );
    Ok(Html(layout(&format!("{} · MSL Wiki", value.name), &body)).into_response())
}

async fn skill_index(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> WebResult<Html<String>> {
    let page = query.page.unwrap_or(1).max(1);
    let values = state
        .database
        .skills(query.q.trim(), PAGE_SIZE, (page - 1) * PAGE_SIZE)
        .await
        .map_err(internal_error)?;
    let content = if values.is_empty() {
        empty_catalog_panel()
    } else {
        format!(
            "<div class=\"list-grid\">{}</div>{}",
            values.iter().map(render_skill_card).collect::<String>(),
            pagination(
                "/skills",
                page,
                values.len() as i64 == PAGE_SIZE,
                &[("q", query.q.as_str())]
            )
        )
    };
    let body = list_page_header("Skill library", "Skills", &query.q, "/skills", &content);
    Ok(Html(layout("Skills · MSL Wiki", &body)))
}

async fn skill_detail(
    State(state): State<AppState>,
    AxumPath(slug): AxumPath<String>,
) -> WebResult<Response> {
    let Some(value) = state.database.skill(&slug).await.map_err(internal_error)? else {
        return Ok(not_found("Skill"));
    };
    let body = format!(
        r#"
<section class="simple-detail">
  <div>{}</div>
  <div>
    <div class="detail-kicker"><span class="tag">Skill</span><span>#{} </span></div>
    <h1>{}</h1>
    <p class="lead">{}</p>
    <dl class="facts wide-facts"><div><dt>Type</dt><dd>{}</dd></div><div><dt>Element</dt><dd>{}</dd></div><div><dt>Cooldown</dt><dd>{}</dd></div></dl>
  </div>
</section>
"#,
        render_large_media(value.icon_asset.as_deref(), &value.name),
        value.id,
        escape_html(&value.name),
        escape_html(if value.description.is_empty() {
            "No description decoded yet."
        } else {
            &value.description
        }),
        option_text(value.skill_type.as_deref()),
        option_text(value.element.as_deref()),
        value
            .cooldown
            .map(|value| value.to_string())
            .unwrap_or_else(|| "—".to_owned())
    );
    Ok(Html(layout(&format!("{} · MSL Wiki", value.name), &body)).into_response())
}

async fn item_index(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> WebResult<Html<String>> {
    let page = query.page.unwrap_or(1).max(1);
    let values = state
        .database
        .items(query.q.trim(), PAGE_SIZE, (page - 1) * PAGE_SIZE)
        .await
        .map_err(internal_error)?;
    let content = if values.is_empty() {
        empty_catalog_panel()
    } else {
        format!(
            "<div class=\"list-grid\">{}</div>{}",
            values.iter().map(render_item_card).collect::<String>(),
            pagination(
                "/items",
                page,
                values.len() as i64 == PAGE_SIZE,
                &[("q", query.q.as_str())]
            )
        )
    };
    let body = list_page_header("Inventory", "Items", &query.q, "/items", &content);
    Ok(Html(layout("Items · MSL Wiki", &body)))
}

async fn item_detail(
    State(state): State<AppState>,
    AxumPath(slug): AxumPath<String>,
) -> WebResult<Response> {
    let Some(value) = state.database.item(&slug).await.map_err(internal_error)? else {
        return Ok(not_found("Item"));
    };
    let body = format!(
        r#"
<section class="simple-detail">
  <div>{}</div>
  <div>
    <div class="detail-kicker"><span class="tag">Item</span><span>#{} </span></div>
    <h1>{}</h1>
    <p class="lead">{}</p>
    <dl class="facts wide-facts"><div><dt>Type</dt><dd>{}</dd></div><div><dt>Rarity</dt><dd>{}</dd></div></dl>
  </div>
</section>
"#,
        render_large_media(value.icon_asset.as_deref(), &value.name),
        value.id,
        escape_html(&value.name),
        escape_html(if value.description.is_empty() {
            "No description decoded yet."
        } else {
            &value.description
        }),
        option_text(value.item_type.as_deref()),
        value
            .rarity
            .map(|value| format!("{value}★"))
            .unwrap_or_else(|| "—".to_owned())
    );
    Ok(Html(layout(&format!("{} · MSL Wiki", value.name), &body)).into_response())
}

async fn site_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        SITE_CSS,
    )
}

async fn media(State(state): State<AppState>, AxumPath(path): AxumPath<String>) -> Response {
    let relative = FilePath::new(&path);
    if relative.as_os_str().is_empty()
        || !relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let full_path = state.media_root.join(relative);
    let Ok(bytes) = std::fs::read(&full_path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let content_type = match full_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "json" => "application/json",
        _ => "application/octet-stream",
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "public, max-age=3600")
        .body(Body::from(bytes))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

type WebResult<T> = std::result::Result<T, (StatusCode, String)>;

fn internal_error(error: anyhow::Error) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

fn not_found(kind: &str) -> Response {
    let body = format!(
        "<section class=\"empty-state page-empty\"><h1>{} not found</h1><p>The requested catalog record does not exist.</p><a class=\"primary-button\" href=\"/\">Back home</a></section>",
        escape_html(kind)
    );
    (
        StatusCode::NOT_FOUND,
        Html(layout("Not found · MSL Wiki", &body)),
    )
        .into_response()
}

fn layout(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="dark">
<title>{}</title>
<link rel="stylesheet" href="/assets/site.css">
</head>
<body>
<header class="site-header">
  <a class="brand" href="/"><span class="brand-mark">M</span><span>MSL <b>Wiki</b></span></a>
  <nav><a href="/astromons">Astromons</a><a href="/skills">Skills</a><a href="/items">Items</a></nav>
  <form class="nav-search" action="/search" method="get"><input name="q" aria-label="Quick search" placeholder="Search…"></form>
</header>
<main>{}</main>
<footer><div><strong>MSL Wiki</strong><span>Data extracted locally from Monster Super League builds.</span></div><a href="/api/v1/search?q=">JSON API</a></footer>
</body>
</html>"#,
        escape_html(title),
        body
    )
}

fn empty_catalog_panel() -> String {
    r#"<div class="empty-state">
<h3>The wiki shell is ready; the catalog is empty.</h3>
<p>The extraction pipeline will populate these records as the protobuf game-data decoder lands. The database schema and public import API are ready now.</p>
</div>"#
        .to_owned()
}

fn render_astromon_card(value: &AstromonRecord) -> String {
    let rarity = value
        .rarity
        .map(|value| "★".repeat(value.clamp(0, 6) as usize))
        .unwrap_or_default();
    format!(
        r#"<a class="entity-card" href="/astromons/{}">
<div class="entity-media {}">{}</div>
<div class="entity-copy"><div class="entity-meta"><span>{}</span><span class="stars">{}</span></div><h3>{}</h3><p>{}</p></div>
</a>"#,
        escape_html(&value.slug),
        element_class(&value.element),
        render_card_media(value.portrait_asset.as_deref(), &value.name),
        escape_html(if value.element.is_empty() {
            "Unknown"
        } else {
            &value.element
        }),
        rarity,
        escape_html(&value.name),
        escape_html(
            value
                .role
                .as_deref()
                .or(value.variant.as_deref())
                .unwrap_or("Astromon")
        )
    )
}

fn render_skill_card(value: &SkillRecord) -> String {
    format!(
        r#"<a class="list-card" href="/skills/{}"><div class="small-media">{}</div><div><span class="tag">{}</span><h3>{}</h3><p>{}</p></div><b>→</b></a>"#,
        escape_html(&value.slug),
        render_card_media(value.icon_asset.as_deref(), &value.name),
        escape_html(value.skill_type.as_deref().unwrap_or("Skill")),
        escape_html(&value.name),
        escape_html(summary(&value.description, 100))
    )
}

fn render_item_card(value: &ItemRecord) -> String {
    format!(
        r#"<a class="list-card" href="/items/{}"><div class="small-media">{}</div><div><span class="tag">{}</span><h3>{}</h3><p>{}</p></div><b>→</b></a>"#,
        escape_html(&value.slug),
        render_card_media(value.icon_asset.as_deref(), &value.name),
        escape_html(value.item_type.as_deref().unwrap_or("Item")),
        escape_html(&value.name),
        escape_html(summary(&value.description, 100))
    )
}

fn render_search_result(value: &SearchResult) -> String {
    let href = match value.kind.as_str() {
        "skill" => format!("/skills/{}", value.slug),
        "item" => format!("/items/{}", value.slug),
        _ => format!("/astromons/{}", value.slug),
    };
    format!(
        r#"<a class="search-result" href="{}"><div class="small-media">{}</div><div><span class="tag">{}</span><h3>{}</h3><p>{}</p></div><span class="result-id">#{}</span></a>"#,
        escape_html(&href),
        render_card_media(value.asset_path.as_deref(), &value.name),
        escape_html(&value.kind),
        escape_html(&value.name),
        escape_html(&value.subtitle),
        value.id
    )
}

fn render_card_media(path: Option<&str>, name: &str) -> String {
    match path.filter(|value| !value.trim().is_empty()) {
        Some(path) => format!(
            "<img src=\"{}\" alt=\"{}\" loading=\"lazy\">",
            escape_html(&media_url(path)),
            escape_html(name)
        ),
        None => format!("<span>{}</span>", escape_html(initials(name))),
    }
}

fn render_large_media(path: Option<&str>, name: &str) -> String {
    match path.filter(|value| !value.trim().is_empty()) {
        Some(path) => format!(
            "<img class=\"large-image\" src=\"{}\" alt=\"{}\">",
            escape_html(&media_url(path)),
            escape_html(name)
        ),
        None => format!(
            "<div class=\"large-placeholder\"><span>{}</span><small>Artwork not mapped</small></div>",
            escape_html(initials(name))
        ),
    }
}

fn media_url(path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") || path.starts_with('/') {
        path.to_owned()
    } else {
        format!(
            "/media/{}",
            path.replace('\\', "/").trim_start_matches("./")
        )
    }
}

fn stat_card(label: &str, value: Option<i64>) -> String {
    format!(
        "<div class=\"stat-card\"><span>{}</span><strong>{}</strong></div>",
        escape_html(label),
        value
            .map(|value| value.to_string())
            .unwrap_or_else(|| "—".to_owned())
    )
}

fn list_page_header(kicker: &str, title: &str, query: &str, action: &str, content: &str) -> String {
    format!(
        r#"<section class="page-head"><span class="eyebrow">{}</span><h1>{}</h1><p>Search the normalized game-data catalog by name or game ID.</p></section>
<section class="filter-bar"><form action="{}" method="get"><input name="q" value="{}" placeholder="Name or ID…"><button type="submit">Search</button>{}</form></section>
<section class="section catalog-section">{}</section>"#,
        escape_html(kicker),
        escape_html(title),
        escape_html(action),
        escape_html(query),
        if query.is_empty() {
            String::new()
        } else {
            format!(
                "<a class=\"ghost-button\" href=\"{}\">Clear</a>",
                escape_html(action)
            )
        },
        content
    )
}

fn element_options(current: &str) -> String {
    ["Fire", "Water", "Wood", "Light", "Dark"]
        .into_iter()
        .map(|element| {
            format!(
                "<option value=\"{}\" {}>{}</option>",
                element,
                selected(current.eq_ignore_ascii_case(element)),
                element
            )
        })
        .collect()
}

fn rarity_options(current: Option<i64>) -> String {
    (1..=6)
        .map(|rarity| {
            format!(
                "<option value=\"{rarity}\" {}>{rarity}★</option>",
                selected(current == Some(rarity))
            )
        })
        .collect()
}

fn pagination(base: &str, page: i64, has_next: bool, params: &[(&str, &str)]) -> String {
    if page <= 1 && !has_next {
        return String::new();
    }
    let query = params
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(key, value)| format!("{}={}", key, url_component(value)))
        .collect::<Vec<_>>();
    let href = |target_page: i64| {
        let mut all = query.clone();
        if target_page > 1 {
            all.push(format!("page={target_page}"));
        }
        if all.is_empty() {
            base.to_owned()
        } else {
            format!("{base}?{}", all.join("&"))
        }
    };

    format!(
        "<nav class=\"pagination\">{}<span>Page {page}</span>{}</nav>",
        if page > 1 {
            format!(
                "<a href=\"{}\">← Previous</a>",
                escape_html(&href(page - 1))
            )
        } else {
            "<span></span>".to_owned()
        },
        if has_next {
            format!("<a href=\"{}\">Next →</a>", escape_html(&href(page + 1)))
        } else {
            "<span></span>".to_owned()
        }
    )
}

fn element_class(value: &str) -> String {
    let normalized = value
        .bytes()
        .filter(|byte| byte.is_ascii_alphanumeric())
        .map(|byte| byte.to_ascii_lowercase() as char)
        .collect::<String>();
    if normalized.is_empty() {
        "element-unknown".to_owned()
    } else {
        format!("element-{normalized}")
    }
}

fn selected(condition: bool) -> &'static str {
    if condition { "selected" } else { "" }
}

fn option_text(value: Option<&str>) -> String {
    escape_html(value.filter(|value| !value.is_empty()).unwrap_or("—"))
}

fn initials(value: &str) -> &str {
    value.trim().get(..1).unwrap_or("?")
}

fn summary(value: &str, maximum: usize) -> &str {
    let value = value.trim();
    if value.is_empty() {
        return "No description decoded yet.";
    }
    if value.len() <= maximum {
        return value;
    }
    let mut end = maximum;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn url_component(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                output.push(byte as char);
            }
            b' ' => output.push('+'),
            _ => output.push_str(&format!("%{byte:02X}")),
        }
    }
    output
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

const SITE_CSS: &str = r#"
:root{color-scheme:dark;--bg:#0a0c14;--panel:#111522;--panel2:#171c2d;--line:#272d42;--text:#f7f8ff;--muted:#9ba5bd;--accent:#9f7aea;--accent2:#5eead4;--shadow:0 18px 55px rgba(0,0,0,.28)}*{box-sizing:border-box}html{scroll-behavior:smooth}body{margin:0;background:radial-gradient(circle at 20% -10%,rgba(159,122,234,.18),transparent 35%),var(--bg);color:var(--text);font:15px/1.55 Inter,ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}a{color:inherit;text-decoration:none}button,input,select{font:inherit}.site-header{height:72px;display:flex;align-items:center;gap:36px;padding:0 max(24px,calc((100vw - 1180px)/2));border-bottom:1px solid rgba(255,255,255,.07);background:rgba(10,12,20,.82);backdrop-filter:blur(18px);position:sticky;top:0;z-index:20}.brand{display:flex;align-items:center;gap:11px;font-size:17px;font-weight:800;letter-spacing:.02em;white-space:nowrap}.brand b{color:var(--accent2)}.brand-mark{display:grid;place-items:center;width:34px;height:34px;border-radius:10px;background:linear-gradient(145deg,var(--accent),#6551d5);box-shadow:0 8px 24px rgba(159,122,234,.3)}.site-header nav{display:flex;gap:24px}.site-header nav a{color:#c8cede;font-weight:650}.site-header nav a:hover,.text-link:hover{color:white}.nav-search{margin-left:auto}.nav-search input,.hero-search input,.filter-bar input,.hero-search select,.filter-bar select{border:1px solid var(--line);background:#0e1220;color:white;border-radius:12px;outline:none}.nav-search input{width:190px;padding:9px 12px}.nav-search input:focus,.hero-search input:focus,.filter-bar input:focus,.hero-search select:focus,.filter-bar select:focus{border-color:#7661d7;box-shadow:0 0 0 3px rgba(159,122,234,.12)}main{max-width:1180px;margin:0 auto;padding:0 24px 80px}.hero{padding:92px 0 62px;text-align:center}.eyebrow{text-transform:uppercase;letter-spacing:.18em;font-size:11px;font-weight:800;color:var(--accent2)}h1,h2,h3,p{margin-top:0}.hero h1,.page-head h1,.detail-copy h1,.simple-detail h1{font-size:clamp(40px,7vw,78px);line-height:.98;letter-spacing:-.055em;margin:18px 0 20px}.hero h1 span{background:linear-gradient(90deg,#c4b5fd,#67e8f9);-webkit-background-clip:text;background-clip:text;color:transparent}.hero>p,.page-head>p,.lead{max-width:680px;margin:0 auto 30px;color:var(--muted);font-size:18px}.hero-search{display:flex;max-width:680px;margin:30px auto 0;padding:7px;border:1px solid var(--line);border-radius:17px;background:rgba(17,21,34,.9);box-shadow:var(--shadow)}.hero-search input{border:0;background:transparent;flex:1;min-width:0;padding:13px 15px;font-size:16px}.hero-search select{border:0;border-left:1px solid var(--line);border-radius:0;background:transparent;padding:0 13px}.hero-search button,.filter-bar button,.primary-button{border:0;border-radius:11px;padding:12px 20px;background:linear-gradient(135deg,#8b6be8,#6651d3);color:#fff;font-weight:800;cursor:pointer}.count-strip{display:flex;justify-content:center;gap:1px;margin:48px auto 0;max-width:690px;background:var(--line);border:1px solid var(--line);border-radius:16px;overflow:hidden}.count-strip a{flex:1;background:var(--panel);padding:18px}.count-strip strong{display:block;font-size:24px}.count-strip span{font-size:12px;color:var(--muted);text-transform:uppercase;letter-spacing:.08em}.section{padding:38px 0}.section-heading{display:flex;align-items:end;justify-content:space-between;gap:16px;margin-bottom:22px}.section-heading h2,.panel h2{margin:5px 0 0;font-size:28px;letter-spacing:-.03em}.text-link{color:#c5b8fb;font-weight:700}.entity-grid{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:16px}.entity-card,.list-card,.panel,.empty-state,.filter-bar,.search-result{border:1px solid var(--line);background:linear-gradient(180deg,rgba(23,28,45,.92),rgba(16,20,32,.95));box-shadow:0 12px 40px rgba(0,0,0,.12)}.entity-card{border-radius:18px;overflow:hidden;transition:.18s transform,.18s border-color}.entity-card:hover{transform:translateY(-3px);border-color:#5e547e}.entity-media{height:190px;display:grid;place-items:center;background:radial-gradient(circle at 50% 42%,rgba(255,255,255,.12),transparent 52%),#151a29;overflow:hidden}.entity-media img{width:100%;height:100%;object-fit:contain;padding:10px;filter:drop-shadow(0 14px 20px rgba(0,0,0,.32))}.entity-media>span,.small-media>span{font-size:52px;font-weight:900;color:rgba(255,255,255,.5)}.element-fire{background-color:#2d171b}.element-water{background-color:#13253a}.element-wood{background-color:#162a20}.element-light{background-color:#2b2819}.element-dark{background-color:#241a34}.entity-copy{padding:16px}.entity-meta{display:flex;justify-content:space-between;gap:10px;color:var(--muted);font-size:12px;text-transform:uppercase;letter-spacing:.08em}.stars{color:#ffd76a;letter-spacing:.02em}.entity-copy h3,.list-card h3,.search-result h3{font-size:18px;margin:7px 0 2px}.entity-copy p,.list-card p,.search-result p{margin:0;color:var(--muted);font-size:13px}.compact-section{max-width:780px}.import-list{border-top:1px solid var(--line)}.import-row{display:grid;grid-template-columns:1fr 130px 90px;gap:16px;padding:13px 0;border-bottom:1px solid var(--line);color:var(--muted)}.import-row strong{color:white}.page-head{padding:70px 0 28px}.page-head h1{font-size:clamp(42px,6vw,68px);margin:10px 0 18px}.page-head>p{margin-left:0}.inline-search{margin-left:0;max-width:800px}.filter-bar{border-radius:16px;padding:13px;margin-bottom:5px}.filter-bar form{display:flex;gap:10px}.filter-bar input{flex:1;min-width:150px;padding:11px 13px}.filter-bar select{padding:10px 13px;min-width:145px}.ghost-button{display:grid;place-items:center;border:1px solid var(--line);border-radius:11px;padding:0 15px;color:#cbd2e2;font-weight:700}.catalog-section{padding-top:22px}.empty-state{border-radius:18px;padding:48px;text-align:center;color:var(--muted)}.empty-state h3,.empty-state h1{color:white;margin-bottom:8px}.empty-state code{background:#0d101b;border:1px solid var(--line);padding:2px 6px;border-radius:6px}.page-empty{margin:90px auto;max-width:620px}.page-empty .primary-button{display:inline-block;margin-top:12px}.search-section{padding-top:16px}.search-results{display:grid;gap:10px;max-width:860px}.search-result,.list-card{display:grid;grid-template-columns:62px 1fr auto;align-items:center;gap:14px;padding:11px 15px;border-radius:14px}.search-result:hover,.list-card:hover{border-color:#5d527b;background:var(--panel2)}.small-media{width:58px;height:58px;border-radius:13px;background:#0c101b;display:grid;place-items:center;overflow:hidden}.small-media img{width:100%;height:100%;object-fit:contain;padding:4px}.small-media>span{font-size:24px}.tag,.element-pill{display:inline-block;border:1px solid #3c445c;background:#1a2032;border-radius:999px;padding:2px 8px;color:#bfc8da;font-size:10px;text-transform:uppercase;letter-spacing:.08em;font-weight:800}.result-id{color:#6f7890;font-family:ui-monospace,monospace}.list-grid{display:grid;grid-template-columns:1fr 1fr;gap:10px}.list-card>div:nth-child(2){min-width:0}.list-card p{white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.pagination{display:grid;grid-template-columns:1fr auto 1fr;align-items:center;margin-top:26px;color:var(--muted)}.pagination a{font-weight:700;color:#c7baf8}.pagination a:last-child{text-align:right}.detail-hero{display:grid;grid-template-columns:minmax(280px,430px) 1fr;gap:70px;align-items:center;padding:70px 0 46px}.detail-media{min-height:420px;border-radius:28px;border:1px solid var(--line);background:radial-gradient(circle at 50% 35%,rgba(159,122,234,.2),transparent 55%),var(--panel);display:grid;place-items:center;overflow:hidden}.large-image{width:100%;height:100%;max-height:500px;object-fit:contain;padding:22px;filter:drop-shadow(0 25px 30px rgba(0,0,0,.32))}.large-placeholder{display:grid;place-items:center;gap:8px;color:var(--muted)}.large-placeholder span{font-size:88px;color:#8f84b6;font-weight:900}.detail-kicker{display:flex;align-items:center;gap:10px;color:var(--muted);font-family:ui-monospace,monospace}.detail-copy h1,.simple-detail h1{font-size:clamp(48px,7vw,76px);margin:16px 0 13px}.detail-subtitle{font-size:18px;color:var(--muted)}.stat-grid{display:grid;grid-template-columns:repeat(4,1fr);gap:10px;margin-top:30px}.stat-card{border:1px solid var(--line);background:var(--panel);border-radius:14px;padding:14px}.stat-card span{display:block;color:var(--muted);font-size:11px;font-weight:800;letter-spacing:.1em}.stat-card strong{font-size:20px}.detail-columns{display:grid;grid-template-columns:1fr 1fr;gap:16px}.panel{border-radius:19px;padding:24px}.facts{margin:18px 0 0}.facts div{display:flex;justify-content:space-between;gap:20px;padding:10px 0;border-bottom:1px solid var(--line)}.facts dt{color:var(--muted)}.facts dd{margin:0;font-weight:700}.detail-links{display:grid;gap:8px;margin-top:18px}.detail-link{display:grid;grid-template-columns:1fr auto 24px;gap:8px;align-items:center;padding:12px;border:1px solid var(--line);border-radius:12px}.detail-link span{color:var(--muted)}.simple-detail{display:grid;grid-template-columns:330px 1fr;gap:55px;align-items:center;padding:85px 0}.simple-detail>div:first-child{height:330px;border:1px solid var(--line);background:var(--panel);border-radius:24px;display:grid;place-items:center;overflow:hidden}.simple-detail .lead{margin-left:0}.wide-facts{max-width:620px}footer{max-width:1180px;margin:0 auto;padding:28px 24px 45px;border-top:1px solid var(--line);display:flex;justify-content:space-between;color:var(--muted)}footer div{display:grid;gap:2px}footer strong{color:white}.muted{color:var(--muted)}@media(max-width:900px){.site-header nav{display:none}.entity-grid{grid-template-columns:repeat(2,minmax(0,1fr))}.detail-hero,.simple-detail{grid-template-columns:1fr;gap:28px}.detail-media{min-height:330px}.detail-columns,.list-grid{grid-template-columns:1fr}.hero{padding-top:65px}}@media(max-width:620px){main{padding-left:15px;padding-right:15px}.site-header{padding:0 15px;gap:14px}.nav-search{display:none}.hero h1{font-size:48px}.hero-search,.filter-bar form{flex-wrap:wrap}.hero-search input,.filter-bar input{flex-basis:100%}.hero-search select{border-left:0}.count-strip{margin-top:35px}.entity-grid{grid-template-columns:1fr 1fr;gap:9px}.entity-media{height:145px}.entity-copy{padding:12px}.import-row{grid-template-columns:1fr}.stat-grid{grid-template-columns:1fr 1fr}.detail-columns{grid-template-columns:1fr}.empty-state{padding:30px 18px}.search-result,.list-card{grid-template-columns:52px 1fr}.search-result .result-id,.list-card>b{display:none}.small-media{width:50px;height:50px}footer{display:grid;gap:16px}.simple-detail>div:first-child{height:280px}.detail-hero{padding-top:45px}}
"#;
