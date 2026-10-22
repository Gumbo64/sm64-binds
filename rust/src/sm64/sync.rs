//! [`SM64Game::sync_from`]: making a game a copy of a base again, cheaply
//! when only a few pages changed since the last time.

use anyhow::{Error, Result};

use super::SM64Game;
use super::dirty::DirtyTracker;

/// Source of [`SM64Game`] ids, so a sync can tell which base it came from.
static NEXT_GAME_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// What [`SM64Game::sync_from`] keeps between calls.
pub(super) struct SyncState {
    /// Unique per game.
    id: u64,
    /// Bumped by anything that may change the game's memory ([`Self::touch`]),
    /// so a sync knows when a base is unchanged.
    generation: u64,
    /// The base and its generation this game was last synced from.
    synced_with: Option<(u64, u64)>,
    tracker: Option<DirtyTracker>,
    tracking_unavailable: bool,
}

impl SyncState {
    pub(super) fn new() -> SyncState {
        SyncState {
            id: NEXT_GAME_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            generation: 0,
            synced_with: None,
            tracker: None,
            tracking_unavailable: false,
        }
    }

    /// Records that the game's memory may have changed. The only place the
    /// generation moves: everything that can change memory goes through
    /// [`SM64Game::parts_mut`] or [`SM64Game::mem_mut`], which call it.
    pub(super) fn touch(&mut self) {
        self.generation += 1;
    }

    fn version(&self) -> (u64, u64) {
        (self.id, self.generation)
    }
}

impl SM64Game {
    /// Makes this game a copy of `base`, like [`SM64Game::copy_from`] does,
    /// but cheaper when called repeatedly with the same base.
    ///
    /// The first call does a full copy and starts tracking which pages this
    /// game writes. Later calls copy back only those pages, as long as `base`
    /// has not changed since the previous sync (anything that could change
    /// its memory, such as stepping it, counts as a change). Otherwise they
    /// compare every page with `base` and copy the ones that differ. The result
    /// is the same either way. Page tracking
    /// needs Linux 6.7+ (see [`SM64Game::dirty_tracking_active`]); elsewhere
    /// every call is a full copy.
    ///
    /// When `base` changes between calls (eg if it isn't a tree search),
    /// [`SM64Game::copy_from`] is faster: comparing every page costs more than
    /// copying them.
    ///
    /// With `SM64_VERIFY_SYNC=1` in the environment, every sync is checked
    /// byte for byte against `base` and fails if they differ.
    pub fn sync_from(&mut self, base: &SM64Game) -> Result<()> {
        if base.build_id != self.build_id || base.layout != self.layout {
            return Err(Error::msg(
                "sync_from: the base game is a different build of the game",
            ));
        }
        let (base_ptr, base_len) = base.mem();
        let base_ptr = base_ptr as *const u8;
        let (ptr, len) = self.mem_mut();
        let tracked = self
            .sync
            .tracker
            .as_ref()
            .is_some_and(|t| t.covers(ptr, len));
        let unchanged_base = self.sync.synced_with == Some(base.sync.version());

        if tracked && unchanged_base && len == base_len {
            let tracker = self.sync.tracker.as_mut().expect("checked above");
            let written = tracker.take_written()?;
            for &(start, end) in &written {
                // SAFETY: both memories are `len` bytes long and `[start, end)`
                // lies within the tracked region, which is exactly that long.
                // They are distinct allocations, so the ranges cannot overlap.
                unsafe {
                    std::ptr::copy_nonoverlapping(base_ptr.add(start), ptr.add(start), end - start)
                };
            }
            // The copies above wrote those pages again; they match the base now.
            for &(start, end) in &written {
                tracker.protect(start, end)?;
            }
        } else if tracked && len == base_len {
            // A different or changed base: copy only the pages that differ from
            // it, and protect just those again, rather than copying everything
            // and protecting all of memory (about ten times slower).
            let tracker = self.sync.tracker.as_mut().expect("checked above");
            tracker.take_written()?;
            const PAGE: usize = 4096;
            let mut copied: Vec<(usize, usize)> = Vec::new();
            for (start, end) in self.layout.mutable_ranges() {
                let end = end.min(len);
                // Whole pages: the read-only bytes in the first page never
                // change, so they compare equal and copying them is harmless.
                let mut page = start - start % PAGE;
                while page < end {
                    let next = (page + PAGE).min(end);
                    // SAFETY: both memories are `len` bytes long and distinct
                    // allocations; `[page, next)` lies within them.
                    let (dst, src) = unsafe {
                        (
                            std::slice::from_raw_parts_mut(ptr.add(page), next - page),
                            std::slice::from_raw_parts(base_ptr.add(page), next - page),
                        )
                    };
                    if dst != src {
                        dst.copy_from_slice(src);
                        match copied.last_mut() {
                            Some(last) if last.1 == page => last.1 = next,
                            _ => copied.push((page, next)),
                        }
                    }
                    page = next;
                }
            }
            for &(start, end) in &copied {
                tracker.protect(start, end.next_multiple_of(PAGE).min(len))?;
            }
        } else {
            self.copy_from(base)?;
            self.reset_tracking();
        }
        self.sync.synced_with = Some(base.sync.version());

        if verify_syncs() {
            let (ptr, len) = self.mem();
            // SAFETY: both memories are valid for their sizes here.
            let (a, b) = unsafe {
                (
                    std::slice::from_raw_parts(ptr as *const u8, len),
                    std::slice::from_raw_parts(base_ptr, base_len),
                )
            };
            if let Some(at) = a.iter().zip(b).position(|(x, y)| x != y) {
                return Err(Error::msg(format!(
                    "sync_from left byte {at:#x} different from the base"
                )));
            }
        }
        Ok(())
    }

    /// Whether [`SM64Game::sync_from`] is tracking written pages, rather
    /// than doing full copies.
    pub fn dirty_tracking_active(&self) -> bool {
        self.sync.tracker.is_some()
    }

    /// Marks every page clean, starting a tracker if needed. Called right
    /// after a full copy, when this game matches its base.
    fn reset_tracking(&mut self) {
        let (ptr, len) = self.mem();
        let sync = &mut self.sync;
        if let Some(tracker) = &mut sync.tracker
            && tracker.covers(ptr, len)
            && tracker.protect(0, len).is_ok()
        {
            return;
        }
        // Memory moved or grew, or there is no tracker yet. Dropping the old
        // tracker closes its userfaultfd, which unregisters the old range.
        sync.tracker = None;
        if !sync.tracking_unavailable {
            sync.tracker = DirtyTracker::new(ptr, len);
            sync.tracking_unavailable = sync.tracker.is_none();
        }
    }
}

/// Whether `SM64_VERIFY_SYNC` asks for every sync to be checked.
fn verify_syncs() -> bool {
    static VERIFY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *VERIFY.get_or_init(|| std::env::var_os("SM64_VERIFY_SYNC").is_some_and(|v| v != "0"))
}
