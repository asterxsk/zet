//! The Windows pseudoconsole session.
//!
//! The lifetime rules that keep this from hanging are described on [`Pty`]. Read that
//! before changing anything about the order handles are closed in.

use std::ffi::{OsStr, OsString, c_void};
use std::io::{Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, HANDLE, S_OK, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::ReadFile;
use windows_sys::Win32::System::Console::{
    COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CREATE_NEW_PROCESS_GROUP, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
    GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList, PROCESS_INFORMATION,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};

use crate::PtyError;

/// How many chunks the reader may run ahead of the consumer.
///
/// The send blocks when this is full, which is the point: backpressure has to reach the
/// child process, or a program that prints faster than the terminal can parse will grow
/// an unbounded queue and the terminal will appear to freeze while it catches up.
pub const DEFAULT_BUFFER: usize = 64;

/// How to start a child process.
#[derive(Clone, Debug)]
pub struct SpawnConfig {
    /// The executable to run.
    pub program: PathBuf,
    /// Its arguments, already split.
    pub args: Vec<OsString>,
    /// Where to start it, or the current directory.
    pub cwd: Option<PathBuf>,
    /// The starting width, in columns.
    pub cols: u16,
    /// The starting height, in rows.
    pub rows: u16,
}

impl SpawnConfig {
    /// A configuration for `program` at a size, with no arguments.
    pub fn new(program: impl Into<PathBuf>, cols: u16, rows: u16) -> Self {
        SpawnConfig {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            cols,
            rows,
        }
    }

    /// Add one argument.
    #[must_use]
    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Add several arguments.
    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }
}

/// A child process attached to a pseudoconsole.
///
/// # Lifetime
///
/// The reader thread this starts is not optional. [`ClosePseudoConsole`] blocks until the
/// pseudoconsole's output has been drained, so it can only ever be called while something
/// is still reading. [`Pty::shutdown`] relies on that: it kills the process tree, then
/// closes the pseudoconsole while the reader keeps draining, then drains the channel
/// until the reader thread ends. Calling [`Pty::kill`] without shutting down leaves the
/// child dead and the session usable for reading whatever it printed on the way out.
pub struct Pty {
    console: Option<Console>,
    input: Option<std::fs::File>,
    process: Option<OwnedHandle>,
    job: OwnedHandle,
    pid: u32,
    events: Receiver<Vec<u8>>,
    reader: Option<std::thread::JoinHandle<()>>,
}

// SAFETY: every field is a handle, a channel, or a join handle. The handles are only
// ever used through `&self` methods, which Windows serialises internally, and the one
// method that consumes the value takes it by value.
unsafe impl Send for Pty {}

impl Pty {
    /// Start `config` attached to a new pseudoconsole.
    ///
    /// # Errors
    ///
    /// Fails if the program does not exist, if the pseudoconsole cannot be created, or if
    /// the process cannot be started.
    pub fn spawn(config: &SpawnConfig) -> Result<Self, PtyError> {
        if config.cols == 0 || config.rows == 0 {
            return Err(PtyError::EmptySize {
                cols: config.cols,
                rows: config.rows,
            });
        }
        if !config.program.is_file() {
            return Err(PtyError::NotFound(config.program.clone()));
        }

        let (console, input, output) = create_console(config.cols, config.rows)?;

        // The job comes before the process, always. A process created outside a job and
        // assigned later can spawn a grandchild in the gap, and that grandchild outlives
        // the terminal.
        let job = create_job()?;

        let process = match start_process(config, console.handle, &job) {
            Ok(process) => process,
            Err(error) => {
                // Nothing will drain the console now, so close it with a reader of our
                // own rather than blocking on a close nobody is reading for. The job
                // closes itself when it goes out of scope here.
                close_console(console, output);
                drop(job);
                return Err(error);
            }
        };

        let (sender, events) = sync_channel::<Vec<u8>>(DEFAULT_BUFFER);
        let reader = std::thread::Builder::new()
            .name("zet-pty-read".into())
            .spawn(move || read_loop(output, &sender))
            .map_err(|error| PtyError::Io {
                what: "spawning the reader thread",
                message: error.to_string(),
            })?;

        Ok(Pty {
            console: Some(console),
            input: Some(input),
            process: Some(process.0),
            job,
            pid: process.1,
            events,
            reader: Some(reader),
        })
    }

