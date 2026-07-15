use std::fs::{self, File, OpenOptions, TryLockError};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};

pub fn temporary_lock_path(primary_path: Option<&Path>) -> PathBuf {
    let suffix = primary_path.map(|path| {
        let mut hasher = DefaultHasher::new();
        path.hash(&mut hasher);
        format!("-{:x}", hasher.finish())
    });
    std::env::temp_dir().join(format!(
        "keypeek-instance{}.lock",
        suffix.as_deref().unwrap_or_default()
    ))
}

pub struct InstanceLock {
    _file: File,
}

impl InstanceLock {
    pub fn acquire(path: &Path) -> io::Result<Option<Self>> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;

        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }

    pub fn acquire_with_fallback(
        primary_path: Option<&Path>,
        fallback_path: &Path,
    ) -> io::Result<Option<Self>> {
        if let Some(primary_path) = primary_path {
            match Self::acquire(primary_path) {
                Ok(result) => return Ok(result),
                Err(error) => {
                    eprintln!(
                        "KeyPeek: could not use instance lock at {} ({error}); trying {}",
                        primary_path.display(),
                        fallback_path.display()
                    );
                }
            }
        }

        Self::acquire(fallback_path)
    }
}

#[cfg(test)]
mod tests {
    use super::InstanceLock;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);

    fn lock_path() -> std::path::PathBuf {
        let id = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("keypeek-instance-{}-{id}.lock", std::process::id()))
    }

    #[test]
    fn permits_only_one_live_lock() {
        let path = lock_path();
        let first = InstanceLock::acquire(&path).unwrap().unwrap();

        assert!(InstanceLock::acquire(&path).unwrap().is_none());

        drop(first);
        assert!(InstanceLock::acquire(&path).unwrap().is_some());

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn falls_back_when_primary_lock_is_unavailable() {
        let unusable_parent = lock_path();
        fs::write(&unusable_parent, "not a directory").unwrap();
        let primary_path = unusable_parent.join("instance.lock");
        let fallback_path = lock_path();

        assert!(
            InstanceLock::acquire_with_fallback(Some(&primary_path), &fallback_path)
                .unwrap()
                .is_some()
        );

        fs::remove_file(unusable_parent).unwrap();
        fs::remove_file(fallback_path).unwrap();
    }
}
