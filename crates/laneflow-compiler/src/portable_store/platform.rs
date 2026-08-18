use std::path::Path;

#[cfg(unix)]
use std::{fs, io};

use super::{PortableInstallError, PortableInstallOperation};

#[cfg_attr(not(unix), allow(dead_code))]
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

    fn sync_directory(
        &self,
        directory: &Path,
        operation: PortableInstallOperation,
    ) -> Result<(), PortableInstallError>;
}

pub(super) struct NativeAtomicInstall;

#[cfg(unix)]
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

    fn sync_directory(
        &self,
        directory: &Path,
        operation: PortableInstallOperation,
    ) -> Result<(), PortableInstallError> {
        sync_directory(directory, operation)
    }
}

#[cfg(windows)]
impl AtomicInstallPlatform for NativeAtomicInstall {
    fn link_no_replace(
        &self,
        _staging_file: &Path,
        _object_path: &Path,
    ) -> Result<AtomicLinkOutcome, PortableInstallError> {
        // `CreateHardLinkW` proves atomic visibility, not persistence of the
        // directory entry. Until a reviewed safe backend can establish that
        // durability boundary, Windows publication must fail before exposing
        // a final object path.
        Err(PortableInstallError::AtomicInstallUnsupported)
    }

    fn sync_directory(
        &self,
        _directory: &Path,
        _operation: PortableInstallOperation,
    ) -> Result<(), PortableInstallError> {
        Err(PortableInstallError::AtomicInstallUnsupported)
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

    fn sync_directory(
        &self,
        _directory: &Path,
        _operation: PortableInstallOperation,
    ) -> Result<(), PortableInstallError> {
        Err(PortableInstallError::AtomicInstallUnsupported)
    }
}

#[cfg(unix)]
fn atomic_link_is_unsupported(error: &io::Error) -> bool {
    if matches!(
        error.kind(),
        io::ErrorKind::Unsupported | io::ErrorKind::CrossesDevices
    ) {
        return true;
    }

    false
}

#[cfg(unix)]
fn sync_directory(
    directory: &Path,
    operation: PortableInstallOperation,
) -> Result<(), PortableInstallError> {
    fs::File::open(directory)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| PortableInstallError::Io {
            operation,
            kind: error.kind(),
        })
}