    /// The process id of the child.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Wait for the next chunk of output, up to `timeout`.
    ///
    /// Returns `None` when the timeout passed with nothing to report and `Some(Err(_))`
    /// when the session has ended.
    #[must_use]
    pub fn next_chunk(&self, timeout: Duration) -> Option<Result<Vec<u8>, PtyError>> {
        match self.events.recv_timeout(timeout) {
            Ok(chunk) => Some(Ok(chunk)),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => Some(Err(PtyError::Io {
                what: "reading from the child",
                message: "the session ended".into(),
            })),
        }
    }

    /// Write bytes to the child's input.
    ///
    /// These are keystrokes or a bracketed paste, already encoded. This crate does not
    /// turn key events into bytes; that belongs to whatever owns the keymap.
    ///
    /// # Errors
    ///
    /// Fails if the child has gone away.
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        let input = self.input.as_mut().ok_or_else(|| PtyError::Io {
            what: "writing to the child",
            message: "the session has ended".into(),
        })?;
        input.write_all(bytes).map_err(|error| PtyError::Io {
            what: "writing to the child",
            message: error.to_string(),
        })?;
        input.flush().map_err(|error| PtyError::Io {
            what: "flushing to the child",
            message: error.to_string(),
        })
    }

    /// Tell the child its window changed size.
    ///
    /// # Errors
    ///
    /// Fails if the pseudoconsole has been closed or the size is zero.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError> {
        if cols == 0 || rows == 0 {
            return Err(PtyError::EmptySize { cols, rows });
        }
        let console = self
            .console
            .as_ref()
            .map(|console| console.handle)
            .ok_or_else(|| PtyError::Io {
                what: "resizing the pseudoconsole",
                message: "the session has ended".into(),
            })?;
        let size = COORD {
            X: cols.cast_signed(),
            Y: rows.cast_signed(),
        };
        // SAFETY: `console` came from `CreatePseudoConsole` and is closed exactly once,
        // in `shutdown`. `size` is a plain value.
        let result = unsafe { ResizePseudoConsole(console, size) };
        if result < 0 {
            return Err(PtyError::Io {
                what: "resizing the pseudoconsole",
                message: format!("HRESULT {result:#x}"),
            });
        }
        Ok(())
    }

    /// Kill the child and everything it started.
    ///
    /// The process tree goes away through the job object, which is the only mechanism
    /// that reaches grandchildren a program detached on purpose.
    ///
    /// # Errors
    ///
    /// Fails if the job could not be terminated.
    pub fn kill(&self) -> Result<(), PtyError> {
        // SAFETY: `job` is a live job handle owned by this struct.
        let ok = unsafe { TerminateJobObject(self.job.as_raw_handle() as HANDLE, 1) };
        if ok == 0 {
            return Err(last_error("terminating the child process tree"));
        }
        Ok(())
    }

    /// Wait for the child to exit, up to `timeout`, and report its exit code.
    ///
    /// Returns `None` if it is still running when the timeout passes.
    #[must_use]
    pub fn wait(&self, timeout: Duration) -> Option<u32> {
        let process = self.process.as_ref()?;
        let millis = u32::try_from(timeout.as_millis()).unwrap_or(INFINITE);
        // SAFETY: `process` is a live process handle owned by this struct.
        let waited = unsafe { WaitForSingleObject(process.as_raw_handle() as HANDLE, millis) };
        if waited != WAIT_OBJECT_0 {
            return None;
        }
        let mut code = 0u32;
        // SAFETY: both handles are live; `code` is a plain out-parameter.
        let ok = unsafe { GetExitCodeProcess(process.as_raw_handle() as HANDLE, &raw mut code) };
        (ok != 0).then_some(code)
    }

    /// End the session and release everything.
    ///
    /// The order here is the whole reason this method exists rather than a `Drop` impl.
    /// The process tree is killed first, then the pseudoconsole is closed *while the
    /// reader thread is still draining it* — a closed pseudoconsole with nobody reading
    /// blocks forever — and only then is the reader joined. The unused output is drained
    /// so the reader is never left blocked on a full channel, which would otherwise be a
    /// deadlock on the exact code path that is supposed to clean up.
    ///
    /// # Errors
    ///
    /// Fails if the child could not be killed. The session is released either way.
    pub fn shutdown(mut self) -> Result<(), PtyError> {
        let killed = self.kill();
        self.release();
        killed
    }

    /// Release every handle without waiting for anything.
    fn release(&mut self) {
        if let Some(mut console) = self.console.take() {
            // The reader is still running and will keep draining, which is what lets this
            // return instead of blocking.
            // SAFETY: the console came from `CreatePseudoConsole` and `take` means this
            // runs at most once.
            unsafe { ClosePseudoConsole(console.handle) };
            // Only now are the pipes ours to close. Closing them first would leave the
            // close above flushing into a pipe with no reader on the far end; leaving
            // them open would keep the reader from ever seeing the pipe break.
            console.close_pipes();
        }

        while self.events.recv_timeout(Duration::from_millis(20)).is_ok() {}

        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }

        // The write end goes last. Closing it earlier reads as "the window went away" to
        // the console, which kills the child rather than letting it exit on its own.
        drop(self.input.take());
        drop(self.process.take());
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        if self.console.is_some() {
            let _ = self.kill();
            self.release();
        }
    }
}

