//! One running terminal.
//!
//! A session is a child process on a pseudoconsole, the parser that turns its output into
//! events, and the terminal state machine that turns those events into a grid. It owns
//! all three, plus one more thing: where the user is looking. The scroll offset is
//! per-tab state — it is as much a part of what a tab *is* as the grid is — so it lives
//! here rather than in whatever draws the grid.
//!
//! # The pump
//!
//! The pty's own reader thread ends when the pseudoconsole is closed, not when the child
//! exits, and its channel is the only place the child's output exists. Something has to
//! be parked on that channel, and it cannot be the host's event loop. That is the pump
//! thread: it waits up to [`PUMP_TIMEOUT`] for a chunk, appends it to a buffer, and calls
//! the [`Waker`]. It parses nothing and knows nothing about the terminal; [`drain`] does
//! the parsing, on the host's thread, when the host is ready.
//!
//! Two of its behaviours are not obvious and both matter:
//!
//! - The child's **exit is polled**, not waited for. `ConPTY` keeps its output pipe open
//!   after the child dies, so a read never returns zero and the channel never
//!   disconnects; an exit is invisible to any read. The pump asks the process handle
//!   every time it wakes from a timeout, and wakes the host once when the answer changes.
//!   Without that, a `cmd /c` tab would sit there looking alive until something else
//!   happened to wake the host.
//! - It **never holds the buffer lock while calling the waker**. A host whose waker
//!   drains the session synchronously would otherwise deadlock against itself.
//!
//! [`drain`]: Session::drain

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use zet_pty::discovery::Profile;
use zet_pty::{Pty, PtyError, SpawnConfig};
use zet_vt::{Parser, Row, Term};

/// How long the pump waits for output before it looks at its stop flag again.
///
/// This is not the latency of the child's output — a chunk arriving on the channel wakes
/// the pump immediately, whatever the timeout is, so typing stays as fast as the pty
/// leaves it. It is how long the session takes to notice the two things that are not
/// events: that the child has exited, and that it has been asked to stop. Shorter means a
/// snappier `close` and more idle wakeups; a tenth of a second is ten wakeups a second
/// with nothing to do, and a `close` that never feels slow.
pub const PUMP_TIMEOUT: Duration = Duration::from_millis(100);

/// How a host is told there is something to drain.
///
/// The pump thread has no idea what an event loop is, and it must not: a host supplies an
/// implementation that posts to whatever it uses to wake itself, and everything this
/// crate knows is that it has something to report. The wake may arrive after the session
/// has been closed, so an implementation has to tolerate being called for a tab that is
/// gone.
pub trait Waker: Send + Sync + 'static {
    /// Tell the host there is output, or an exit, to look at.
    fn wake(&self);
}

/// A waker that does nothing, for driving a session without a host.
///
/// Sessions are tested, and a test has no event loop to post to. Without this, every test
/// would carry a mock event loop it never asserts on, and the trait would look like it
/// required one.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopWaker;

impl Waker for NoopWaker {
    fn wake(&self) {}
}

/// Something that went wrong starting or driving a session.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// The pseudoconsole layer failed.
    #[error(transparent)]
    Pty(#[from] PtyError),

    /// The pump thread could not be started.
    ///
    /// The child has already been started by this point and is torn down on the way out,
    /// so what is reported is the reason there is no session, not a reason to go looking
    /// for a stray process.
    #[error("could not start the session's pump thread: {0}")]
    Pump(String),

    /// There is no session with this number.
    ///
    /// The numbers a host holds are stable but not eternal, and a stale one has to be an
    /// error rather than a lookup that quietly lands on a different terminal.
    #[error("no session numbered {0}")]
    NoSession(u32),

    /// The session is over.
    #[error("the session has ended")]
    Ended,
}

