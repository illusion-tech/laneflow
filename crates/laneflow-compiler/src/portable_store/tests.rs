use std::{
    cell::Cell,
    fs,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::{
    sync::{Arc, Barrier},
    thread,
};

use super::*;

static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(0);

struct TestRoot(PathBuf);

impl TestRoot {
    fn new(name: &str) -> Self {
        let ordinal = NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "laneflow-portable-store-{name}-{:08x}-{ordinal:016x}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

#[test]
fn configured_root_must_exist_and_is_never_created_recursively() {
    let root = TestRoot::new("missing-root");
    fs::remove_dir(root.path()).unwrap();

    assert_eq!(
        LocalPortableObjectInstaller::try_open(root.path()).unwrap_err(),
        PortableInstallError::Io {
            operation: PortableInstallOperation::PrepareStoreRoot,
            kind: io::ErrorKind::NotFound,
        }
    );
    assert!(!root.path().exists());
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn candidate(bytes: &[u8]) -> PortableObjectCandidate {
    crate::portable_emitter::close_object(bytes.into())
}

fn staging_is_empty(store: &LocalPortableObjectInstaller) -> bool {
    fs::read_dir(&store.staging_directory)
        .unwrap()
        .next()
        .is_none()
}

#[cfg(unix)]
#[test]
fn installs_closed_bytes_only_at_digest_path_and_reuses_exact_winner() {
    let root = TestRoot::new("install-reuse");
    let store = LocalPortableObjectInstaller::try_open(root.path()).unwrap();
    let object = candidate(b"complete portable object");

    let installed = store.install_candidate(&object).unwrap();
    assert_eq!(
        installed.disposition(),
        PortableInstallDisposition::Installed
    );
    assert_eq!(installed.digest(), object.digest());
    assert_eq!(installed.byte_length(), object.byte_length());
    assert_eq!(installed.object_key(), object.object_key());
    assert_eq!(
        fs::read(store.object_path(object.object_key()).unwrap()).unwrap(),
        object.bytes()
    );
    assert!(staging_is_empty(&store));

    let reused = store.install_candidate(&object).unwrap();
    assert_eq!(reused.disposition(), PortableInstallDisposition::Reused);
    assert_eq!(reused.object_key(), object.object_key());
    assert!(staging_is_empty(&store));
}

#[cfg(unix)]
#[test]
fn installs_closed_exact_bytes_without_accepting_caller_binding() {
    let root = TestRoot::new("exact-bytes");
    let store = LocalPortableObjectInstaller::try_open(root.path()).unwrap();
    let bytes =
        include_bytes!("../../tests/fixtures/portable-v1/lfca-v1-variants/min-headless.lfca");
    let installed = store.install_exact_bytes(bytes).unwrap();
    assert_eq!(
        installed.disposition(),
        PortableInstallDisposition::Installed
    );
    assert_eq!(installed.digest(), sha256(bytes));
    assert_eq!(
        installed.byte_length(),
        ExactByteLength::new(bytes.len() as u64)
    );
    assert_eq!(
        fs::read(store.object_path(installed.object_key()).unwrap()).unwrap(),
        bytes
    );
    assert!(staging_is_empty(&store));
}

#[cfg(unix)]
#[test]
fn existing_different_bytes_never_overwrite_winner() {
    let root = TestRoot::new("different-winner");
    let store = LocalPortableObjectInstaller::try_open(root.path()).unwrap();
    let object = candidate(b"expected complete object");
    let path = store.object_path(object.object_key()).unwrap();
    fs::write(&path, b"different winner").unwrap();

    assert_eq!(
        store.install_candidate(&object),
        Err(PortableInstallError::ExistingObjectMismatch)
    );
    assert_eq!(fs::read(path).unwrap(), b"different winner");
    assert!(staging_is_empty(&store));
}

#[cfg(unix)]
#[test]
fn stale_partial_staging_is_never_observed_or_reused() {
    let root = TestRoot::new("stale-staging");
    let store = LocalPortableObjectInstaller::try_open(root.path()).unwrap();
    let stale_directory = store.staging_directory.join("crashed-publisher");
    fs::create_dir(&stale_directory).unwrap();
    let stale_file = stale_directory.join("object.closed");
    fs::write(&stale_file, b"partial").unwrap();
    let object = candidate(b"fresh complete retry");

    let installed = store.install_candidate(&object).unwrap();
    assert_eq!(
        installed.disposition(),
        PortableInstallDisposition::Installed
    );
    assert_eq!(fs::read(&stale_file).unwrap(), b"partial");
    assert_eq!(
        fs::read(store.object_path(object.object_key()).unwrap()).unwrap(),
        object.bytes()
    );
    assert_eq!(fs::read_dir(&store.staging_directory).unwrap().count(), 1);
}

#[test]
fn binding_checks_precede_all_staging_and_path_mapping() {
    let root = TestRoot::new("binding");
    let store = LocalPortableObjectInstaller::try_open(root.path()).unwrap();
    let bytes = b"bound object";
    let digest = sha256(bytes);

    assert_eq!(
        store.install_bound_object(
            bytes,
            digest,
            "sha256/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            &NativeAtomicInstall,
        ),
        Err(PortableInstallError::NonCanonicalObjectKey)
    );
    let key = object_key(digest);
    let mut wrong_digest_bytes = digest.into_bytes();
    wrong_digest_bytes[0] ^= 1;
    let wrong_digest = Sha256Digest::from_bytes(wrong_digest_bytes);
    assert_eq!(
        store.install_bound_object(
            bytes,
            wrong_digest,
            &object_key(wrong_digest),
            &NativeAtomicInstall
        ),
        Err(PortableInstallError::ObjectDigestMismatch)
    );
    assert!(!store.object_path(&key).unwrap().exists());
    assert!(staging_is_empty(&store));
}

#[cfg(unix)]
#[test]
fn concurrent_same_bytes_have_one_winner_and_only_safe_reuse() {
    let root = TestRoot::new("concurrent-same");
    let store = Arc::new(LocalPortableObjectInstaller::try_open(root.path()).unwrap());
    let object = Arc::new(candidate(b"same complete object from every publisher"));
    let barrier = Arc::new(Barrier::new(8));
    let mut threads = Vec::new();
    for _ in 0..8 {
        let store = Arc::clone(&store);
        let object = Arc::clone(&object);
        let barrier = Arc::clone(&barrier);
        threads.push(thread::spawn(move || {
            barrier.wait();
            store.install_candidate(&object).unwrap().disposition()
        }));
    }

    let dispositions: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert_eq!(
        dispositions
            .iter()
            .filter(|value| **value == PortableInstallDisposition::Installed)
            .count(),
        1
    );
    assert_eq!(
        dispositions
            .iter()
            .filter(|value| **value == PortableInstallDisposition::Reused)
            .count(),
        7
    );
    assert_eq!(
        fs::read(store.object_path(object.object_key()).unwrap()).unwrap(),
        object.bytes()
    );
    assert!(staging_is_empty(&store));
}

struct UnsupportedPlatform;

impl AtomicInstallPlatform for UnsupportedPlatform {
    fn link_no_replace(
        &self,
        _staging_file: &Path,
        _object_path: &Path,
    ) -> Result<AtomicLinkOutcome, PortableInstallError> {
        Err(PortableInstallError::AtomicInstallUnsupported)
    }

    fn sync_directory(
        &self,
        _directory: &Path,
        _operation: PortableInstallOperation,
    ) -> Result<(), PortableInstallError> {
        panic!("directory sync must not run after unsupported install")
    }
}

#[test]
fn unsupported_atomic_primitive_never_streams_to_final_path() {
    let root = TestRoot::new("unsupported");
    let store = LocalPortableObjectInstaller::try_open(root.path()).unwrap();
    let object = candidate(b"must remain staged only");

    assert_eq!(
        store.install_bound_object(
            object.bytes(),
            object.digest(),
            object.object_key(),
            &UnsupportedPlatform,
        ),
        Err(PortableInstallError::AtomicInstallUnsupported)
    );
    assert!(!store.object_path(object.object_key()).unwrap().exists());
    assert!(staging_is_empty(&store));
}

struct DifferentWinnerPlatform;

impl AtomicInstallPlatform for DifferentWinnerPlatform {
    fn link_no_replace(
        &self,
        _staging_file: &Path,
        object_path: &Path,
    ) -> Result<AtomicLinkOutcome, PortableInstallError> {
        fs::write(object_path, b"concurrent different winner").unwrap();
        Ok(AtomicLinkOutcome::AlreadyExists)
    }

    fn sync_directory(
        &self,
        _directory: &Path,
        _operation: PortableInstallOperation,
    ) -> Result<(), PortableInstallError> {
        panic!("loser must not sync winner directory")
    }
}

struct ReusedWinnerPlatform {
    sync_calls: Cell<usize>,
}

impl AtomicInstallPlatform for ReusedWinnerPlatform {
    fn link_no_replace(
        &self,
        staging_file: &Path,
        object_path: &Path,
    ) -> Result<AtomicLinkOutcome, PortableInstallError> {
        fs::hard_link(staging_file, object_path).unwrap();
        Ok(AtomicLinkOutcome::AlreadyExists)
    }

    fn sync_directory(
        &self,
        _directory: &Path,
        operation: PortableInstallOperation,
    ) -> Result<(), PortableInstallError> {
        assert_eq!(operation, PortableInstallOperation::SyncObjectDirectory);
        self.sync_calls.set(self.sync_calls.get() + 1);
        Ok(())
    }
}

#[test]
fn reused_winner_passes_the_same_directory_durability_barrier() {
    let root = TestRoot::new("reused-sync");
    let store = LocalPortableObjectInstaller::try_open(root.path()).unwrap();
    let object = candidate(b"winner visible before another publisher syncs it");
    let platform = ReusedWinnerPlatform {
        sync_calls: Cell::new(0),
    };

    let installation = store
        .install_bound_object(
            object.bytes(),
            object.digest(),
            object.object_key(),
            &platform,
        )
        .unwrap();

    assert_eq!(
        installation.disposition(),
        PortableInstallDisposition::Reused
    );
    assert_eq!(platform.sync_calls.get(), 1);
}

#[test]
fn concurrent_different_winner_is_compared_and_never_replaced() {
    let root = TestRoot::new("concurrent-different");
    let store = LocalPortableObjectInstaller::try_open(root.path()).unwrap();
    let object = candidate(b"losing complete object");

    assert_eq!(
        store.install_bound_object(
            object.bytes(),
            object.digest(),
            object.object_key(),
            &DifferentWinnerPlatform,
        ),
        Err(PortableInstallError::ExistingObjectMismatch)
    );
    assert_eq!(
        fs::read(store.object_path(object.object_key()).unwrap()).unwrap(),
        b"concurrent different winner"
    );
    assert!(staging_is_empty(&store));
}

struct SyncFailurePlatform;

impl AtomicInstallPlatform for SyncFailurePlatform {
    fn link_no_replace(
        &self,
        staging_file: &Path,
        object_path: &Path,
    ) -> Result<AtomicLinkOutcome, PortableInstallError> {
        fs::hard_link(staging_file, object_path).unwrap();
        Ok(AtomicLinkOutcome::Installed)
    }

    fn sync_directory(
        &self,
        _directory: &Path,
        operation: PortableInstallOperation,
    ) -> Result<(), PortableInstallError> {
        Err(PortableInstallError::Io {
            operation,
            kind: io::ErrorKind::Other,
        })
    }
}

#[test]
fn directory_sync_failure_reports_error_but_can_only_leave_complete_object() {
    let root = TestRoot::new("sync-failure");
    let store = LocalPortableObjectInstaller::try_open(root.path()).unwrap();
    let object = candidate(b"fully synced before atomic install");

    assert_eq!(
        store.install_bound_object(
            object.bytes(),
            object.digest(),
            object.object_key(),
            &SyncFailurePlatform,
        ),
        Err(PortableInstallError::Io {
            operation: PortableInstallOperation::SyncObjectDirectory,
            kind: io::ErrorKind::Other,
        })
    );
    assert_eq!(
        fs::read(store.object_path(object.object_key()).unwrap()).unwrap(),
        object.bytes()
    );
    assert!(staging_is_empty(&store));
}

#[cfg(windows)]
#[test]
fn native_windows_install_fails_closed_before_exposing_a_final_path() {
    let root = TestRoot::new("windows-unsupported");
    let store = LocalPortableObjectInstaller::try_open(root.path()).unwrap();
    let object = candidate(b"durability must be established before publication");

    assert_eq!(
        store.install_candidate(&object),
        Err(PortableInstallError::AtomicInstallUnsupported)
    );
    assert!(!store.object_path(object.object_key()).unwrap().exists());
    assert!(staging_is_empty(&store));
}
