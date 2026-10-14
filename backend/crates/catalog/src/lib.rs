//! The catalog: one SQLite file in WAL mode, migrations, queries, edit
//! history and XMP sidecars.
//!
//! Scheduled for M2 (CAT-01). M1 opens folders directly, without a catalog.
//! rusqlite is adopted after the M0 license review.
