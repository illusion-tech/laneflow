//! 按角色命名的 32 字节值类型的公共实现（ADR 0033 §4：类型名不写入算法）。

/// 定义 `#[repr(transparent)]` 的 32 字节值类型：原样封装与取出字节、小写十六进制
/// 格式化，以及 `类型名(hex)` 形式的 `Debug`。
macro_rules! bytes32_type {
    ($(#[$meta:meta])* $vis:vis struct $name:ident;) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
        #[repr(transparent)]
        $vis struct $name([u8; 32]);

        impl $name {
            /// 原样封装 32 个字节，不执行 hash 或信任验证。
            #[must_use]
            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            /// 借用原始字节。
            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }

            /// 取出原始字节。
            #[must_use]
            pub const fn into_bytes(self) -> [u8; 32] {
                self.0
            }
        }

        impl ::std::fmt::LowerHex for $name {
            fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                for byte in self.0 {
                    write!(formatter, "{byte:02x}")?;
                }
                Ok(())
            }
        }

        impl ::std::fmt::Debug for $name {
            fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                write!(formatter, concat!(stringify!($name), "({:x})"), self)
            }
        }
    };
}

pub(crate) use bytes32_type;
