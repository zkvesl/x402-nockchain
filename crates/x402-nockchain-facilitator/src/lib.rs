//! Reference Nockchain x402 facilitator.
//!
//! Scaffold. Exposes an axum service at `/verify`, `/settle`, and
//! `/discovery/resources`, persisting the catalog to SQLite. Cataloging
//! happens as a side effect of `/verify` or `/settle` per
//! `coinbase/x402:specs/extensions/bazaar.md §Facilitator Behavior`
//! ("the facilitator IS the registry").
