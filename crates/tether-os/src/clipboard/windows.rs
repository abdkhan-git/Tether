//! Clipboard change notifications and privacy marks on Windows.

use std::io;
use std::ptr::{null, null_mut};
use std::sync::mpsc;
use std::time::Duration;

use windows_sys::Win32::Foundation::GlobalFree;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber,
    IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, RemoveClipboardFormatListener,
    SetClipboardData,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, PostMessageW, PostThreadMessageW,
    RegisterClassW, HWND_MESSAGE, MSG, WM_APP, WM_CLIPBOARDUPDATE, WM_QUIT, WNDCLASSW,
};

use super::{Mode, Watcher};

/// Re-posted from the window procedure, so the message loop sees every change.
const WM_TETHER_CHANGED: u32 = WM_APP + 1;
const CLASS_NAME: &str = "TetherClipboardListener";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// Window procedure for the listener window.
///
/// # Safety
/// Called by Windows with a valid window handle and message arguments.
unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_CLIPBOARDUPDATE {
        // SAFETY: `hwnd` is the live window this procedure was called for.
        unsafe { PostMessageW(hwnd, WM_TETHER_CHANGED, 0, 0) };
        return 0;
    }
    // SAFETY: default handling, with exactly the arguments Windows passed in.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// A message-only window (invisible, receives no broadcasts) registered as a
/// clipboard format listener. Must be called on the thread that will pump its messages.
fn create_listener() -> io::Result<HWND> {
    let class = wide(CLASS_NAME);
    // SAFETY: a null name asks for the handle of the running executable.
    let instance = unsafe { GetModuleHandleW(null()) };
    let wc = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: class.as_ptr(),
        ..Default::default()
    };
    // SAFETY: `wc` is fully initialised and `class` outlives the call (Windows
    // copies the name). Registering again fails harmlessly with
    // ERROR_CLASS_ALREADY_EXISTS when another watcher registered it first.
    unsafe { RegisterClassW(&wc) };
    // SAFETY: the class is registered (above or earlier); all pointer
    // arguments are null or point to live, NUL-terminated strings.
    let hwnd = unsafe {
        CreateWindowExW(0, class.as_ptr(), null(), 0, 0, 0, 0, 0, HWND_MESSAGE, null_mut(), instance, null())
    };
    if hwnd.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `hwnd` is the window just created on this thread.
    if unsafe { AddClipboardFormatListener(hwnd) } == 0 {
        let e = io::Error::last_os_error();
        // SAFETY: as above; destroying our own window on its own thread.
        unsafe { DestroyWindow(hwnd) };
        return Err(e);
    }
    Ok(hwnd)
}

pub fn watch<F: FnMut() -> bool + Send + 'static>(mut on_change: F) -> io::Result<Watcher> {
    let (ready_tx, ready_rx) = mpsc::sync_channel::<io::Result<u32>>(1);
    let thread = std::thread::Builder::new().name("clipboard-watch".into()).spawn(move || {
        let hwnd = match create_listener() {
            Ok(hwnd) => hwnd,
            Err(e) => {
                let _ = ready_tx.send(Err(e));
                return;
            }
        };
        // SAFETY: no preconditions. The thread's message queue exists now
        // (creating the window made it), so PostThreadMessageW can reach it.
        let _ = ready_tx.send(Ok(unsafe { GetCurrentThreadId() }));
        let mut msg = MSG::default();
        // SAFETY: `msg` is a valid out-pointer; a null window means every
        // message for this thread. Returns 0 on WM_QUIT and -1 on error.
        while unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) } > 0 {
            if msg.message == WM_CLIPBOARDUPDATE || msg.message == WM_TETHER_CHANGED {
                if !on_change() {
                    break;
                }
            } else {
                // SAFETY: `msg` was filled in by GetMessageW just above.
                unsafe { DispatchMessageW(&msg) };
            }
        }
        // SAFETY: `hwnd` is our window, on its own thread, removed and destroyed once.
        unsafe {
            RemoveClipboardFormatListener(hwnd);
            DestroyWindow(hwnd);
        }
    })?;
    let thread_id = match ready_rx.recv() {
        Ok(Ok(id)) => id,
        Ok(Err(e)) => {
            let _ = thread.join();
            return Err(e);
        }
        Err(_) => return Err(io::Error::other("the clipboard watcher thread stopped")),
    };
    let stop = move || {
        // SAFETY: posting to a thread id touches no memory of ours; if the
        // thread has already exited the call just fails.
        unsafe { PostThreadMessageW(thread_id, WM_QUIT, 0, 0) };
    };
    Ok(Watcher::new(Mode::Notifications, Box::new(stop), Some(thread)))
}

