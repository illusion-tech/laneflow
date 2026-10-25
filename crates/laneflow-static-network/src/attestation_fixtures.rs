//! 认证比较阶段夹具；默认发布构建不包含此模块或调用点。
//! 独立拥有的修订视图不等于在用 backing 改写、物理共页或硬件故障注入。

use std::{cell::RefCell, collections::VecDeque, sync::Arc};

use crate::SharedNetworkRevision;

pub(crate) type Comparison = (
    Option<Arc<SharedNetworkRevision>>,
    Option<Arc<SharedNetworkRevision>>,
);

#[derive(Default)]
struct Injection {
    target: Option<Arc<SharedNetworkRevision>>,
    views: VecDeque<Comparison>,
    invalid: bool,
}

thread_local! {
    static COMPARISONS: RefCell<Injection> = RefCell::new(Injection::default());
}

/// 翻转唯一可变修订的指定车道长度最低位；类型系统禁止用于在途消费者持有的修订。
///
/// # Panics
/// `index` 不在车道长度列内时 panic。
pub fn flip_lane_length(root: &mut SharedNetworkRevision, index: usize) {
    root.traffic.lane_lengths_millimetres[index] ^= 1;
}

/// 在当前线程的一次认证调用中依次替换修订比较视图，退出（含 unwind）后清理。
/// 来源准备、比较器、来源复核与复算仍执行真实路径；不预制认证结果。
///
/// # Panics
/// 嵌套武装、空视图、认证使用不同根、比较次数不足或超量时 panic。
pub fn with_root_comparisons<T>(
    root: &Arc<SharedNetworkRevision>,
    views: Vec<Arc<SharedNetworkRevision>>,
    operation: impl FnOnce() -> T,
) -> T {
    assert!(!views.is_empty(), "comparison views must not be empty");
    COMPARISONS.with(|slot| {
        let mut injection = slot.borrow_mut();
        assert!(
            injection.target.is_none() && injection.views.is_empty(),
            "comparison injection is already armed"
        );
        *injection = Injection {
            target: Some(root.clone()),
            views: views.into_iter().map(|view| (None, Some(view))).collect(),
            invalid: false,
        };
    });
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            reset();
        }
    }
    let _reset = Reset;
    let result = operation();
    COMPARISONS.with(|slot| {
        let injection = slot.borrow();
        // 比较阶段 panic 会被认证核心归类，夹具误用必须在该边界外再次拒绝。
        assert!(!injection.invalid, "comparison injection was misused");
        assert!(injection.views.is_empty(), "comparison views not consumed");
    });
    result
}

pub(crate) fn take_comparison(root: &SharedNetworkRevision) -> Comparison {
    COMPARISONS.with(|slot| {
        let mut injection = slot.borrow_mut();
        if let Some(target) = &injection.target
            && (!std::ptr::eq(target.as_ref(), root) || injection.views.is_empty())
        {
            injection.invalid = true;
            panic!("comparison injection target or count mismatch");
        }
        injection.views.pop_front().unwrap_or_default()
    })
}

#[cfg(test)]
pub(crate) fn comparisons(views: Vec<Comparison>) {
    COMPARISONS.with(|slot| {
        assert!(slot.borrow().target.is_none(), "scoped injection is active");
        *slot.borrow_mut() = Injection {
            views: views.into(),
            ..Injection::default()
        };
    });
}

pub(crate) fn reset() {
    COMPARISONS.with(|slot| *slot.borrow_mut() = Injection::default());
}
