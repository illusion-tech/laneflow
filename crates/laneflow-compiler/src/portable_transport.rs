//! 可移植对象进入 hash/预检前的有界 transport 读取。
//!
//! 本模块只关闭 transport 长度边界，不把读取成功包装成 checked/validated view。
//! 调用方仍须把返回的 exact bytes 交给 `laneflow-format` 完成对象结构和值域预检。

use std::io::{self, Read};

use laneflow_format::FormatLimits;

const UNKNOWN_LENGTH_READ_BUFFER_BYTES: usize = 64 * 1024;

struct BoundedReadBuffer {
    bytes: Vec<u8>,
    max_capacity: usize,
}

impl BoundedReadBuffer {
    fn new(max_capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(max_capacity.min(UNKNOWN_LENGTH_READ_BUFFER_BYTES)),
            max_capacity,
        }
    }

    fn extend_from_slice(&mut self, chunk: &[u8]) -> Result<(), PortableReadError> {
        let required = self
            .bytes
            .len()
            .checked_add(chunk.len())
            .ok_or(PortableReadError::ArithmeticOverflow)?;
        if required > self.max_capacity {
            return Err(PortableReadError::ArithmeticOverflow);
        }
        if required > self.bytes.capacity() {
            let doubled = self
                .bytes
                .capacity()
                .checked_mul(2)
                .unwrap_or(self.max_capacity);
            let target = doubled.max(required).min(self.max_capacity);
            self.bytes.reserve_exact(target - self.bytes.len());
        }
        self.bytes.extend_from_slice(chunk);
        Ok(())
    }

    fn len(&self) -> usize {
        self.bytes.len()
    }

    fn into_boxed_slice(self) -> Box<[u8]> {
        self.bytes.into_boxed_slice()
    }
}

/// 有界 transport 读取失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortableReadError {
    /// 已知或实际读取长度超过调用方单对象上限。
    LimitExceeded { actual: u64, limit: u64 },
    /// 已知 transport 长度与实际可读取字节数不一致。
    LengthMismatch { declared: u64, actual: u64 },
    /// 长度不能安全转换或累计。
    ArithmeticOverflow,
    /// 底层 transport 读取失败。
    Io(io::ErrorKind),
}

impl From<io::Error> for PortableReadError {
    fn from(value: io::Error) -> Self {
        Self::Io(value.kind())
    }
}

