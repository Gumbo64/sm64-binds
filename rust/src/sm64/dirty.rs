//! Tracks which pages of a game's linear memory have been written, so a
//! [`SM64Game::sync_from`](super::SM64Game::sync_from) a base game only copies
//! those pages.
//!
//! Linux 6.7+ only: the memory is registered with a userfaultfd in
//! asynchronous write-protect mode, so the kernel clears a page's protection
//! on its first write without stopping the writer, and `PAGEMAP_SCAN`
//! reports the written pages and protects them again in one call. Only
//! user-mode writes need tracking (WASM code and our own copies), which is
//! what `UFFD_USER_MODE_ONLY` allows without privileges.
//!
//! When any of that is unavailable, [`DirtyTracker::new`] returns `None` and
//! callers fall back to full copies.

/// A written range of memory, as byte offsets from the start of the tracked
/// region.
pub(crate) type Range = (usize, usize);

#[cfg(target_os = "linux")]
mod imp {
    use super::Range;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    const UFFD_API: u64 = 0xAA;
    const UFFD_USER_MODE_ONLY: libc::c_int = 1;
    const UFFD_FEATURE_WP_UNPOPULATED: u64 = 1 << 13;
    const UFFD_FEATURE_WP_ASYNC: u64 = 1 << 15;
    const UFFDIO_REGISTER_MODE_WP: u64 = 1 << 1;
    const UFFDIO_WRITEPROTECT_MODE_WP: u64 = 1 << 0;

    // _IOWR values from linux/userfaultfd.h and linux/fs.h.
    const UFFDIO_API: libc::c_ulong = 0xc018_aa3f;
    const UFFDIO_REGISTER: libc::c_ulong = 0xc020_aa00;
    const UFFDIO_WRITEPROTECT: libc::c_ulong = 0xc018_aa06;
    const PAGEMAP_SCAN: libc::c_ulong = 0xc060_6610;

    const PAGE_IS_WRITTEN: u64 = 1 << 1;
    const PM_SCAN_WP_MATCHING: u64 = 1 << 0;
    const PM_SCAN_CHECK_WPASYNC: u64 = 1 << 1;

    #[repr(C)]
    struct UffdioApi {
        api: u64,
        features: u64,
        ioctls: u64,
    }

    #[repr(C)]
    struct UffdioRange {
        start: u64,
        len: u64,
    }

    #[repr(C)]
    struct UffdioRegister {
        range: UffdioRange,
        mode: u64,
        ioctls: u64,
    }

    #[repr(C)]
    struct UffdioWriteprotect {
        range: UffdioRange,
        mode: u64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct PageRegion {
        start: u64,
        end: u64,
        categories: u64,
    }

    #[repr(C)]
    struct PmScanArg {
        size: u64,
        flags: u64,
        start: u64,
        end: u64,
        walk_end: u64,
        vec: u64,
        vec_len: u64,
        max_pages: u64,
        category_inverted: u64,
        category_mask: u64,
        category_anyof_mask: u64,
        return_mask: u64,
    }

    pub(crate) struct DirtyTracker {
        uffd: OwnedFd,
        pagemap: OwnedFd,
        base: usize,
        len: usize,
        regions: Vec<PageRegion>,
    }

    fn ioctl<T>(fd: &OwnedFd, request: libc::c_ulong, arg: &mut T) -> std::io::Result<libc::c_int> {
        // SAFETY: each request is paired with the struct the kernel expects.
        let ret = unsafe { libc::ioctl(fd.as_raw_fd(), request, arg as *mut T) };
        if ret < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(ret)
        }
    }

