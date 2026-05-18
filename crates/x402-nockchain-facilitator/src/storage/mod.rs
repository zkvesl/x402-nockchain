//! Catalog storage. The [`CatalogStore`] trait abstracts upsert + list
//! over whatever backend the facilitator is wired to. Two impls ship
//! in-tree:
//!
//! - [`SqliteCatalogStore`]: the default, schema in
//!   `migrations/0001_catalog.sql`. Used by `examples/demo.sh` and any
//!   real deployment.
//! - [`InMemoryCatalogStore`]: a zero-dep impl for tests and demos.
//!
//! Both pass a shared contract test suite at
//! `tests/catalog_store_contract.rs`.

pub mod contract;
pub mod memory;
pub mod sqlite;

pub use contract::CatalogStore;
pub use memory::InMemoryCatalogStore;
pub use sqlite::SqliteCatalogStore;