pub fn sequence() -> Option<u64> {
    // SAFETY: no preconditions; returns 0 when the clipboard is not accessible.
    match unsafe { GetClipboardSequenceNumber() } {
        0 => None,
        n => Some(u64::from(n)),
    }
}

fn format(name: &str) -> u32 {
    let name = wide(name);
    // SAFETY: `name` is NUL-terminated UTF-16 that outlives the call.
    unsafe { RegisterClipboardFormatW(name.as_ptr()) }
}

fn available(format: u32) -> bool {
    // SAFETY: only queries; does not need the clipboard to be open.
    format != 0 && unsafe { IsClipboardFormatAvailable(format) } != 0
}

/// The clipboard, opened by this thread until dropped.
struct Open;

impl Open {
    fn new() -> Option<Self> {
        for _ in 0..10 {
            // SAFETY: a null owner is allowed; on success we must close it (Drop).
            if unsafe { OpenClipboard(null_mut()) } != 0 {
                return Some(Open);
            }
            std::thread::sleep(Duration::from_millis(5)); // another app has it open
        }
        None
    }

    /// The DWORD stored under `format`, if there is one.
    fn read_u32(&self, format: u32) -> Option<u32> {
        // SAFETY: the clipboard is open (we hold `self`); the handle stays
        // owned by the clipboard and is not freed here.
        let handle = unsafe { GetClipboardData(format) };
        if handle.is_null() {
            return None;
        }
        // SAFETY: `handle` is a global memory handle owned by the clipboard.
        if unsafe { GlobalSize(handle) } < 4 {
            return None;
        }
        // SAFETY: as above; a successful lock is paired with GlobalUnlock below.
        let ptr = unsafe { GlobalLock(handle) };
        if ptr.is_null() {
            return None;
        }
        // SAFETY: at least 4 bytes are locked at `ptr` (checked above);
        // read_unaligned has no alignment requirement.
        let value = unsafe { std::ptr::read_unaligned(ptr.cast::<u32>()) };
        // SAFETY: balances the successful GlobalLock above.
        unsafe { GlobalUnlock(handle) };
        Some(value)
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: this thread opened the clipboard in `Open::new`.
        unsafe { CloseClipboard() };
    }
}

