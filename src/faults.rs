//! Fault injection for the lock-retry and partial-write acceptance cases.
//! Only compiled with the non-default "fault-injection" feature (the test
//! suite enables it); otherwise every hook is a no-op.
#[cfg(feature = "fault-injection")]
use std::cell::RefCell;
#[cfg(feature = "fault-injection")]
use std::collections::HashMap;
use std::io;

#[derive(Clone, Copy, Debug)]
pub enum Fault {
    /// Fail the step with this raw OS error number.
    Os(i32),
    /// Let a write step write at most this many bytes.
    Short(usize),
}

#[cfg(feature = "fault-injection")]
thread_local! {
    static ARMED: RefCell<Vec<(&'static str, Fault)>> = const { RefCell::new(Vec::new()) };
    static CALLS: RefCell<HashMap<&'static str, usize>> = RefCell::new(HashMap::new());
}

/// Arms "times" consecutive faults for step "op" on this thread.
#[cfg(feature = "fault-injection")]
pub fn arm(op: &'static str, fault: Fault, times: usize) {
    ARMED.with(|a| a.borrow_mut().extend(std::iter::repeat_n((op, fault), times)));
}

/// How many times step "op" ran on this thread.
#[cfg(feature = "fault-injection")]
pub fn calls(op: &'static str) -> usize {
    CALLS.with(|c| c.borrow().get(op).copied().unwrap_or(0))
}

#[cfg(feature = "fault-injection")]
pub fn reset() {
    ARMED.with(|a| a.borrow_mut().clear());
    CALLS.with(|c| c.borrow_mut().clear());
}

#[cfg(not(feature = "fault-injection"))]
pub(crate) fn hit(_op: &'static str) -> Option<Fault> {
    None
}

#[cfg(feature = "fault-injection")]
pub(crate) fn hit(op: &'static str) -> Option<Fault> {
    CALLS.with(|c| *c.borrow_mut().entry(op).or_default() += 1);
    ARMED.with(|a| {
        let mut a = a.borrow_mut();
        let i = a.iter().position(|(o, _)| *o == op)?;
        Some(a.remove(i).1)
    })
}

pub(crate) fn os(op: &'static str) -> io::Result<()> {
    match hit(op) {
        Some(Fault::Os(code)) => Err(io::Error::from_raw_os_error(code)),
        _ => Ok(()),
    }
}
