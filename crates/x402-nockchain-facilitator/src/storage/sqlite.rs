//! SQLite-backed [`CatalogStore`]. Schema is defined in
//! `migrations/0001_catalog.sql`.
//!
//! `accepts` and `metadata` are stored as TEXT (JSON-serialized) so the
//! row shape is loose; the `(resource, type)` UNIQUE constraint plus the
//! `type` index handle the only hot-path lookups (catalog upsert from
//! `/verify`, listing by kind from `/discovery/resources`). Filtering by
//! `network`/`scheme` is done in Rust over the parsed `accepts` blob —
//! the catalog is small enough (< 10K rows in any realistic deployment)
//! that this is faster than the JSON-extension dance and keeps the
//! schema portable.

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use sqlx::{FromRow, SqlitePool};
use x402_types::bazaar::{DiscoveryResource, ListDiscoveryResourcesParams};
use x402_types::payment::PaymentRequirements;

use super::contract::CatalogStore;

const DEFAULT_LIST_LIMIT: u32 = 50;

/// SQLite-backed catalog. Cheap to clone — the underlying [`SqlitePool`]
/// is internally `Arc`-shared.
#[derive(Debug, Clone)]
pub struct SqliteCatalogStore {
    pool: SqlitePool,
}

impl SqliteCatalogStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Underlying pool, exposed for callers that need to reuse the same
    /// connection (e.g., custom migrations). Prefer [`CatalogStore`]
    /// methods for catalog operations.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

#[async_trait]
impl CatalogStore for SqliteCatalogStore {
    async fn upsert(&self, resource: &DiscoveryResource) -> Result<()> {
        let accepts_json =
            serde_json::to_string(&resource.accepts).context("serialize accepts JSON")?;
        let metadata_json = resource
            .metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("serialize metadata JSON")?;

        sqlx::query(
            "INSERT INTO catalog (resource, type, x402_version, accepts, last_updated, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(resource, type) DO UPDATE SET
                 x402_version = excluded.x402_version,
                 accepts      = excluded.accepts,
                 last_updated = excluded.last_updated,
                 metadata     = excluded.metadata",
        )
        .bind(&resource.resource)
        .bind(&resource.kind)
        .bind(resource.x402_version as i64)
        .bind(&accepts_json)
        .bind(&resource.last_updated)
        .bind(&metadata_json)
        .execute(&self.pool)
        .await
        .context("upsert catalog entry")?;

        Ok(())
    }

    async fn list(
        &self,
        params: &ListDiscoveryResourcesParams,
    ) -> Result<(Vec<DiscoveryResource>, u32)> {
        let rows = match params.kind.as_deref() {
            Some(k) => sqlx::query_as::<_, RawCatalogRow>(
                "SELECT resource, type, x402_version, accepts, last_updated, metadata
                 FROM catalog
                 WHERE type = ?1
                 ORDER BY id",
            )
            .bind(k)
            .fetch_all(&self.pool)
            .await
            .context("fetch catalog rows by kind")?,
            None => sqlx::query_as::<_, RawCatalogRow>(
                "SELECT resource, type, x402_version, accepts, last_updated, metadata
                 FROM catalog
                 ORDER BY id",
            )
            .fetch_all(&self.pool)
            .await
            .context("fetch catalog rows")?,
        };

        let mut decoded: Vec<DiscoveryResource> = rows
            .into_iter()
            .map(RawCatalogRow::into_resource)
            .collect::<Result<_>>()?;

        if let Some(net) = params.network.as_deref() {
            decoded.retain(|r| accepts_has_network(&r.accepts, net));
        }
        if let Some(scheme) = params.scheme.as_deref() {
            decoded.retain(|r| accepts_has_scheme(&r.accepts, scheme));
        }

        let total = decoded.len() as u32;
        let limit = params.limit.unwrap_or(DEFAULT_LIST_LIMIT) as usize;
        let offset = params.offset.unwrap_or(0) as usize;

        let items = decoded.into_iter().skip(offset).take(limit).collect();
        Ok((items, total))
    }
}

fn accepts_has_network(accepts: &[PaymentRequirements], network: &str) -> bool {
    accepts.iter().any(|p| p.network == network)
}

fn accepts_has_scheme(accepts: &[PaymentRequirements], scheme: &str) -> bool {
    accepts.iter().any(|p| p.scheme == scheme)
}

#[derive(FromRow)]
struct RawCatalogRow {
    resource: String,
    #[sqlx(rename = "type")]
    kind: String,
    x402_version: i64,
    accepts: String,
    last_updated: String,
    metadata: Option<String>,
}

impl RawCatalogRow {
    fn into_resource(self) -> Result<DiscoveryResource> {
        let accepts: Vec<PaymentRequirements> =
            serde_json::from_str(&self.accepts).context("parse accepts JSON")?;
        let metadata: Option<Value> = self
            .metadata
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .context("parse metadata JSON")?;
        Ok(DiscoveryResource {
            resource: self.resource,
            kind: self.kind,
            x402_version: self.x402_version as u32,
            accepts,
            last_updated: self.last_updated,
            metadata,
        })
    }
}
