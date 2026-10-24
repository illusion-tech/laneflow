//! 在真实文件写出与首次读回之间注入故障；只在显式测试特性下编译（#1043）。

use std::{cell::RefCell, io, path::Path};

type AfterWrite = Box<dyn FnMut(&Path) -> io::Result<()>>;

thread_local! {
    static AFTER_WRITE: RefCell<Option<AfterWrite>> = const { RefCell::new(None) };
}

pub(super) fn after_write(path: &Path) -> io::Result<()> {
    AFTER_WRITE.with_borrow_mut(|hook| match hook {
        Some(hook) => hook(path),
        None => Ok(()),
    })
}

/// 仅在当前线程、`action` 的动态作用域内安装写后注入；退出（含 panic）时恢复外层注入。
/// 注入返回的 I/O 错误由 `write_verified` 按既有失败清理规则处理。
///
/// # Panics
///
/// `hook` 或 `action` 的 panic 原样传播。`hook` 不得重入 `write_verified`，否则线程局部
/// 可变借用冲突会 panic；测试注入应使用普通文件 I/O。
pub fn with_after_write<R>(
    hook: impl FnMut(&Path) -> io::Result<()> + 'static,
    action: impl FnOnce() -> R,
) -> R {
    struct Restore(Option<AfterWrite>);
    impl Drop for Restore {
        fn drop(&mut self) {
            AFTER_WRITE.set(self.0.take());
        }
    }
    let _restore = Restore(AFTER_WRITE.replace(Some(Box::new(hook))));
    action()
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use super::*;

    #[test]
    fn hook_is_thread_local_and_restored_after_nested_unwind() {
        let calls = Rc::new(Cell::new(0));
        let outer_calls = Rc::clone(&calls);
        with_after_write(
            move |_| {
                outer_calls.set(outer_calls.get() + 1);
                Ok(())
            },
            || {
                std::thread::spawn(|| after_write(Path::new("other-thread")).unwrap())
                    .join()
                    .unwrap();
                assert_eq!(calls.get(), 0);
                let outcome = std::panic::catch_unwind(|| {
                    with_after_write(
                        |_| Err(io::Error::other("inner hook")),
                        || {
                            assert!(after_write(Path::new("inner")).is_err());
                            panic!("injected nested failure");
                        },
                    );
                });
                assert!(outcome.is_err());
                after_write(Path::new("outer")).unwrap();
                assert_eq!(calls.get(), 1);
            },
        );
        after_write(Path::new("restored")).unwrap();
        assert_eq!(calls.get(), 1);
    }
}
