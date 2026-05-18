//! In-memory [`CatalogStore`]. Zero-dependency implementation used by
//! the facilitator's own contract test suite and by demos that don't
//! want to spin a SQLite pool.
//!
//! Backed by a `Mutex<Vec<DiscoveryResource>>`. Order is insertion order;
//! re-upserting `(resource, kind)` replaces the row in place so listings
//! stay stable across overwrites.

use std::sync::Mutex;

use anyhow::Result;
use async_trait::async_trait;
use x402_types::bazaar::{DiscoveryResource, ListDiscoveryResourcesParams};
use x402_types::payment::PaymentRequirements;

use super::contract::CatalogStore;

const DEFAULT_LIST_LIMIT: u32 = 50;

#[derive(Debug, Default)]
pub struct InMemoryCatalogStore {
    rows: Mutex<Vec<DiscoveryResource>>,
}

impl InMemoryCatalogStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl CatalogStore for InMemoryCatalogStore {
    async fn upsert(&self, resource: &DiscoveryResource) -> Result<()> {
        let mut rows = self.rows.lock().expect("catalog mutex poisoned");
        match rows
            .iter()
            .position(|r| r.resource == resource.resource && r.kind == resource.kind)
        {
            Some(idx) => rows[idx] = resource.clone(),
            None => rows.push(resource.clone()),
        }
        Ok(())
    }

    async fn list(
        &self,
        params: &ListDiscoveryResourcesParams,
    ) -> Result<(Vec<DiscoveryResource>, u32)> {
        let rows = self.rows.lock().expect("catalog mutex poisoned");
        let mut filtered: Vec<DiscoveryResource> = rows
            .iter()
            .filter(|r| match params.kind.as_deref() {
                Some(k) => r.kind == k,
                None => true,
            })
            .cloned()
            .collect();
        drop(rows);

        if let Some(net) = params.network.as_deref() {
            filtered.retain(|r| accepts_has_network(&r.accepts, net));
        }
        if let Some(scheme) = params.scheme.as_deref() {
            filtered.retain(|r| accepts_has_scheme(&r.accepts, scheme));
        }

        let total = filtered.len() as u32;
        let limit = params.limit.unwrap_or(DEFAULT_LIST_LIMIT) as usize;
        let offset = params.offset.unwrap_or(0) as usize;
        let items = filtered.into_iter().skip(offset).take(limit).collect();
        Ok((items, total))
    }
}

fn accepts_has_network(accepts: &[PaymentRequirements], network: &str) -> bool {
    accepts.iter().any(|p| p.network == network)
}

fn accepts_has_scheme(accepts: &[PaymentRequirements], scheme: &str) -> bool {
    accepts.iter().any(|p| p.scheme == scheme)
}
