//! 后发射检查可接纳的不可变对象来源能力。

use laneflow_static_contract::ExactByteLength;

pub(crate) mod private {
    use super::ObjectSourceError;

    pub trait SealedImmutableBacking {
        fn contiguous_bytes(&self) -> Result<&[u8], ObjectSourceError>;
    }
}

/// 对象来源读取与 immutable backing 核对的稳定失败分类。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObjectSourceError {
    /// 请求范围越过来源在 finish 时固定的 exact length。
    OutOfBounds,
    /// 底层来源读取失败。
    ReadFailed,
    /// 来源 identity、exact length 或不可变性保证发生漂移。
    BackingChanged,
}

/// 可重复读取、长度固定且不能由 safe downstream 自行实现的对象来源。
///
/// 公开接口有意不暴露路径、原始文件句柄、可写映射或连续 backing。完整 slice 与 crate
/// 登记的 closed staged backing 共享此入口，因此 checker 不建立平行信任路径。
pub trait BoundedReReadableObjectSource: private::SealedImmutableBacking {
    #[must_use]
    fn exact_byte_length(&self) -> ExactByteLength;

    fn read_exact_at(&self, offset: u64, destination: &mut [u8]) -> Result<(), ObjectSourceError>;
}

impl private::SealedImmutableBacking for &[u8] {
    fn contiguous_bytes(&self) -> Result<&[u8], ObjectSourceError> {
        Ok(self)
    }
}

impl BoundedReReadableObjectSource for &[u8] {
    fn exact_byte_length(&self) -> ExactByteLength {
        ExactByteLength::new(self.len() as u64)
    }

    fn read_exact_at(&self, offset: u64, destination: &mut [u8]) -> Result<(), ObjectSourceError> {
        let start = usize::try_from(offset).map_err(|_| ObjectSourceError::OutOfBounds)?;
        let end = start
            .checked_add(destination.len())
            .ok_or(ObjectSourceError::OutOfBounds)?;
        let source = self.get(start..end).ok_or(ObjectSourceError::OutOfBounds)?;
        destination.copy_from_slice(source);
        Ok(())
    }
}

impl<const N: usize> private::SealedImmutableBacking for &[u8; N] {
    fn contiguous_bytes(&self) -> Result<&[u8], ObjectSourceError> {
        Ok(&self[..])
    }
}

impl<const N: usize> BoundedReReadableObjectSource for &[u8; N] {
    fn exact_byte_length(&self) -> ExactByteLength {
        ExactByteLength::new(N as u64)
    }

    fn read_exact_at(&self, offset: u64, destination: &mut [u8]) -> Result<(), ObjectSourceError> {
        (&self[..]).read_exact_at(offset, destination)
    }
}

impl<S> private::SealedImmutableBacking for &S
where
    S: BoundedReReadableObjectSource + ?Sized,
{
    fn contiguous_bytes(&self) -> Result<&[u8], ObjectSourceError> {
        private::SealedImmutableBacking::contiguous_bytes(*self)
    }
}

impl<S> BoundedReReadableObjectSource for &S
where
    S: BoundedReReadableObjectSource + ?Sized,
{
    fn exact_byte_length(&self) -> ExactByteLength {
        (*self).exact_byte_length()
    }

    fn read_exact_at(&self, offset: u64, destination: &mut [u8]) -> Result<(), ObjectSourceError> {
        (*self).read_exact_at(offset, destination)
    }
}

pub(crate) fn contiguous_bytes<S>(source: &S) -> Result<&[u8], ObjectSourceError>
where
    S: BoundedReReadableObjectSource + ?Sized,
{
    private::SealedImmutableBacking::contiguous_bytes(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_source_reads_exact_ranges_and_closes_bounds() {
        let source = &[1_u8, 2, 3, 4][..];
        let mut destination = [0_u8; 2];
        source.read_exact_at(1, &mut destination).unwrap();
        assert_eq!(destination, [2, 3]);
        assert_eq!(source.exact_byte_length(), ExactByteLength::new(4));
        assert_eq!(
            source.read_exact_at(3, &mut destination),
            Err(ObjectSourceError::OutOfBounds)
        );
    }
}