/// What one call to [`Session::drain`] took in and what it changed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Drained {
    /// How many bytes were handed to the parser.
    pub bytes: usize,

    /// Whether those bytes changed the grid.
    ///
    /// This is about *this* batch and nothing older: the damage tracker is cleared before
    /// the bytes are parsed, so a drain with nothing to feed reports no damage, and the
    /// rows the tracker names afterwards are exactly the rows this call touched. A caller
    /// that wants that row-level detail has to read it from [`Session::term`] before
    /// draining again.
    pub damage: bool,

    /// Whether this is the drain that first saw the child exit.
    ///
    /// An edge rather than a state — [`Session::is_exited`] is the state — because its
    /// job is to say that something *happened*, and a flag that stays true would keep
    /// saying it.
    pub exited: bool,
}

/// One running terminal.
///
/// The child keeps running, and its output keeps accumulating, until the session is
/// closed or dropped. A terminal shows what a program printed on its way out, so nothing
/// here tears anything down in response to the child exiting; a host reads
/// [`Session::is_exited`] and decides when the tab has been read.
pub struct Session {
    /// The pty, taken during teardown so that `Pty::shutdown` can consume it.
    pty: Option<Arc<Pty>>,

    /// Set to tell the pump thread to stop.
    stop: Arc<AtomicBool>,

    /// The pump thread, joined during teardown.
    pump: Option<JoinHandle<()>>,

    /// What the pump has read and `drain` has not taken yet.
    pending: Arc<Mutex<Vec<u8>>>,

    /// Bytes in flight across a chunk boundary. A sequence split between two chunks is
    /// still one sequence, and a parser constructed per drain would lose the state that
    /// makes that work.
    parser: Parser,

    term: Term,

    /// Shown when the program has set no title of its own.
    fallback_title: String,

    /// How many rows above the live screen the view is scrolled.
    ///
    /// An atomic rather than a plain field because [`Session::write`] is the one method
    /// that has to move it and the interface gives that method `&self`: typing is the
    /// user asking to see the prompt, so it returns the view to the bottom, and a shared
    /// reference is all it has to do it with. Relaxed ordering is enough — nothing reads
    /// the offset to make a decision about anything else, so the only requirement is that
    /// the value is not torn, which it cannot be.
    scroll_offset: AtomicUsize,

    /// Whether a drain has already reported the child's exit.
    reported_exit: bool,

    cols: u16,
    rows: u16,
}

impl Session {
    /// Start `profile` on a pseudoconsole, with `waker` told whenever there is output.
    ///
    /// `cwd` is where the child starts; `None` is this process's directory.
    ///
    /// # Errors
    ///
    /// Fails if the program is missing, if the pseudoconsole cannot be created, if the
    /// process cannot be started, or if the pump thread cannot be spawned. A failure
    /// after the child is running tears it down rather than leaving an orphan.
    pub fn spawn(
        profile: &Profile,
        cols: u16,
        rows: u16,
        cwd: Option<PathBuf>,
        waker: Arc<dyn Waker>,
    ) -> Result<Self, SessionError> {
        let mut config = SpawnConfig::new(profile.program.clone(), cols, rows);
        config.args.clone_from(&profile.args);
        config.cwd = cwd;

        let pty = Arc::new(Pty::spawn(&config)?);

        let pending = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let pump = std::thread::Builder::new()
            .name("zet-session-pump".into())
            .spawn({
                let pty = Arc::clone(&pty);
                let pending = Arc::clone(&pending);
                let stop = Arc::clone(&stop);
                move || pump(&pty, &pending, &stop, waker.as_ref())
            })
            .map_err(|error| SessionError::Pump(error.to_string()))?;

        Ok(Session {
            pty: Some(pty),
            stop,
            pump: Some(pump),
            pending,
            parser: Parser::new(),
            term: Term::new(usize::from(cols), usize::from(rows)),
            fallback_title: profile.name.clone(),
            scroll_offset: AtomicUsize::new(0),
            reported_exit: false,
            cols,
            rows,
        })
    }

