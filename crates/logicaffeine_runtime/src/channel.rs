//! Channels — FIFO, bounded, move-semantics pipes (the language's `Pipe`).
//!
//! A channel carries [`RtPayload`] values in FIFO order. Capacity `Some(n)` bounds
//! the buffer (a full channel blocks senders); `Some(0)` is a rendezvous channel
//! (every send hands off directly to a receiver); `None` is unbounded.

use std::collections::VecDeque;

use crate::payload::RtPayload;
use crate::task::TaskId;

/// A scheduler-assigned channel handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChanId(pub u64);

/// A channel's buffer and its blocked senders/receivers.
pub(crate) struct Chan {
    /// `Some(n)` bounded (n may be 0 = rendezvous); `None` unbounded.
    pub capacity: Option<usize>,
    /// Buffered values awaiting a receiver, FIFO.
    pub queue: VecDeque<RtPayload>,
    /// Senders parked because the buffer was full, with the value they want to send.
    pub blocked_senders: VecDeque<(TaskId, RtPayload)>,
    /// Receivers parked because the buffer was empty (includes select-waiters).
    pub blocked_receivers: VecDeque<TaskId>,
    /// Once closed, a receive on an empty channel yields `Nothing` instead of
    /// blocking, and a closed channel counts as receive-ready for `Select`.
    pub closed: bool,
}

impl Chan {
    pub(crate) fn new(capacity: Option<usize>) -> Self {
        Chan {
            capacity,
            queue: VecDeque::new(),
            blocked_senders: VecDeque::new(),
            blocked_receivers: VecDeque::new(),
            closed: false,
        }
    }

    /// Is there buffer room for one more value right now?
    pub(crate) fn has_room(&self) -> bool {
        match self.capacity {
            None => true,
            Some(cap) => self.queue.len() < cap,
        }
    }

    /// Can a receive succeed immediately (a buffered value, a waiting sender, or
    /// a closed channel — which delivers `Nothing`)?
    pub(crate) fn can_recv(&self) -> bool {
        !self.queue.is_empty() || !self.blocked_senders.is_empty() || self.closed
    }
}