/// Read until the pseudoconsole closes, forwarding each chunk to `sender`.
fn read_loop(mut output: std::fs::File, sender: &SyncSender<Vec<u8>>) {
    let handle = output.as_raw_handle() as HANDLE;
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let mut read = 0u32;
        // SAFETY: `handle` is the live read end of the console pipe, `buffer` is writable
        // for its whole length, and `read` is a plain out-parameter. The overlapped
        // pointer is null, which is what makes this the blocking form.
        let ok = unsafe {
            ReadFile(
                handle,
                buffer.as_mut_ptr(),
                u32::try_from(buffer.len()).unwrap_or(u32::MAX),
                &raw mut read,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || read == 0 {
            break;
        }
        // A full channel parks this thread, which is the backpressure the child feels.
        if sender.send(buffer[..read as usize].to_vec()).is_err() {
            break;
        }
    }
    // `ReadFile` returns a handle this function must not close: `output` owns it and drops
    // it here, which is correct because nothing else holds the read end.
    let _ = &mut output;
}

/// Create the console and the two pipes attached to it.
///
/// Returns the console, our write end, and our read end.
fn create_console(
    cols: u16,
    rows: u16,
) -> Result<(Console, std::fs::File, std::fs::File), PtyError> {
    // The pipes are not inheritable. The console duplicates what it needs, and an
    // inheritable pipe handle would leak into every process the child starts.
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(0),
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 0,
    };

    let mut to_console = HANDLE::default();
    let mut from_us = HANDLE::default();
    // SAFETY: both out-parameters are valid, and `attributes` is a fully initialised
    // SECURITY_ATTRIBUTES with the length field set to its own size.
    let ok = unsafe { CreatePipe(&raw mut to_console, &raw mut from_us, &raw const attributes, 0) };
    if ok == 0 {
        return Err(last_error("creating the input pipe"));
    }
    let _ = &mut attributes;

    let mut to_us = HANDLE::default();
    let mut from_console = HANDLE::default();
    // SAFETY: as above.
    let ok = unsafe { CreatePipe(&raw mut to_us, &raw mut from_console, &raw const attributes, 0) };
    if ok == 0 {
        // SAFETY: both handles came from a successful `CreatePipe` and are not owned by
        // anything yet.
        unsafe {
            CloseHandle(to_console);
            CloseHandle(from_us);
        }
        return Err(last_error("creating the output pipe"));
    }

    // A console dimension is a signed 16-bit count, and the values reaching here come
    // from the renderer's own grid, which is nowhere near that ceiling.
    let size = COORD {
        X: cols.cast_signed(),
        Y: rows.cast_signed(),
    };
    // `HPCON` is an `isize`, not a pointer, and its failure sentinel is zero.
    let mut console: HPCON = 0;
    // SAFETY: the two handles are the console's ends of the pipes, `size` is a plain
    // value, and `console` is a valid out-parameter. A null HPCON means failure; the
    // function reports failure through its HRESULT.
    let result =
        unsafe { CreatePseudoConsole(size, to_console, from_console, 0, &raw mut console) };

    if result != S_OK || console == 0 {
        // SAFETY: all four handles came from a successful `CreatePipe` and are closed
        // exactly once, here.
        unsafe {
            CloseHandle(to_console);
            CloseHandle(from_console);
            CloseHandle(from_us);
            CloseHandle(to_us);
        }
        return Err(PtyError::ConPty(format!("HRESULT {result:#x}")));
    }

    // SAFETY: both handles are the non-inheritable ends of the pipes created above and
    // are not held anywhere else, so transferring ownership to `File` is sound. `File`
    // closes them with `CloseHandle` on drop, which is what these handles need.
    let input = unsafe { std::fs::File::from_raw_handle(from_us as RawHandle) };
    // SAFETY: as above.
    let output = unsafe { std::fs::File::from_raw_handle(to_us as RawHandle) };
    Ok((
        Console {
            handle: console,
            attached: [to_console, from_console],
        },
        input,
        output,
    ))
}