    /// What to show on the tab.
    ///
    /// The grid holds whatever `OSC 0`/`OSC 2` set, and that is empty until the program
    /// sets one — a shell nobody has customised never does. Falling back to the profile's
    /// name is what keeps a fresh tab from being a blank one; deriving a title from the
    /// process id would be inventing one, and it would be wrong for every shell that
    /// names itself.
    #[must_use]
    pub fn title(&self) -> String {
        let title = self.term.title();
        if title.is_empty() {
            self.fallback_title.clone()
        } else {
            title.to_owned()
        }
    }

    /// The terminal: the grid, the cursor, the modes, and the damage tracker.
    ///
    /// Read-only on purpose. Everything that mutates it is either a sequence the child
    /// sent, which arrives through [`Session::drain`], or an input method the host calls
    /// by name, and both need the session's own bookkeeping to move with them.
    #[must_use]
    pub fn term(&self) -> &Term {
        &self.term
    }

    /// Take everything the child has produced since the last call, feed it to the parser,
    /// and report what changed.
    ///
    /// Never blocks: the pump has already done the waiting.
    #[must_use]
    pub fn drain(&mut self) -> Drained {
        // Split rather than swap, so the pump keeps the allocation it has grown and the
        // lock is held for the copy and nothing else. Holding it across the parse would
        // stall the pump behind every escape sequence the parser walks.
        let bytes = {
            let mut buffer = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            buffer.split_off(0)
        };

        // Cleared before the bytes are parsed, not after. What the tracker holds when
        // this returns is then exactly what these bytes did, and `damage` answers "did
        // anything change just now" rather than "has anything changed since the process
        // started".
        self.term.grid_mut().damage_mut().clear();
        if !bytes.is_empty() {
            self.parser.advance_slice(&bytes, &mut self.term);
        }

        // A terminal that never answers a `CSI 6n` makes programs hang on startup, so
        // these are not optional. The program that asked may have exited in the meantime;
        // a reply nobody is left to read is not worth failing a drain over.
        let responses = self.term.take_responses();
        if !responses.is_empty() {
            let _ = self.pty().write(&responses);
        }

        let damage = !self.term.grid().damage().is_empty();
        let ended = self.is_exited();
        let exited = ended && !self.reported_exit;
        self.reported_exit |= ended;

        // A program can throw its own scrollback away with `ED 3`, or a resize can
        // re-wrap it shorter, and a view left above the new history would show rows that
        // no longer exist.
        self.clamp_scroll();

        Drained {
            bytes: bytes.len(),
            damage,
            exited,
        }
    }

    /// Send bytes to the child: a keystroke, a paste, an encoded mouse report.
    ///
    /// This is also the one thing that returns the view to the bottom. Output must not —
    /// a program that logs while the user is reading history would otherwise yank the
    /// view to the bottom on every line, which is the behaviour that makes a terminal
    /// useless for reading anything longer than a screen. Typing is the user saying they
    /// want the prompt, so the view follows.
    ///
    /// # Errors
    ///
    /// Fails with [`SessionError::Ended`] once the child has gone — the console's input
    /// pipe outlives the child, so a write into it would succeed and report nothing — and
    /// with the pty's own error once its writer has gone.
    ///
    /// What this does not do is wait for the bytes to reach the child. They are queued and
    /// carried by a thread of the pty's own, because a program that is not reading its
    /// input would otherwise block this call — and this call is made from the thread that
    /// owns the window. See `Pty::write`.
    pub fn write(&self, bytes: &[u8]) -> Result<(), SessionError> {
        if self.is_exited() {
            return Err(SessionError::Ended);
        }
        self.scroll_offset.store(0, Ordering::Relaxed);
        self.pty().write(bytes)?;
        Ok(())
    }

