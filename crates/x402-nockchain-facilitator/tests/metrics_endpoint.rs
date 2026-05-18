//! `/metrics` end-to-end smoke. Boots the facilitator router merged with
//! `metrics_router`, drives a `/discovery/resources` GET so the request
//! counter has a non-zero sample, then GETs `/metrics` and asserts the
//! Prometheus text-exposition body contains the expected metric name.

use std::sync::Arc;

use anyhow::Result;
use axum::Router;
use tokio::net::TcpListener;
use x402_nockchain_facilitator::{
    install_metrics_recorder, metrics_router, router as facilitator_router, AppState,
    InMemoryCatalogStore,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn metrics_endpoint_renders_counter_after_request() -> Result<()> {
    let catalog = Arc::new(InMemoryCatalogStore::new());
    let state = AppState::with_stub_chain(catalog);
    let handle = install_metrics_recorder()?;

    let app: Router = facilitator_router(state).merge(metrics_router(handle));
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let base = format!("http://{}", addr);
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    // Drive a request through `/discovery/resources` so the counter
    // has a non-zero sample (prometheus only renders families that
    // have observations).
    let http = reqwest::Client::new();
    let resp = http
        .get(format!("{}/discovery/resources", base))
        .send()
        .await?;
    assert!(resp.status().is_success());

    let metrics = http.get(format!("{}/metrics", base)).send().await?;
    assert!(metrics.status().is_success());
    let body = metrics.text().await?;

    assert!(
        body.contains("x402_requests_total"),
        "/metrics body missing x402_requests_total counter:\n{body}"
    );
    assert!(
        body.contains("endpoint=\"/discovery/resources\""),
        "/metrics body missing endpoint label:\n{body}"
    );

    // Print the first 30 lines of /metrics output so `cargo test --
    // --nocapture` doubles as a quick operator-readable smoke.
    let preview: Vec<&str> = body.lines().take(30).collect();
    println!("--- /metrics (first 30 lines) ---\n{}", preview.join("\n"));
    Ok(())
}