/// Close a console that nobody is reading, by draining it first.
fn close_console(mut console: Console, mut output: std::fs::File) {
    let drain = std::thread::spawn(move || {
        let mut sink = vec![0u8; 16 * 1024];
        // The close only returns once the client has drained everything, so this has to
        // run concurrently with it rather than before it.
        while let Ok(read) = output.read(&mut sink) {
            if read == 0 {
                break;
            }
        }
    });
    // SAFETY: the console came from `CreatePseudoConsole` and is closed exactly once.
    unsafe { ClosePseudoConsole(console.handle) };
    console.close_pipes();
    let _ = drain.join();
}

/// A pseudoconsole, and the pipe ends it was created from.
///
/// The two pipe ends have to stay open for as long as the console does, and the console
/// uses the very handles it was given rather than copies of them. Closing them early
/// leaves a live pseudoconsole attached to a dead pipe: the session starts, emits its
/// opening sequences, and then goes silent forever while the child runs to completion
/// writing into a console nobody can hear. Nothing reports an error, so it reads as a
/// child that produces no output.
struct Console {
    /// The pseudoconsole itself.
    handle: HPCON,
    /// The pipe ends given to `CreatePseudoConsole`.
    attached: [HANDLE; 2],
}

impl Console {
    /// Release the pipe ends the pseudoconsole was built from.
    ///
    /// This is only safe once the console behind them has been closed. Closing them
    /// earlier cuts the console off from its own output; leaving them open after the
    /// console is gone keeps the reader from ever seeing the pipe break, so the reader
    /// thread would block on a channel that can no longer carry anything.
    fn close_pipes(&mut self) {
        for handle in std::mem::take(&mut self.attached) {
            if !handle.is_null() {
                // SAFETY: each handle came from a successful `CreatePipe`, was given to
                // `CreatePseudoConsole`, and has not been closed since — `take` makes this
                // run at most once per handle.
                unsafe { CloseHandle(handle) };
            }
        }
    }
}

