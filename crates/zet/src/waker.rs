//! Turning a shell's "there is output" into a wakeup the event loop can see.
//!
//! `zet-session` pumps the pseudoconsole on its own thread and, when bytes arrive, calls
//! whatever [`Waker`] it was given. The thread that draws is never the thread that reads,
//! so something has to cross the gap, and `winit`'s own answer is the only one that does
//! not mean polling: an [`EventLoopProxy`] posts a user event that wakes a loop parked in
//! `ControlFlow::Wait`.

use winit::event_loop::EventLoopProxy;
use zet_session::Waker;

/// Something for the event loop to do that did not come from the window.
///
/// One variant, because there is one thing: a session has output to drain, or has exited.
/// The session that woke us is not named, and does not need to be — draining walks every
/// open session, which is a handful of non-blocking reads and therefore cheaper than
/// tracking which one moved.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wake {
    /// Drain the sessions.
    Output,
}

/// A [`Waker`] that posts to the event loop.
pub struct ProxyWaker {
    proxy: EventLoopProxy<Wake>,
}

impl ProxyWaker {
    /// Wrap a proxy.
    #[must_use]
    pub const fn new(proxy: EventLoopProxy<Wake>) -> Self {
        Self { proxy }
    }
}

impl Waker for ProxyWaker {
    fn wake(&self) {
        // A failed send means the loop is gone, which means the app is shutting down and
        // there is nobody left to tell. That is not an error worth reporting: the only
        // work it would interrupt is the teardown that closed the loop in the first place.
        let _ = self.proxy.send_event(Wake::Output);
    }
}