    /// Tell the child its window changed size.
    ///
    /// The pseudoconsole is resized first, so a size it refuses leaves the grid agreeing
    /// with the child rather than disagreeing with it. The grid re-wraps on a width
    /// change, which is why the view is re-clamped afterwards.
    ///
    /// # Errors
    ///
    /// Fails if the pseudoconsole refuses the size — a zero in either dimension, or a
    /// session that has already been torn down.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), SessionError> {
        self.pty().resize(cols, rows)?;
        self.term.resize(usize::from(cols), usize::from(rows));
        self.cols = cols;
        self.rows = rows;
        self.clamp_scroll();
        Ok(())
    }

    /// How wide the terminal is, in columns.
    #[must_use]
    pub fn cols(&self) -> u16 {
        self.cols
    }

    /// How tall the terminal is, in rows.
    #[must_use]
    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// How many rows up from the live screen the view is scrolled.
    #[must_use]
    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
            .load(Ordering::Relaxed)
            .min(self.max_scroll())
    }

    /// Scroll the view: positive goes up into the history, negative comes back down
    /// towards the live screen.
    ///
    /// Clamped to the history, so a host can hand this a wheel delta without knowing how
    /// much scrollback there is.
    pub fn scroll(&mut self, delta: i32) {
        let current = self.scroll_offset();
        let magnitude = usize::try_from(delta.unsigned_abs()).unwrap_or(usize::MAX);
        let target = if delta >= 0 {
            current.saturating_add(magnitude)
        } else {
            current.saturating_sub(magnitude)
        };
        self.scroll_to(target);
    }

    /// Scroll to `offset` rows above the live screen, clamped to the history.
    pub fn scroll_to(&mut self, offset: usize) {
        self.scroll_offset
            .store(offset.min(self.max_scroll()), Ordering::Relaxed);
    }

    /// Return the view to the live screen.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_to(0);
    }

    /// The rows on screen, top to bottom.
    ///
    /// Exactly [`Session::rows`] of them, always. Scrolled back, the top of the window
    /// comes out of the history and the live screen supplies the rest; scrolled to the
    /// bottom, this is the live screen.
    ///
    /// Nothing in production calls this. The renderer does the same arithmetic inline,
    /// because it needs each row's *position* as well as the row — a rectangle is a row
    /// and a column — and it does not depend on this crate at all. What reads this is the
    /// session's own tests, which is a real use and the reason it is kept.
    ///
    /// The doc used to end "a renderer is handed rows to draw and never has to know where
    /// the history ends", which described a seam that was never built. Saying so is
    /// cheaper than leaving a comment that reads as a contract.
    #[must_use]
    pub fn visible_rows(&self) -> Vec<&Row> {
        let grid = self.term.grid();
        let top = grid.history_top(self.scroll_offset());
        // In range by construction: `top` and `top + rows` are inside the history the
        // grid holds, which is the scrollback and the screen together.
        (top..top + grid.rows())
            .filter_map(|index| grid.row_from_history(index))
            .collect()
    }

    /// Whether the child has exited.
    ///
    /// A poll of the process handle, so it is true from the moment the child is gone even
    /// if nobody has drained the output it printed on the way out. The session stays
    /// usable afterwards: the pty is still there and what the child wrote is still
    /// arriving.
    #[must_use]
    pub fn is_exited(&self) -> bool {
        self.exit_code().is_some()
    }

    /// The child's exit code, or `None` while it is still running.
    ///
    /// A poll with no wait. The handle stays signalled once the child has exited, so this
    /// is a state and not an event, and asking twice costs one system call.
    #[must_use]
    pub fn exit_code(&self) -> Option<u32> {
        self.pty().wait(Duration::ZERO)
    }

    /// End the session, killing the child and everything it started.
    ///
    /// The same teardown as dropping it, with the difference that this reports what went
    /// wrong on the way out. The session is released either way.
    ///
    /// # Errors
    ///
    /// Fails if the child's process tree could not be killed.
    pub fn close(mut self) -> Result<(), SessionError> {
        self.teardown()
    }

    /// Stop the pump and release the pseudoconsole.
    ///
    /// The order is the whole reason this is a method rather than the body of `Drop`.
    /// The pump is the other owner of the pty, and `Pty::shutdown` consumes the value, so
    /// the pump has to be stopped and joined before the pty can be handed over. Joining
    /// is bounded by [`PUMP_TIMEOUT`], which is why that timeout is as short as it is.
    fn teardown(&mut self) -> Result<(), SessionError> {
        let Some(pty) = self.pty.take() else {
            // Already torn down. Nothing here is idempotent by accident: taking the pty
            // is what marks the session gone.
            return Ok(());
        };

        self.stop.store(true, Ordering::Relaxed);
        if let Some(pump) = self.pump.take() {
            let _ = pump.join();
        }

        match Arc::try_unwrap(pty) {
            Ok(pty) => pty.shutdown().map_err(SessionError::from),
            Err(shared) => {
                // Something else is still holding the pty. Dropping the last handle tears
                // it down exactly the same way — `Pty`'s own `Drop` kills the tree and
                // drains the console while the reader is still running — so the only
                // thing given up is the error report.
                drop(shared);
                Ok(())
            }
        }
    }

    /// The pty.
    ///
    /// The field is only ever empty inside teardown, and no accessor is reachable from
    /// there. Carrying an `Option` through every caller instead would put a branch that
    /// cannot be taken in front of every keystroke and every frame.
    fn pty(&self) -> &Pty {
        self.pty
            .as_deref()
            .expect("the session's pty is only taken while the session is being torn down")
    }

    /// How far back the view can go: the whole history.
    fn max_scroll(&self) -> usize {
        self.term.grid().scrollback_len()
    }

    /// Bring the scroll offset back inside the history it is measured against.
    fn clamp_scroll(&self) {
        let max = self.max_scroll();
        if self.scroll_offset.load(Ordering::Relaxed) > max {
            self.scroll_offset.store(max, Ordering::Relaxed);
        }
    }
}