/// A job object that kills everything in it when the last handle closes.
fn create_job() -> Result<OwnedHandle, PtyError> {
    // SAFETY: null attributes and a null name are both the documented "anonymous,
    // default security" form.
    let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if raw.is_null() {
        return Err(last_error("creating the job object"));
    }

    // SAFETY: the limit structure is plain data, so an all-zero bit pattern is a valid
    // value for it.
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: `job` is a live job handle, `limits` is a fully sized struct of the class
    // named in the second argument, and the size matches it exactly.
    let ok = unsafe {
        SetInformationJobObject(
            raw,
            JobObjectExtendedLimitInformation,
            std::ptr::from_ref(&limits).cast::<c_void>(),
            u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()).unwrap_or(0),
        )
    };
    if ok == 0 {
        // SAFETY: `raw` is a live handle created above and is closed exactly once.
        unsafe { CloseHandle(raw) };
        return Err(last_error("configuring the job object"));
    }

    // SAFETY: `raw` is a live, unowned job handle, so wrapping it here transfers
    // ownership exactly once.
    Ok(unsafe { OwnedHandle::from_raw_handle(raw as RawHandle) })
}

/// Start the child, suspended, inside the job, then let it run.
fn start_process(
    config: &SpawnConfig,
    console: HPCON,
    job: &OwnedHandle,
) -> Result<(OwnedHandle, u32), PtyError> {
    let mut command_line = build_command_line(&config.program, &config.args);
    let program = wide(&config.program);
    let cwd = config.cwd.as_ref().map(|path| wide(path));

    // SAFETY: the startup structure is plain data, so an all-zero bit pattern is a valid
    // value for it.
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb =
        u32::try_from(std::mem::size_of::<STARTUPINFOEXW>()).unwrap_or(u32::MAX);
    // The child's standard handles are cleared rather than inherited.
    //
    // A process started with a pseudoconsole gets that console, but its standard handles
    // still come from ours, and inheriting them is wrong in both directions. A console
    // handle writes to *our* console, so the child's output shows up outside the session
    // while the pseudoconsole renders nothing; a handle that is not inheritable is simply
    // invalid in the child, so a program that writes to it produces nothing anywhere.
    // Either way the session looks like a child that starts and then says nothing. Null
    // handles are what a windowed terminal passes without asking, because its own are
    // null; setting them explicitly is what makes that independent of how zet was
    // launched. The handles are already zero from `zeroed` above.
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;

    // The attribute list is variable-length: the first call reports the size, the second
    // fills it in.
    let mut list_size = 0usize;
    // SAFETY: a null list pointer is the documented sizing call, and `list_size` is a
    // valid out-parameter.
    unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &raw mut list_size) };
    if list_size == 0 {
        return Err(last_error("sizing the process attribute list"));
    }
    // The list has to be pointer-aligned. A `Vec<u8>` is aligned to one byte, and a
    // misaligned list is not rejected — the process simply fails to initialise, which
    // arrives as STATUS_DLL_INIT_FAILED from the child rather than as an error here.
    let mut list = vec![0usize; list_size.div_ceil(std::mem::size_of::<usize>())];
    startup.lpAttributeList = list.as_mut_ptr().cast();

    // SAFETY: `list` is at least `list_size` bytes, which is the size the sizing call
    // reported, and the pointer came from a `Vec` that outlives this block.
    let ok = unsafe {
        InitializeProcThreadAttributeList(startup.lpAttributeList, 1, 0, &raw mut list_size)
    };
    if ok == 0 {
        return Err(last_error("allocating the process attribute list"));
    }

    // SAFETY: the attribute list is initialised and holds one slot, the attribute is the
    // documented pseudoconsole one, and the value is a live `HPCON` whose size is given
    // correctly.
    //
    // `lpValue` is the handle itself, not a pointer to it — that is what the documented
    // sample passes, and it is not a stylistic choice. Handing over `&console` stores the
    // address of a stack local in the attribute list, and the child then starts with a
    // pseudoconsole handle that was never valid: it exits with `STATUS_DLL_INIT_FAILED`
    // and `CreateProcessW` still reports success, so the only symptom is a child that
    // dies before it prints anything.
    let ok = unsafe {
        UpdateProcThreadAttribute(
            startup.lpAttributeList,
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
            console as *const c_void,
            std::mem::size_of::<HPCON>(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        // SAFETY: the list was initialised above and is deleted exactly once.
        unsafe { DeleteProcThreadAttributeList(startup.lpAttributeList) };
        return Err(last_error("attaching the pseudoconsole to the process"));
    }

    // SAFETY: the process information structure is plain data, so an all-zero bit
    // pattern is a valid value for it.
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let flags =
        EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_NEW_PROCESS_GROUP | CREATE_SUSPENDED;

    // SAFETY: the program and command line are null-terminated UTF-16 buffers that
    // outlive the call, `startup` is a valid STARTUPINFOEXW of the declared size, and
    // `info` is a valid out-parameter. A null environment inherits the parent's. The
    // handles into the child are the pseudoconsole's, so nothing needs inheriting.
    let ok = unsafe {
        CreateProcessW(
            program.as_ptr(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            flags,
            std::ptr::null(),
            cwd.as_ref().map_or(std::ptr::null(), Vec::as_ptr),
            std::ptr::from_ref(&startup.StartupInfo).cast(),
            &raw mut info,
        )
    };

    // SAFETY: the list was initialised above and is deleted exactly once, whether or not
    // the process started.
    unsafe { DeleteProcThreadAttributeList(startup.lpAttributeList) };

    if ok == 0 {
        return Err(PtyError::Spawn {
            program: config.program.display().to_string(),
            message: std::io::Error::last_os_error().to_string(),
        });
    }

    // Assign before resuming. A process that gets to run first can spawn a grandchild
    // that the job never sees.
    // SAFETY: both handles are live: `info.hProcess` from `CreateProcessW`, and the job
    // owned by the caller.
    let assigned = unsafe { AssignProcessToJobObject(job.as_raw_handle() as HANDLE, info.hProcess) };
    if assigned == 0 {
        // SAFETY: the thread handle came from `CreateProcessW` and is closed once.
        unsafe { CloseHandle(info.hThread) };
        // SAFETY: the process handle came from `CreateProcessW` and is closed once.
        unsafe { TerminateProcess(info.hProcess, 1) };
        // SAFETY: as above.
        unsafe { CloseHandle(info.hProcess) };
        return Err(last_error("adding the process to the job object"));
    }

    // SAFETY: `info.hThread` is the live primary thread handle from `CreateProcessW`.
    // A failure here leaves the process suspended forever, so it is reported rather than
    // ignored.
    if unsafe { ResumeThread(info.hThread) } == u32::MAX {
        // SAFETY: both handles came from `CreateProcessW` and are closed exactly once.
        unsafe {
            TerminateProcess(info.hProcess, 1);
            CloseHandle(info.hThread);
            CloseHandle(info.hProcess);
        }
        return Err(last_error("starting the child's first thread"));
    }
    // SAFETY: the thread handle is not used again and is closed exactly once.
    unsafe { CloseHandle(info.hThread) };

    // SAFETY: `info.hProcess` is a live, unowned process handle, so wrapping it transfers
    // ownership exactly once.
    let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess as RawHandle) };
    Ok((process, info.dwProcessId))
}