/// 在任何读取或分配前检查已知 transport 长度，再读取恰好这些字节。
///
/// 调用方拥有 transport framing；本函数不会探测或消费声明边界后的下一个字节。返回 bytes
/// 仍须由 format preflight 把 wire `objectByteLength` 与该 exact 长度比较。
///
/// # Errors
///
/// 长度超过调用方上限、与 transport 实际长度不一致、算术转换失败或底层读取失败时返回错误。
pub fn read_portable_object_known_length<R: Read>(
    reader: &mut R,
    declared_length: u64,
    limits: FormatLimits,
) -> Result<Box<[u8]>, PortableReadError> {
    let limit = limits.max_object_bytes();
    if declared_length > limit {
        return Err(PortableReadError::LimitExceeded {
            actual: declared_length,
            limit,
        });
    }

    let allocation_length =
        usize::try_from(declared_length).map_err(|_| PortableReadError::ArithmeticOverflow)?;
    let mut bytes = vec![0_u8; allocation_length];
    let mut filled = 0_usize;
    while filled < bytes.len() {
        match reader.read(&mut bytes[filled..]) {
            Ok(0) => {
                return Err(PortableReadError::LengthMismatch {
                    declared: declared_length,
                    actual: u64::try_from(filled)
                        .map_err(|_| PortableReadError::ArithmeticOverflow)?,
                });
            }
            Ok(read) => {
                filled = filled
                    .checked_add(read)
                    .ok_or(PortableReadError::ArithmeticOverflow)?;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }

    Ok(bytes.into_boxed_slice())
}

/// 读取未知长度 transport，但绝不读取超过调用方上限后的第一个字节。
///
/// # Errors
///
/// 观察到 `maxObjectBytes + 1`、长度转换失败或底层读取失败时返回错误。
pub fn read_portable_object_to_end<R: Read>(
    reader: &mut R,
    limits: FormatLimits,
) -> Result<Box<[u8]>, PortableReadError> {
    let limit = limits.max_object_bytes();
    let bounded_length = limit
        .checked_add(1)
        .ok_or(PortableReadError::ArithmeticOverflow)?;
    let bounded_capacity =
        usize::try_from(bounded_length).map_err(|_| PortableReadError::ArithmeticOverflow)?;
    let mut bytes = BoundedReadBuffer::new(bounded_capacity);
    let mut read_buffer = [0_u8; UNKNOWN_LENGTH_READ_BUFFER_BYTES];
    loop {
        let remaining = bounded_capacity
            .checked_sub(bytes.len())
            .ok_or(PortableReadError::ArithmeticOverflow)?;
        if remaining == 0 {
            break;
        }
        let read_length = remaining.min(read_buffer.len());
        match reader.read(&mut read_buffer[..read_length]) {
            Ok(0) => break,
            Ok(read) => bytes.extend_from_slice(&read_buffer[..read])?,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
    let actual = u64::try_from(bytes.len()).map_err(|_| PortableReadError::ArithmeticOverflow)?;
    if actual > limit {
        return Err(PortableReadError::LimitExceeded { actual, limit });
    }
    Ok(bytes.into_boxed_slice())
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, io::Cursor, rc::Rc};

    use laneflow_format::{FormatLimitConfig, FormatLimits};

    use super::*;

    struct CountingReader<R> {
        inner: R,
        calls: Rc<Cell<u64>>,
        bytes_read: Rc<Cell<u64>>,
    }

    type CountedCursor = (
        CountingReader<Cursor<Vec<u8>>>,
        Rc<Cell<u64>>,
        Rc<Cell<u64>>,
    );

    impl<R: Read> Read for CountingReader<R> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.calls.set(self.calls.get() + 1);
            let read = self.inner.read(buffer)?;
            self.bytes_read.set(
                self.bytes_read
                    .get()
                    .checked_add(u64::try_from(read).unwrap())
                    .unwrap(),
            );
            Ok(read)
        }
    }

    fn counting_reader(bytes: Vec<u8>) -> CountedCursor {
        let calls = Rc::new(Cell::new(0));
        let bytes_read = Rc::new(Cell::new(0));
        (
            CountingReader {
                inner: Cursor::new(bytes),
                calls: Rc::clone(&calls),
                bytes_read: Rc::clone(&bytes_read),
            },
            calls,
            bytes_read,
        )
    }

    fn limits(max_object_bytes: u64) -> FormatLimits {
        let mut config = FormatLimitConfig::V1_HARD;
        config.max_object_bytes = max_object_bytes;
        FormatLimits::try_new(config).unwrap()
    }

    #[test]
    fn known_over_limit_rejects_before_read_or_allocation() {
        let (mut reader, calls, bytes_read) = counting_reader(vec![0; 5]);
        assert_eq!(
            read_portable_object_known_length(&mut reader, 5, limits(4)),
            Err(PortableReadError::LimitExceeded {
                actual: 5,
                limit: 4,
            })
        );
        assert_eq!(calls.get(), 0);
        assert_eq!(bytes_read.get(), 0);
    }

    #[test]
    fn known_length_requires_all_declared_bytes_without_consuming_the_next_frame() {
        let mut exact = Cursor::new(b"abcd".to_vec());
        assert_eq!(
            read_portable_object_known_length(&mut exact, 4, limits(4)).unwrap(),
            Box::<[u8]>::from(&b"abcd"[..])
        );

        let mut short = Cursor::new(b"abc".to_vec());
        assert_eq!(
            read_portable_object_known_length(&mut short, 4, limits(4)),
            Err(PortableReadError::LengthMismatch {
                declared: 4,
                actual: 3,
            })
        );

        let mut framed = Cursor::new(b"abcde".to_vec());
        assert_eq!(
            read_portable_object_known_length(&mut framed, 4, limits(4)).unwrap(),
            Box::<[u8]>::from(&b"abcd"[..])
        );
        assert_eq!(framed.position(), 4);
    }

    #[test]
    fn unknown_length_accepts_boundary_and_stops_at_limit_plus_one() {
        let mut exact = Cursor::new(vec![7; 4]);
        assert_eq!(
            read_portable_object_to_end(&mut exact, limits(4)).unwrap(),
            vec![7; 4].into_boxed_slice()
        );

        let (mut over, _calls, bytes_read) = counting_reader(vec![7; 32]);
        assert_eq!(
            read_portable_object_to_end(&mut over, limits(4)),
            Err(PortableReadError::LimitExceeded {
                actual: 5,
                limit: 4,
            })
        );
        assert_eq!(bytes_read.get(), 5);
    }

    #[test]
    fn unknown_length_growth_never_reserves_past_the_checked_boundary() {
        let max_capacity = 1_048_577;
        let mut bytes = BoundedReadBuffer::new(max_capacity);
        let chunk = [0_u8; 16 * 1024];
        while bytes.len() < max_capacity {
            let remaining = max_capacity - bytes.len();
            bytes
                .extend_from_slice(&chunk[..remaining.min(chunk.len())])
                .unwrap();
            assert!(bytes.bytes.capacity() <= max_capacity);
        }
        assert_eq!(bytes.len(), max_capacity);
    }
}
