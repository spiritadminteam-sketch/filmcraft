//! Waveform peaks kept on disk in the media cache (`<Media Cache>/Peaks`), and a gate that runs
//! one waveform job at a time.
//!
//! Without them every project open decoded the whole sound of every file again, all files at
//! once, each in its own thread, through the very decoders playback was using: the disk and the
//! decoders were busy for the first seconds and playback crackled and stuttered. Now a file's
//! peaks are worked out once (one job at a time, through a reader of their own) and read back
//! from the cache on the next open.

use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, PoisonError};

const MAGIC: &[u8; 8] = b"FCPEAK01";
/// A cache file never claims more peaks than this (24 h of sound at one peak per 256 samples at
/// 48 kHz is ~16 M); anything bigger is treated as damaged.
const MAX_PEAKS: u64 = 32_000_000;

/// FNV-1a: a stable hash for cache file names (std's hasher may change between Rust versions).
fn fnv1a(parts: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for b in *p {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        // separator, so ("ab","c") and ("a","bc") differ
        h ^= 0xff;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// The cache file for audio stream `stream` of the media file `media`, or `None` when the file
/// can't be looked at. Its name changes when the file's size or modification time does, so an
/// edited or replaced file gets new peaks.
pub fn cache_path(cache_dir: &Path, media: &str, stream: usize) -> Option<PathBuf> {
    let md = std::fs::metadata(media).ok()?;
    let modified = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
    let h = fnv1a(&[media.as_bytes(), &md.len().to_le_bytes(), &modified.to_le_bytes(), &(stream as u64).to_le_bytes()]);
    Some(cache_dir.join("Peaks").join(format!("{h:016x}.fcpeak")))
}

/// Peaks read back from `path`, or `None` when it is missing or not a whole cache file.
pub fn read(path: &Path) -> Option<Vec<(f32, f32)>> {
    let b = std::fs::read(path).ok()?;
    let head = b.get(..16)?;
    if head.get(..8)? != MAGIC {
        return None;
    }
    let n = u64::from_le_bytes(head.get(8..16)?.try_into().ok()?);
    if n > MAX_PEAKS {
        return None;
    }
    let body = b.get(16..)?;
    if body.len() as u64 != n.checked_mul(8)? {
        return None;
    }
    let f = |c: &[u8]| c.try_into().ok().map(f32::from_le_bytes);
    body.chunks_exact(8).map(|c| Some((f(c.get(..4)?)?, f(c.get(4..)?)?))).collect()
}

/// Store `peaks` at `path` (best effort: a cache that can't be written is only slower).
pub fn write(path: &Path, peaks: &[(f32, f32)]) {
    let Some(dir) = path.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let mut b = Vec::with_capacity(16 + peaks.len() * 8);
    b.extend_from_slice(MAGIC);
    b.extend_from_slice(&(peaks.len() as u64).to_le_bytes());
    for (lo, hi) in peaks {
        b.extend_from_slice(&lo.to_le_bytes());
        b.extend_from_slice(&hi.to_le_bytes());
    }
    // written beside, then renamed: a crash mid-write never leaves a half file under the real name
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, &b).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// How many waveform jobs may decode at once.
const SLOTS: usize = 1;

static RUNNING: Mutex<usize> = Mutex::new(0);
static FREED: Condvar = Condvar::new();

/// A turn to run a waveform job; the next job starts when it is dropped.
pub struct Turn(());

impl Drop for Turn {
    fn drop(&mut self) {
        let mut n = RUNNING.lock().unwrap_or_else(PoisonError::into_inner);
        *n = n.saturating_sub(1);
        FREED.notify_one();
    }
}

/// Wait for a turn to run a waveform job.
pub fn turn() -> Turn {
    let mut n = RUNNING.lock().unwrap_or_else(PoisonError::into_inner);
    while *n >= SLOTS {
        n = FREED.wait(n).unwrap_or_else(PoisonError::into_inner);
    }
    *n += 1;
    Turn(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fc-peaks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn peaks_round_trip_and_follow_the_file() {
        let d = dir("rt");
        let media = d.join("clip.wav");
        std::fs::write(&media, b"one").unwrap();
        let m = media.to_string_lossy().into_owned();
        let p = cache_path(&d, &m, 0).unwrap();
        assert_ne!(Some(p.clone()), cache_path(&d, &m, 1), "each stream has its own file");
        assert!(read(&p).is_none());
        let peaks = vec![(-0.5, 0.25), (0.0, 0.0), (-1.0, 1.0)];
        write(&p, &peaks);
        assert_eq!(read(&p), Some(peaks));
        // a changed file is a new cache entry
        std::fs::write(&media, b"longer now").unwrap();
        assert_ne!(cache_path(&d, &m, 0), Some(p));
        assert!(cache_path(&d, &d.join("missing.wav").to_string_lossy(), 0).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn damaged_cache_files_are_ignored() {
        let d = dir("bad");
        let p = d.join("x.fcpeak");
        for bytes in [&b""[..], b"FCPEAK01", b"FCPEAK01\x05\0\0\0\0\0\0\0abc", b"NOTPEAKS\0\0\0\0\0\0\0\0", b"FCPEAK01\xff\xff\xff\xff\xff\xff\xff\xff"] {
            std::fs::write(&p, bytes).unwrap();
            assert!(read(&p).is_none(), "{bytes:?}");
        }
        std::fs::write(&p, b"FCPEAK01\0\0\0\0\0\0\0\0").unwrap();
        assert_eq!(read(&p), Some(Vec::new()));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn one_waveform_job_runs_at_a_time() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NOW: AtomicUsize = AtomicUsize::new(0);
        static MAX: AtomicUsize = AtomicUsize::new(0);
        let hs: Vec<_> = (0..6)
            .map(|_| {
                std::thread::spawn(|| {
                    let _t = turn();
                    let n = NOW.fetch_add(1, Ordering::SeqCst) + 1;
                    MAX.fetch_max(n, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    NOW.fetch_sub(1, Ordering::SeqCst);
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        assert_eq!(MAX.load(Ordering::SeqCst), SLOTS);
    }
}