/// The command line `CreateProcessW` wants: the program, quoted, then the arguments.
fn build_command_line(program: &Path, args: &[OsString]) -> Vec<u16> {
    let mut line: Vec<u16> = Vec::new();
    push_quoted(&mut line, program.as_os_str());
    for arg in args {
        line.push(u16::from(b' '));
        push_quoted(&mut line, arg);
    }
    line.push(0);
    line
}

/// Append `value`, quoted the way the C runtime will parse it back.
///
/// The rule is the one every Windows program agrees on: wrap in quotes, and backslash-
/// escape any quote and any run of backslashes that immediately precedes one. Getting
/// this wrong is how a path with a space in it becomes two arguments.
fn push_quoted(line: &mut Vec<u16>, value: &OsStr) {
    let utf16: Vec<u16> = value.encode_wide().collect();
    if !utf16.is_empty() && !utf16.iter().any(|c| *c == u16::from(b' ') || *c == u16::from(b'\t') || *c == u16::from(b'"')) {
        line.extend_from_slice(&utf16);
        return;
    }
    line.push(u16::from(b'"'));
    let mut backslashes = 0usize;
    for unit in utf16 {
        if unit == u16::from(b'\\') {
            backslashes += 1;
            continue;
        }
        if unit == u16::from(b'"') {
            // A quote has to be escaped, and so does every backslash before it, or the
            // backslashes are read as escaping the closing quote instead.
            line.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes * 2 + 1));
            line.push(u16::from(b'"'));
        } else {
            line.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes));
            line.push(unit);
        }
        backslashes = 0;
    }
    // A trailing run of backslashes would escape the closing quote if left alone.
    line.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes * 2));
    line.push(u16::from(b'"'));
}