/// Put `text` on the clipboard together with extra formats holding the given
/// bytes, the way a password manager adds its privacy marks. For tests.
pub fn write_text_with_formats(text: &str, extra: &[(&str, &[u8])]) -> io::Result<()> {
    const CF_UNICODETEXT: u32 = 13;
    let _open = Open::new().ok_or_else(|| io::Error::other("the clipboard is busy"))?;
    // SAFETY: the clipboard is open on this thread (`_open`).
    if unsafe { EmptyClipboard() } == 0 {
        return Err(io::Error::last_os_error());
    }
    let utf16: Vec<u8> = text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    let mut formats = vec![(CF_UNICODETEXT, utf16)];
    formats.extend(extra.iter().map(|(name, data)| (format(name), data.to_vec())));
    for (fmt, data) in formats {
        // SAFETY: a fresh movable block of `data.len()` bytes; checked for null.
        let block = unsafe { GlobalAlloc(GMEM_MOVEABLE, data.len().max(1)) };
        if block.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `block` is ours and at least `data.len()` bytes; the lock is
        // checked and released before the block is handed over.
        unsafe {
            let p = GlobalLock(block).cast::<u8>();
            if p.is_null() {
                GlobalFree(block);
                return Err(io::Error::last_os_error());
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
            GlobalUnlock(block);
        }
        // SAFETY: the clipboard is open; on success it owns `block`, so we
        // free it only if the call failed.
        if unsafe { SetClipboardData(fmt, block) }.is_null() {
            let e = io::Error::last_os_error();
            // SAFETY: the clipboard refused the block, so it is still ours.
            unsafe { GlobalFree(block) };
            return Err(e);
        }
    }
    Ok(())
}

pub fn marked_private() -> bool {
    // Set by apps (password managers among them) that ask clipboard monitors
    // to ignore this copy altogether.
    if available(format("ExcludeClipboardContentFromMonitorProcessing")) || available(format("Clipboard Viewer Ignore"))
    {
        return true;
    }
    // A DWORD: 0 keeps the copy out of clipboard history / cloud sync.
    let flags: Vec<u32> = ["CanIncludeInClipboardHistory", "CanUploadToCloudClipboard"]
        .map(format)
        .into_iter()
        .filter(|f| available(*f))
        .collect();
    if flags.is_empty() {
        return false;
    }
    // A flag is present but unreadable: assume the app meant it (fail closed).
    let Some(open) = Open::new() else { return true };
    flags.iter().any(|f| open.read_u32(*f).map_or(true, |v| v == 0))
}

#[cfg(test)]
mod tests {
    //! Tests that write the real clipboard are `#[ignore]`d: they would
    //! clobber what the person running the tests has copied (each restores
    //! the text afterwards) and need an interactive desktop. Run them with
    //! `cargo test -p tether-os -- --ignored --test-threads=1`.

    use super::*;
    use std::time::Instant;

    fn write(text: &str, extra: &[(&str, &[u8])]) {
        write_text_with_formats(text, extra).expect("write the clipboard");
    }

    fn keep_user_text() -> impl Drop {
        struct Restore(Option<String>);
        impl Drop for Restore {
            fn drop(&mut self) {
                if let (Some(text), Ok(mut c)) = (self.0.take(), arboard::Clipboard::new()) {
                    let _ = c.set_text(text);
                }
            }
        }
        Restore(arboard::Clipboard::new().ok().and_then(|mut c| c.get_text().ok()))
    }

    #[test]
    fn the_watcher_starts_and_stops_cleanly() {
        for _ in 0..3 {
            let w = watch(|| true).unwrap();
            assert_eq!(w.mode(), Mode::Notifications);
            drop(w); // joins: would hang if WM_QUIT were not delivered
        }
        let _ = (sequence(), marked_private()); // read-only; must not panic
    }

    #[test]
    #[ignore = "writes the real clipboard"]
    fn private_marks_are_seen() {
        let _restore = keep_user_text();
        write("plain", &[]);
        assert!(!marked_private());
        write("hunter2", &[("ExcludeClipboardContentFromMonitorProcessing", &[0])]);
        assert!(marked_private());
        write("hunter2", &[("CanUploadToCloudClipboard", &0u32.to_le_bytes())]);
        assert!(marked_private());
        write("hunter2", &[("CanIncludeInClipboardHistory", &0u32.to_le_bytes())]);
        assert!(marked_private());
        write("fine to share", &[("CanUploadToCloudClipboard", &1u32.to_le_bytes())]);
        assert!(!marked_private());
    }

    #[test]
    #[ignore = "writes the real clipboard"]
    fn changes_are_reported_quickly() {
        let _restore = keep_user_text();
        let (tx, rx) = mpsc::channel();
        let w = watch(move || tx.send(Instant::now()).is_ok()).unwrap();
        let mut lat = Vec::new();
        for i in 0..100 {
            std::thread::sleep(Duration::from_millis(20));
            while rx.try_recv().is_ok() {}
            let before = sequence();
            let start = Instant::now();
            write(&format!("latency probe {i}"), &[]);
            let seen = rx.recv_timeout(Duration::from_secs(1)).expect("a change is reported");
            lat.push(seen.saturating_duration_since(start));
            assert_ne!(sequence(), before, "the sequence number moves with each copy");
        }
        drop(w);
        lat.sort();
        let (p50, p99, max) = (lat[49], lat[98], lat[99]);
        println!("clipboard change to callback over 100 copies: p50 {p50:?}, p99 {p99:?}, max {max:?}");
        assert!(p99 < Duration::from_millis(100), "p99 {p99:?}");
    }
}
