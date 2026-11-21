//! The till's local SQLite store: the catalogue cache it prices from, the
//! sales it records, and the queue of sales not yet sent to the cloud.
//!
//! Every money and quantity column is `TEXT` holding an exact decimal string;
//! see `migrations/0001_init.sql` for why. Only the runtime `sqlx` query API
//! is used (the `macros` feature is deliberately not enabled), so a build
//! never needs a live database.

pub mod schema;
pub mod sequence;

pub use schema::{Store, StoreError, decode_money, decode_pmt_ty, decode_qty, decode_tax_ty};
pub use sequence::PendingSale;