/// `value` as null-terminated UTF-16.
fn wide(value: &Path) -> Vec<u16> {
    value.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// The error Windows last reported.
fn last_error(what: &'static str) -> PtyError {
    PtyError::Io {
        what,
        message: std::io::Error::last_os_error().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quote(value: &str) -> String {
        let mut line = Vec::new();
        push_quoted(&mut line, OsStr::new(value));
        String::from_utf16_lossy(&line)
    }

    #[test]
    fn a_simple_path_is_not_quoted() {
        assert_eq!(quote(r"C:\Windows\System32\cmd.exe"), r"C:\Windows\System32\cmd.exe");
    }

    #[test]
    fn a_path_with_a_space_is_quoted() {
        assert_eq!(
            quote(r"C:\Program Files\PowerShell\7\pwsh.exe"),
            r#""C:\Program Files\PowerShell\7\pwsh.exe""#
        );
    }

    #[test]
    fn a_trailing_backslash_does_not_escape_the_closing_quote() {
        assert_eq!(quote(r"C:\Program Files\"), r#""C:\Program Files\\""#);
    }

    #[test]
    fn an_embedded_quote_is_escaped() {
        assert_eq!(quote("say \"hi\""), r#""say \"hi\"""#);
    }

    #[test]
    fn backslashes_before_a_quote_are_doubled() {
        assert_eq!(quote("a\\\"b"), "\"a\\\\\\\"b\"");
    }

    #[test]
    fn a_command_line_starts_with_the_program_and_separates_the_arguments() {
        let line = build_command_line(
            Path::new(r"C:\Program Files\PowerShell\7\pwsh.exe"),
            &[OsString::from("-NoLogo"), OsString::from("-NoProfile")],
        );
        let text = String::from_utf16_lossy(&line[..line.len() - 1]);
        assert_eq!(
            text,
            r#""C:\Program Files\PowerShell\7\pwsh.exe" -NoLogo -NoProfile"#
        );
    }

    #[test]
    fn a_zero_sized_terminal_is_rejected_before_anything_is_created() {
        let config = SpawnConfig::new(r"C:\Windows\System32\cmd.exe", 0, 24);
        assert!(matches!(
            Pty::spawn(&config),
            Err(PtyError::EmptySize { cols: 0, .. })
        ));
    }

    #[test]
    fn a_program_that_does_not_exist_is_reported_as_missing() {
        let config = SpawnConfig::new(r"C:\no\such\program.exe", 80, 24);
        assert!(matches!(Pty::spawn(&config), Err(PtyError::NotFound(_))));
    }
}
