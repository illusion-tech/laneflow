//! #787 在真实 Motion 工作块入口构造两线程会合，避免短任务的调度运气。
use std::cell::RefCell;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::ThreadId;
use std::time::Duration;

thread_local! {
    static ACTIVE: RefCell<Option<Arc<Probe>>> = const { RefCell::new(None) };
}
crate::kernel::execution::carry_hooks!(carry_test_hooks: ACTIVE);

#[derive(Clone, Default)]
pub(crate) struct Observation {
    pub(crate) entries: Vec<(usize, ThreadId)>,
    pub(crate) timed_out: bool,
}

pub(crate) struct Probe {
    state: Mutex<Observation>,
    paired: Condvar,
    timeout: Option<Duration>,
}

impl Probe {
    pub(crate) fn enter(&self, start: usize) {
        let mut state = self.state.lock().unwrap();
        state.entries.push((start, std::thread::current().id()));
        let Some(timeout) = self.timeout else {
            return;
        };
        if state.timed_out {
            return;
        }
        if state.entries.len() >= 2 {
            self.paired.notify_all();
            return;
        }
        // 第一块在入口等待第二块；该线程不能在等待期间认领下一块。
        // 只有一次会合，超时后锁存失败并放行余下块，串行退化不会永久挂起。
        let (mut state, result) = self
            .paired
            .wait_timeout_while(state, timeout, |state| state.entries.len() < 2)
            .unwrap();
        if result.timed_out() && state.entries.len() < 2 {
            state.timed_out = true;
        }
    }
}

pub(crate) struct Guard {
    probe: Arc<Probe>,
    previous: Option<Arc<Probe>>,
    coordinator_only: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Guard {
    pub(crate) fn start(timeout: Option<Duration>) -> Self {
        let probe = Arc::new(Probe {
            state: Mutex::new(Observation::default()),
            paired: Condvar::new(),
            timeout,
        });
        let previous = ACTIVE.with(|cell| cell.replace(Some(Arc::clone(&probe))));
        Self {
            probe,
            previous,
            coordinator_only: std::marker::PhantomData,
        }
    }

    pub(crate) fn observation(&self) -> Observation {
        self.probe.state.lock().unwrap().clone()
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        ACTIVE.with(|cell| cell.replace(self.previous.take()));
    }
}

// 由协调器捕获，再传给真实任务；辅助线程不能读取协调器的 TLS。
pub(crate) fn current() -> Option<Arc<Probe>> {
    ACTIVE.with(|cell| cell.borrow().clone())
}

#[test]
fn serial_chunks_time_out_and_later_arrivals_do_not_erase_failure() {
    let guard = Guard::start(Some(Duration::from_millis(10)));
    let probe = current().unwrap();
    probe.enter(0);
    probe.enter(8);
    let observation = guard.observation();
    assert!(observation.timed_out);
    assert_eq!(observation.entries.len(), 2);
    assert_eq!(observation.entries[0].1, observation.entries[1].1);
}

#[test]
fn nested_guard_restores_probe_after_unwind() {
    assert!(current().is_none());
    let outer = Guard::start(Some(Duration::from_secs(5)));
    let original = current().unwrap();
    let result = std::panic::catch_unwind(|| {
        let _inner = Guard::start(Some(Duration::from_secs(5)));
        assert!(!Arc::ptr_eq(&current().unwrap(), &original));
        panic!("probe unwind");
    });
    assert!(result.is_err());
    assert!(Arc::ptr_eq(&current().unwrap(), &original));
    drop(outer);
    assert!(current().is_none());
}
