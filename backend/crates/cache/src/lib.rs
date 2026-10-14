//! Thumbnail, preview and tile caches, with an in-memory LRU tier and a disk
//! tier. Keyed by photo id plus edit version, so invalidation is a key change.
//!
//! Scheduled for M1.
