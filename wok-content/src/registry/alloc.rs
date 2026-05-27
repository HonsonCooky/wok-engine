//! Per-kind typed serial newtypes and the generic `KindTable` that holds entries indexed by
//! serial with a slug lookup. Tombstones live alongside live entries in the same vector;
//! they survive round-trip through `RegistrySerde` (`registry/serde.rs`) so a save->load
//! pair preserves the slot.

use std::collections::HashMap;
use std::marker::PhantomData;

use wok_scene::Slug;

use crate::error::{AssetKind, RegistryError};
use crate::registry::entry::RegistryEntry;

/// One slot in a `KindTable`. `Empty` is the sentinel for serials that were never allocated
/// (gaps when loading a registry that skipped serials, never produced by `alloc_next`).
/// `Live` carries a real entry. `Tombstone` is a slot whose serial is preserved but whose
/// entry was removed; tombstones survive load->save round-trip per plan section 4.1.
#[derive(Debug, Clone, PartialEq)]
pub enum EntrySlot<E> {
    Empty,
    Live(E),
    Tombstone,
}

impl<E> EntrySlot<E> {
    pub fn as_live(&self) -> Option<&E> {
        match self {
            EntrySlot::Live(e) => Some(e),
            _ => None,
        }
    }

    pub fn as_live_mut(&mut self) -> Option<&mut E> {
        match self {
            EntrySlot::Live(e) => Some(e),
            _ => None,
        }
    }
}

/// Per-kind serial newtype. Defined via macro so all five kinds get the same shape (debug,
/// equality, hash, ordering, raw accessor) without diverging implementations. The raw u32
/// constructor is `pub` so callers that already have a serial from an `AssetId::serial()`
/// can build the typed wrapper; this is the only conversion path.
macro_rules! define_serial {
    ($name:ident, $kind_label:expr) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub u32);

        impl $name {
            pub const fn raw(self) -> u32 {
                self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}#{}", $kind_label, self.0)
            }
        }
    };
}

define_serial!(MeshSerial, "mesh");
define_serial!(AudioSerial, "audio");
define_serial!(AnimationSerial, "animation");
define_serial!(VoiceSerial, "voice");
define_serial!(LightSerial, "light");

/// Generic table holding one kind's entries. `S` is the typed serial wrapper (phantom only;
/// the table stores raw u32 internally), `E` is the entry type.
#[derive(Debug, Clone)]
pub struct KindTable<S, E> {
    next_serial: u32,
    entries: Vec<EntrySlot<E>>,
    by_slug: HashMap<Slug, u32>,
    _marker: PhantomData<S>,
}

impl<S, E: RegistryEntry> Default for KindTable<S, E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S, E: RegistryEntry> KindTable<S, E> {
    pub fn new() -> Self {
        KindTable {
            next_serial: 0,
            entries: Vec::new(),
            by_slug: HashMap::new(),
            _marker: PhantomData,
        }
    }

    pub fn next_serial(&self) -> u32 {
        self.next_serial
    }

    /// Allocate the next serial and store the entry. Returns `SlugCollision` (with the
    /// caller-provided `kind`) if the slug is already in the table. The kind argument is the
    /// `AssetKind` discriminator for error reporting; threading it through here keeps the
    /// table itself kind-agnostic.
    pub fn alloc(&mut self, kind: AssetKind, entry: E) -> Result<u32, RegistryError> {
        if let Some(&existing) = self.by_slug.get(entry.slug()) {
            return Err(RegistryError::SlugCollision {
                kind,
                slug: entry.slug().clone(),
                existing,
            });
        }
        let serial = self.next_serial;
        let slug = entry.slug().clone();
        // Grow the entries vector to cover this serial. Holes are EntrySlot::Empty.
        while self.entries.len() <= serial as usize {
            self.entries.push(EntrySlot::Empty);
        }
        self.entries[serial as usize] = EntrySlot::Live(entry);
        self.by_slug.insert(slug, serial);
        self.next_serial += 1;
        Ok(serial)
    }

