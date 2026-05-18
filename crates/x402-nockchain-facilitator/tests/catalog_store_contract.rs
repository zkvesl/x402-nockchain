//! Generic contract test suite for [`CatalogStore`].
//!
//! The same `async fn` is run against both built-in impls
//! ([`SqliteCatalogStore`] and [`InMemoryCatalogStore`]) — a third-party
//! impl can prove compatibility by adding a third pair of test functions.
//!
//! Each test takes an `&dyn CatalogStore`, never the concrete type, so
//! impl-specific behaviour can't leak into the asserts.

use std::sync::Arc;

use x402_nockchain_facilitator::{
    in_memory_pool, CatalogStore, InMemoryCatalogStore, SqliteCatalogStore,
};
use x402_types::bazaar::{DiscoveryResource, ListDiscoveryResourcesParams};
use x402_types::payment::PaymentRequirements;

// ---------------------------------------------------------------------------
// Fixture builders
// ---------------------------------------------------------------------------

fn requirements(scheme: &str, network: &str) -> PaymentRequirements {
    PaymentRequirements {
        scheme: scheme.into(),
        network: network.into(),
        max_amount_required: "100".into(),
        resource: "/dummy".into(),
        asset: "NOCK".into(),
        pay_to: "payee".into(),
        max_timeout_seconds: 30,
        description: None,
        mime_type: None,
        output_schema: None,
        extra: None,
        extensions: None,
    }
}

fn resource(
    name: &str,
    kind: &str,
    accepts: Vec<PaymentRequirements>,
) -> DiscoveryResource {
    DiscoveryResource {
        resource: format!("https://example.test/{name}"),
        kind: kind.into(),
        x402_version: 2,
        accepts,
        last_updated: "2026-04-24T00:00:00Z".into(),
        metadata: None,
    }
}

// ---------------------------------------------------------------------------
// Per-impl factories
// ---------------------------------------------------------------------------

async fn make_memory() -> Arc<dyn CatalogStore> {
    Arc::new(InMemoryCatalogStore::new())
}

async fn make_sqlite() -> Arc<dyn CatalogStore> {
    let pool = in_memory_pool().await.expect("open sqlite");
    Arc::new(SqliteCatalogStore::new(pool))
}

// ---------------------------------------------------------------------------
// Contract behaviours
// ---------------------------------------------------------------------------

async fn empty_list_returns_zero(store: Arc<dyn CatalogStore>) {
    let (items, total) = store
        .list(&ListDiscoveryResourcesParams::default())
        .await
        .unwrap();
    assert!(items.is_empty());
    assert_eq!(total, 0);
}

async fn upsert_then_list_round_trips(store: Arc<dyn CatalogStore>) {
    let r = resource(
        "echo",
        "mcp",
        vec![requirements("exact", "nockchain:mainnet")],
    );
    store.upsert(&r).await.unwrap();

    let (items, total) = store
        .list(&ListDiscoveryResourcesParams::default())
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].resource, r.resource);
    assert_eq!(items[0].kind, "mcp");
}

async fn upsert_replaces_existing(store: Arc<dyn CatalogStore>) {
    let r1 = resource(
        "echo",
        "mcp",
        vec![requirements("exact", "nockchain:mainnet")],
    );
    let mut r2 = r1.clone();
    r2.last_updated = "2026-04-25T00:00:00Z".into();
    r2.x402_version = 3;

    store.upsert(&r1).await.unwrap();
    store.upsert(&r2).await.unwrap();

    let (items, total) = store
        .list(&ListDiscoveryResourcesParams::default())
        .await
        .unwrap();
    assert_eq!(total, 1, "duplicate (resource, kind) must collapse");
    assert_eq!(items[0].x402_version, 3);
    assert_eq!(items[0].last_updated, "2026-04-25T00:00:00Z");
}

async fn distinct_kinds_for_same_resource_coexist(store: Arc<dyn CatalogStore>) {
    let mcp = resource("dual", "mcp", vec![requirements("exact", "nockchain:mainnet")]);
    let http = resource(
        "dual",
        "http",
        vec![requirements("exact", "nockchain:mainnet")],
    );
    store.upsert(&mcp).await.unwrap();
    store.upsert(&http).await.unwrap();

    let (_, total) = store
        .list(&ListDiscoveryResourcesParams::default())
        .await
        .unwrap();
    assert_eq!(total, 2);
}