impl Drop for Session {
    /// Release the session, killing the child.
    ///
    /// A failure has nowhere to go at this point — [`Session::close`] is the reporting
    /// path — and the process tree is killed and the handles released either way. What
    /// this must not do is leave the pump parked on a pty nobody will ever shut down,
    /// which would keep the child running after its tab was gone.
    fn drop(&mut self) {
        let _ = self.teardown();
    }
}

/// Move the child's output into `pending` until `stop` is set, waking `waker` as it goes.
///
/// The waker is called with the buffer unlocked: a host whose waker drains the session
/// synchronously would otherwise deadlock on the lock this thread is holding.
fn pump(pty: &Pty, pending: &Mutex<Vec<u8>>, stop: &AtomicBool, waker: &dyn Waker) {
    // Whether the child's exit has been announced. The exit is not an event on the wire:
    // ConPTY keeps its output pipe open after the child dies, so a read never returns
    // zero, the channel never disconnects, and the only way to learn it happened is to
    // ask the process handle. This is why the pump wakes on a timeout at all.
    let mut announced = false;

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        if !announced && pty.wait(Duration::ZERO).is_some() {
            announced = true;
            waker.wake();
        }

        match pty.next_chunk(PUMP_TIMEOUT) {
            Some(Ok(chunk)) => {
                {
                    let mut buffer = pending.lock().unwrap_or_else(PoisonError::into_inner);
                    buffer.extend_from_slice(&chunk);
                }
                waker.wake();
            }
            // The channel ends when the pty is released, which cannot happen while this
            // thread holds a handle to it. Kept as a stop condition anyway, so that a
            // change in the pty layer cannot leave this spinning through an empty loop.
            Some(Err(_)) => break,
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pump_timeout_is_short_enough_that_a_close_still_feels_immediate() {
        // `close` cannot return until the pump notices the stop flag, so this timeout is
        // a floor on how long closing a tab blocks the host.
        assert!(
            PUMP_TIMEOUT <= Duration::from_millis(250),
            "closing a session would block for {PUMP_TIMEOUT:?}"
        );
    }

    #[test]
    fn the_noop_waker_does_nothing_and_is_usable_wherever_a_waker_is() {
        let waker: Arc<dyn Waker> = Arc::new(NoopWaker);
        waker.wake();
        waker.wake();
    }
}
