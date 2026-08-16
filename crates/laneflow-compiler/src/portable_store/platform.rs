use std::{fs, io, path::Path};

use super::{PortableInstallError, PortableInstallOperation};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AtomicLinkOutcome {
    Installed,
    AlreadyExists,
}

pub(super) trait AtomicInstallPlatform {
    fn link_no_replace(
        &self,
        staging_file: &Path,
        object_path: &Path,
    ) -> Result<AtomicLinkOutcome, PortableInstallError>;

    fn sync_object_directory(&self, object_directory: &Path) -> Result<(), PortableInstallError>;
}

pub(super) struct NativeAtomicInstall;

#[cfg(any(unix, windows))]
impl AtomicInstallPlatform for NativeAtomicInstall {
    fn link_no_replace(
        &self,
        staging_file: &Path,
        object_path: &Path,
    ) -> Result<AtomicLinkOutcome, PortableInstallError> {
        match fs::hard_link(staging_file, object_path) {
            Ok(()) => Ok(AtomicLinkOutcome::Installed),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                Ok(AtomicLinkOutcome::AlreadyExists)
            }
            Err(error) if atomic_link_is_unsupported(&error) => {
                Err(PortableInstallError::AtomicInstallUnsupported)
            }
            Err(error) => Err(PortableInstallError::Io {
                operation: PortableInstallOperation::InstallNoReplace,
                kind: error.kind(),
            }),
        }
    }

    fn sync_object_directory(&self, object_directory: &Path) -> Result<(), PortableInstallError> {
        sync_object_directory(object_directory)
    }
}

#[cfg(not(any(unix, windows)))]
impl AtomicInstallPlatform for NativeAtomicInstall {
    fn link_no_replace(
        &self,
        _staging_file: &Path,
        _object_path: &Path,
    ) -> Result<AtomicLinkOutcome, PortableInstallError> {
        Err(PortableInstallError::AtomicInstallUnsupported)
    }

    fn sync_object_directory(&self, _object_directory: &Path) -> Result<(), PortableInstallError> {
        Err(PortableInstallError::AtomicInstallUnsupported)
    }
}

#[cfg(any(unix, windows))]
fn atomic_link_is_unsupported(error: &io::Error) -> bool {
    if matches!(
        error.kind(),
        io::ErrorKind::Unsupported | io::ErrorKind::CrossesDevices
    ) {
        return true;
    }

    #[cfg(windows)]
    {
        // Win32: ERROR_INVALID_FUNCTION / ERROR_NOT_SUPPORTED are the stable
        // signals for filesystems that cannot create hard links. A staging
        // junction that crosses volumes can surface ERROR_NOT_SAME_DEVICE.
        matches!(error.raw_os_error(), Some(1 | 17 | 50))
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(unix)]
fn sync_object_directory(object_directory: &Path) -> Result<(), PortableInstallError> {
    fs::File::open(object_directory)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| PortableInstallError::Io {
            operation: PortableInstallOperation::SyncObjectDirectory,
            kind: error.kind(),
        })
}

#[cfg(windows)]
fn sync_object_directory(_object_directory: &Path) -> Result<(), PortableInstallError> {
    // `CreateHardLinkW` is available only on filesystems whose link operation
    // publishes one complete directory entry. The staged file data was synced
    // before linking; after a crash the journal can therefore expose the
    // complete link or no link, never a streamed/partial destination. Rust's
    // safe standard library has no portable directory-fsync contract on
    // Windows, so there is no additional fallible operation at this point.
    Ok(())
}
