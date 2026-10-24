//! Thumbnail, preview and tile caches, with an in-memory LRU tier and a disk
//! tier. Keyed by photo id plus edit version, so invalidation is a key change.
//!
//! [`ByteLru`] is the in-memory tier: least recently used first out, with a
//! cap in bytes. The prefetched neighbors' level 1 lives in one (DEV-01,
//! *Prefetched neighbors*: 768 MB).

use std::collections::HashMap;
use std::hash::Hash;

/// A least-recently-used map whose entries each count their own bytes,
/// within a cap. Inserting past the cap evicts the least recently used
/// entries first; an entry larger than the cap is refused.
#[derive(Debug)]
pub struct ByteLru<K, V> {
    cap: u64,
    used: u64,
    clock: u64,
    items: HashMap<K, Entry<V>>,
}

#[derive(Debug)]
struct Entry<V> {
    value: V,
    bytes: u64,
    last: u64,
}

impl<K: Eq + Hash + Clone, V> ByteLru<K, V> {
    pub fn new(cap: u64) -> Self {
        Self {
            cap,
            used: 0,
            clock: 0,
            items: HashMap::new(),
        }
    }

    /// The bytes held.
    pub fn used(&self) -> u64 {
        self.used
    }

    pub fn cap(&self) -> u64 {
        self.cap
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn contains(&self, key: &K) -> bool {
        self.items.contains_key(key)
    }

    /// The value, marked as just used.
    pub fn get(&mut self, key: &K) -> Option<&V> {
        self.clock += 1;
        let clock = self.clock;
        self.items.get_mut(key).map(|e| {
            e.last = clock;
            &e.value
        })
    }

    /// Inserts `value` of `bytes`, replacing the key's old value, and
    /// evicts the least recently used entries until it fits. Returns what
    /// was evicted, oldest first, or the value back when it alone is larger
    /// than the cap (nothing is evicted then).
    pub fn insert(&mut self, key: K, value: V, bytes: u64) -> Result<Vec<(K, V)>, V> {
        if bytes > self.cap {
            return Err(value);
        }
        self.remove(&key);
        let mut evicted = Vec::new();
        while self.used + bytes > self.cap {
            let oldest = self
                .items
                .iter()
                .min_by_key(|(_, e)| e.last)
                .map(|(k, _)| k.clone())
                .expect("used > 0 means an entry");
            let v = self.remove(&oldest).expect("present");
            evicted.push((oldest, v));
        }
        self.clock += 1;
        self.used += bytes;
        self.items.insert(
            key,
            Entry {
                value,
                bytes,
                last: self.clock,
            },
        );
        Ok(evicted)
    }

    /// Takes the key's value out.
    pub fn remove(&mut self, key: &K) -> Option<V> {
        let e = self.items.remove(key)?;
        self.used -= e.bytes;
        Some(e.value)
    }

    /// Keeps only the entries `keep` accepts.
    pub fn retain(&mut self, mut keep: impl FnMut(&K) -> bool) {
        let gone: Vec<K> = self.items.keys().filter(|k| !keep(k)).cloned().collect();
        for k in gone {
            self.remove(&k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_the_least_recently_used_first() {
        let mut c = ByteLru::new(100);
        c.insert("a", 1, 40).unwrap();
        c.insert("b", 2, 40).unwrap();
        // "a" is used, so "b" is the oldest.
        assert_eq!(c.get(&"a"), Some(&1));
        let evicted = c.insert("c", 3, 40).unwrap();
        assert_eq!(evicted, [("b", 2)]);
        assert_eq!((c.used(), c.len()), (80, 2));
        assert!(c.contains(&"a") && c.contains(&"c"));
    }

    #[test]
    fn evicts_as_many_as_it_takes() {
        let mut c = ByteLru::new(100);
        for (i, k) in ["a", "b", "c", "d"].into_iter().enumerate() {
            c.insert(k, i, 25).unwrap();
        }
        let evicted = c.insert("big", 9, 70).unwrap();
        assert_eq!(evicted, [("a", 0), ("b", 1), ("c", 2)]);
        assert_eq!(c.used(), 95);
    }

    #[test]
    fn replacing_a_key_frees_its_old_bytes() {
        let mut c = ByteLru::new(100);
        c.insert("a", 1, 60).unwrap();
        assert!(c.insert("a", 2, 70).unwrap().is_empty());
        assert_eq!((c.used(), c.get(&"a")), (70, Some(&2)));
    }

    #[test]
    fn a_value_larger_than_the_cap_is_refused() {
        let mut c = ByteLru::new(100);
        c.insert("a", 1, 50).unwrap();
        assert_eq!(c.insert("huge", 2, 101), Err(2));
        assert_eq!((c.used(), c.len()), (50, 1), "nothing evicted");
    }

    #[test]
    fn retain_and_remove_keep_the_count() {
        let mut c = ByteLru::new(100);
        c.insert(1, "x", 10).unwrap();
        c.insert(2, "y", 20).unwrap();
        c.insert(3, "z", 30).unwrap();
        c.retain(|k| *k != 2);
        assert_eq!((c.used(), c.len()), (40, 2));
        assert_eq!(c.remove(&3), Some("z"));
        assert_eq!(c.used(), 10);
    }
}