async fn kind_filter_narrows_listing(store: Arc<dyn CatalogStore>) {
    store
        .upsert(&resource(
            "a",
            "mcp",
            vec![requirements("exact", "nockchain:mainnet")],
        ))
        .await
        .unwrap();
    store
        .upsert(&resource(
            "b",
            "http",
            vec![requirements("exact", "nockchain:mainnet")],
        ))
        .await
        .unwrap();

    let (items, total) = store
        .list(&ListDiscoveryResourcesParams {
            kind: Some("mcp".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(items[0].kind, "mcp");
}

async fn network_filter_matches_any_accepts_entry(store: Arc<dyn CatalogStore>) {
    store
        .upsert(&resource(
            "a",
            "mcp",
            vec![
                requirements("exact", "nockchain:mainnet"),
                requirements("upto", "nockchain:fakenet"),
            ],
        ))
        .await
        .unwrap();
    store
        .upsert(&resource(
            "b",
            "mcp",
            vec![requirements("exact", "ethereum:mainnet")],
        ))
        .await
        .unwrap();

    let (items, total) = store
        .list(&ListDiscoveryResourcesParams {
            network: Some("nockchain:fakenet".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert!(items[0].resource.ends_with('a'));
}

async fn scheme_filter_matches_any_accepts_entry(store: Arc<dyn CatalogStore>) {
    store
        .upsert(&resource(
            "a",
            "mcp",
            vec![requirements("upto", "nockchain:mainnet")],
        ))
        .await
        .unwrap();
    store
        .upsert(&resource(
            "b",
            "mcp",
            vec![requirements("exact", "nockchain:mainnet")],
        ))
        .await
        .unwrap();

    let (items, total) = store
        .list(&ListDiscoveryResourcesParams {
            scheme: Some("upto".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert!(items[0].resource.ends_with('a'));
}

async fn pagination_total_reflects_filter_count(store: Arc<dyn CatalogStore>) {
    for n in 0..5 {
        store
            .upsert(&resource(
                &format!("r{n}"),
                "mcp",
                vec![requirements("exact", "nockchain:mainnet")],
            ))
            .await
            .unwrap();
    }
    let (items, total) = store
        .list(&ListDiscoveryResourcesParams {
            limit: Some(2),
            offset: Some(1),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(total, 5, "total counts pre-pagination matches");
    assert_eq!(items.len(), 2);
    assert!(items[0].resource.ends_with("r1"));
    assert!(items[1].resource.ends_with("r2"));
}

async fn combined_filters_intersect(store: Arc<dyn CatalogStore>) {
    store
        .upsert(&resource(
            "match",
            "mcp",
            vec![requirements("exact", "nockchain:mainnet")],
        ))
        .await
        .unwrap();
    store
        .upsert(&resource(
            "wrong-kind",
            "http",
            vec![requirements("exact", "nockchain:mainnet")],
        ))
        .await
        .unwrap();
    store
        .upsert(&resource(
            "wrong-network",
            "mcp",
            vec![requirements("exact", "ethereum:mainnet")],
        ))
        .await
        .unwrap();
    store
        .upsert(&resource(
            "wrong-scheme",
            "mcp",
            vec![requirements("upto", "nockchain:mainnet")],
        ))
        .await
        .unwrap();

    let (items, total) = store
        .list(&ListDiscoveryResourcesParams {
            kind: Some("mcp".into()),
            network: Some("nockchain:mainnet".into()),
            scheme: Some("exact".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert!(items[0].resource.ends_with("match"));
}

// ---------------------------------------------------------------------------
// Per-impl tests — each behaviour, twice.
// ---------------------------------------------------------------------------

macro_rules! contract_tests {
    ($mod_name:ident, $factory:expr) => {
        mod $mod_name {
            #[tokio::test]
            async fn empty_list_returns_zero() {
                super::empty_list_returns_zero($factory.await).await;
            }

            #[tokio::test]
            async fn upsert_then_list_round_trips() {
                super::upsert_then_list_round_trips($factory.await).await;
            }

            #[tokio::test]
            async fn upsert_replaces_existing() {
                super::upsert_replaces_existing($factory.await).await;
            }

            #[tokio::test]
            async fn distinct_kinds_for_same_resource_coexist() {
                super::distinct_kinds_for_same_resource_coexist($factory.await).await;
            }

            #[tokio::test]
            async fn kind_filter_narrows_listing() {
                super::kind_filter_narrows_listing($factory.await).await;
            }

            #[tokio::test]
            async fn network_filter_matches_any_accepts_entry() {
                super::network_filter_matches_any_accepts_entry($factory.await).await;
            }

            #[tokio::test]
            async fn scheme_filter_matches_any_accepts_entry() {
                super::scheme_filter_matches_any_accepts_entry($factory.await).await;
            }

            #[tokio::test]
            async fn pagination_total_reflects_filter_count() {
                super::pagination_total_reflects_filter_count($factory.await).await;
            }

            #[tokio::test]
            async fn combined_filters_intersect() {
                super::combined_filters_intersect($factory.await).await;
            }
        }
    };
}

contract_tests!(memory, super::make_memory());
contract_tests!(sqlite, super::make_sqlite());