    impl DirtyTracker {
        /// Starts tracking `len` bytes at `base`, which must be page aligned.
        /// Every page starts out clean.
        pub(crate) fn new(base: *mut u8, len: usize) -> Option<DirtyTracker> {
            let base = base as usize;
            let page = 4096;
            if !base.is_multiple_of(page) || !len.is_multiple_of(page) || len == 0 {
                return None;
            }
            // SAFETY: plain syscall; the returned fd is owned below.
            let fd = unsafe {
                libc::syscall(
                    libc::SYS_userfaultfd,
                    libc::O_CLOEXEC | libc::O_NONBLOCK | UFFD_USER_MODE_ONLY,
                )
            };
            if fd < 0 {
                return None;
            }
            // SAFETY: `fd` is a fresh descriptor nothing else owns.
            let uffd = unsafe { OwnedFd::from_raw_fd(fd as libc::c_int) };
            let mut api = UffdioApi {
                api: UFFD_API,
                features: UFFD_FEATURE_WP_ASYNC | UFFD_FEATURE_WP_UNPOPULATED,
                ioctls: 0,
            };
            ioctl(&uffd, UFFDIO_API, &mut api).ok()?;

            let path = c"/proc/self/pagemap";
            // SAFETY: valid C string; the fd is owned below.
            let pm = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
            if pm < 0 {
                return None;
            }
            // SAFETY: `pm` is a fresh descriptor nothing else owns.
            let pagemap = unsafe { OwnedFd::from_raw_fd(pm) };

            // Track at 4 KiB granularity: a transparent huge page would make
            // every write dirty 2 MiB. This does not affect correctness.
            // SAFETY: advisory only, on memory this game owns.
            unsafe { libc::madvise(base as *mut libc::c_void, len, libc::MADV_NOHUGEPAGE) };

            let mut reg = UffdioRegister {
                range: UffdioRange {
                    start: base as u64,
                    len: len as u64,
                },
                mode: UFFDIO_REGISTER_MODE_WP,
                ioctls: 0,
            };
            ioctl(&uffd, UFFDIO_REGISTER, &mut reg).ok()?;

            let tracker = DirtyTracker {
                uffd,
                pagemap,
                base,
                len,
                regions: vec![PageRegion::default(); 256],
            };
            tracker.protect(0, len).ok()?;
            Some(tracker)
        }

        /// Whether this tracker covers exactly `len` bytes at `base`.
        pub(crate) fn covers(&self, base: *const u8, len: usize) -> bool {
            self.base == base as usize && self.len == len
        }

        /// Write-protects `[start, end)` again (offsets into the region).
        pub(crate) fn protect(&self, start: usize, end: usize) -> std::io::Result<()> {
            let mut wp = UffdioWriteprotect {
                range: UffdioRange {
                    start: (self.base + start) as u64,
                    len: (end - start) as u64,
                },
                mode: UFFDIO_WRITEPROTECT_MODE_WP,
            };
            ioctl(&self.uffd, UFFDIO_WRITEPROTECT, &mut wp).map(|_| ())
        }

        /// Returns the ranges written since they were last protected, and
        /// protects them again.
        pub(crate) fn take_written(&mut self) -> std::io::Result<Vec<Range>> {
            let mut out = Vec::new();
            let end = (self.base + self.len) as u64;
            let mut start = self.base as u64;
            while start < end {
                let mut arg = PmScanArg {
                    size: std::mem::size_of::<PmScanArg>() as u64,
                    flags: PM_SCAN_WP_MATCHING | PM_SCAN_CHECK_WPASYNC,
                    start,
                    end,
                    walk_end: 0,
                    vec: self.regions.as_mut_ptr() as u64,
                    vec_len: self.regions.len() as u64,
                    max_pages: 0,
                    category_inverted: 0,
                    category_mask: PAGE_IS_WRITTEN,
                    category_anyof_mask: 0,
                    return_mask: PAGE_IS_WRITTEN,
                };
                let n = ioctl(&self.pagemap, PAGEMAP_SCAN, &mut arg)? as usize;
                for r in &self.regions[..n] {
                    out.push((r.start as usize - self.base, r.end as usize - self.base));
                }
                if arg.walk_end <= start {
                    return Err(std::io::Error::other("PAGEMAP_SCAN made no progress"));
                }
                start = arg.walk_end;
            }
            Ok(out)
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::Range;

    pub(crate) struct DirtyTracker;

    impl DirtyTracker {
        pub(crate) fn new(_base: *mut u8, _len: usize) -> Option<DirtyTracker> {
            None
        }
        pub(crate) fn covers(&self, _base: *const u8, _len: usize) -> bool {
            false
        }
        pub(crate) fn protect(&self, _start: usize, _end: usize) -> std::io::Result<()> {
            Ok(())
        }
        pub(crate) fn take_written(&mut self) -> std::io::Result<Vec<Range>> {
            Ok(Vec::new())
        }
    }
}

pub(crate) use imp::DirtyTracker;
