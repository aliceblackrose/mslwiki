use anyhow::Result;
use axum::{Json, Router, extract::State, response::Html, routing::get};
use msl_db::{Database, ImportRecord};
use std::net::SocketAddr;

pub fn router(database: Database) -> Router {
    Router::new()
        .route("/", get(home))
        .route("/health", get(health))
        .route("/api/v1/imports", get(imports))
        .with_state(database)
}

pub async fn serve(database: Database, address: SocketAddr) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, router(database)).await?;
    Ok(())
}

async fn health() -> &'static str {
    "ok"
}

async fn imports(
    State(database): State<Database>,
) -> Result<Json<Vec<ImportRecord>>, (axum::http::StatusCode, String)> {
    database
        .recent_imports(20)
        .await
        .map(Json)
        .map_err(internal_error)
}

async fn home(State(database): State<Database>) -> Html<String> {
    let imports = database.recent_imports(10).await.unwrap_or_default();

    let rows = imports
        .into_iter()
        .map(|import| {
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape_html(&import.name),
                escape_html(&import.version_name),
                import.file_count,
                escape_html(&import.imported_at),
            )
        })
        .collect::<String>();

    Html(format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>MSL Wiki</title>
<style>
body {{ font-family: system-ui, sans-serif; max-width: 960px; margin: 3rem auto; padding: 0 1rem; }}
table {{ border-collapse: collapse; width: 100%; }}
th, td {{ border-bottom: 1px solid #ddd; padding: .65rem; text-align: left; }}
code {{ background: #eee; padding: .15rem .3rem; }}
</style>
</head>
<body>
<h1>MSL Wiki</h1>
<p>Automated Monster Super League data and asset pipeline.</p>
<p>Import a build with <code>mslwiki import &lt;file.xapk&gt;</code>.</p>
<h2>Recent imports</h2>
<table>
<thead><tr><th>Game</th><th>Version</th><th>Staged files</th><th>Imported</th></tr></thead>
<tbody>{rows}</tbody>
</table>
</body>
</html>"#
    ))
}

fn internal_error(error: anyhow::Error) -> (axum::http::StatusCode, String) {
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        error.to_string(),
    )
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