    /// Insert an entry at an explicit serial, used by the on-disk loader. Updates
    /// `next_serial` to remain strictly greater than every populated serial. Returns
    /// `SlugCollision` if the slug is already mapped to a different serial.
    pub fn insert_at(
        &mut self,
        kind: AssetKind,
        serial: u32,
        entry: E,
    ) -> Result<(), RegistryError> {
        if let Some(&existing) = self.by_slug.get(entry.slug())
            && existing != serial
        {
            return Err(RegistryError::SlugCollision {
                kind,
                slug: entry.slug().clone(),
                existing,
            });
        }
        while self.entries.len() <= serial as usize {
            self.entries.push(EntrySlot::Empty);
        }
        let slug = entry.slug().clone();
        self.entries[serial as usize] = EntrySlot::Live(entry);
        self.by_slug.insert(slug, serial);
        if serial >= self.next_serial {
            self.next_serial = serial + 1;
        }
        Ok(())
    }

    /// Insert a tombstone at the given serial. Used by the on-disk loader for entries marked
    /// `"status": "deleted"`. Tombstones do not occupy a slug.
    pub fn insert_tombstone(&mut self, serial: u32) {
        while self.entries.len() <= serial as usize {
            self.entries.push(EntrySlot::Empty);
        }
        self.entries[serial as usize] = EntrySlot::Tombstone;
        if serial >= self.next_serial {
            self.next_serial = serial + 1;
        }
    }

    /// Override `next_serial` to a specific value. Used by the on-disk loader so the saved
    /// `next_serial` is preserved even when some serials past the last entry remain empty.
    pub fn set_next_serial(&mut self, next: u32) {
        self.next_serial = next;
    }

    pub fn get(&self, serial: u32) -> Option<&E> {
        self.entries.get(serial as usize).and_then(EntrySlot::as_live)
    }

    pub fn get_mut(&mut self, serial: u32) -> Option<&mut E> {
        self.entries
            .get_mut(serial as usize)
            .and_then(EntrySlot::as_live_mut)
    }

    pub fn by_slug(&self, slug: &Slug) -> Option<u32> {
        self.by_slug.get(slug).copied()
    }

    pub fn iter_slots(&self) -> impl Iterator<Item = (u32, &EntrySlot<E>)> {
        self.entries
            .iter()
            .enumerate()
            .map(|(i, s)| (i as u32, s))
    }

    pub fn live_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|s| matches!(s, EntrySlot::Live(_)))
            .count()
    }

    /// Rename a live entry's slug. Returns `UnknownSerial` if the serial is not live;
    /// `SlugCollision` if the new slug is held by a different serial; `InvalidRename` if the
    /// new slug equals the current slug (the no-op case is rejected rather than ignored so
    /// callers do not chain into rename loops on identical values).
    pub fn rename(
        &mut self,
        kind: AssetKind,
        serial: u32,
        new_slug: Slug,
    ) -> Result<(), RegistryError> {
        // Validate against the immutable side of the table first: existence, no-op detection,
        // and slug-collision lookup. Then take a mutable borrow only for the write half. This
        // ordering keeps the borrow checker happy without runtime cells.
        let current_slug = self
            .get(serial)
            .ok_or(RegistryError::UnknownSerial { kind, serial })?
            .slug()
            .clone();
        if current_slug == new_slug {
            return Err(RegistryError::InvalidRename(format!(
                "{kind} serial {serial} already has slug {new_slug:?}"
            )));
        }
        if let Some(&existing) = self.by_slug.get(&new_slug)
            && existing != serial
        {
            return Err(RegistryError::SlugCollision {
                kind,
                slug: new_slug,
                existing,
            });
        }
        let entry = self.get_mut(serial).expect("existence checked above");
        *entry.slug_mut() = new_slug.clone();
        self.by_slug.remove(&current_slug);
        self.by_slug.insert(new_slug, serial);
        Ok(())
    }

    /// Drop all `UsageSite` entries on every live entry. Called at the start of
    /// `populate_from_scene` so the walk produces a fresh usage map without leftover sites.
    pub fn clear_all_usage(&mut self) {
        for slot in &mut self.entries {
            if let EntrySlot::Live(e) = slot {
                e.clear_usage();
            }
        }
    }
}
