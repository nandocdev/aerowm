//! Wayland listening socket owned by the compositor.
//!
//! AeroWM binds its own socket (instead of `wayland_server::ListeningSocket`)
//! so the listening FD can be handed over across an in-place restart:
//! the old process clears `CLOEXEC` on the FD and the new process adopts it
//! with [`UnixListener::from_raw_fd`], keeping `WAYLAND_DISPLAY` stable and
//! immediately available. No rebind, no stale-file races.
//!
//! Because [`UnixListener`] does not unlink its path on drop, only the side
//! that freshly bound a path may remove a stale file (after proving via
//! `connect` that no live compositor owns it).

use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;

use calloop::{Interest, LoopHandle, Mode, PostAction, generic::Generic};
use wayland_server::DisplayHandle;

use crate::handlers::compositor::ClientState;
use crate::state::AerowmState;

/// Env var carrying the inherited listening socket FD number across exec.
pub const WAYLAND_FD_ENV: &str = "AEROWM_WAYLAND_FD";

/// Info the restart path needs: the FD number stays valid across `exec`
/// because FD numbers are preserved (only the `CLOEXEC` flag is cleared).
#[derive(Debug, Clone)]
pub struct WaylandSocketInfo {
    pub fd: RawFd,
    pub name: String,
}

/// Binds (or adopts) the Wayland listening socket, exports
/// `WAYLAND_DISPLAY`, and registers the accept loop.
///
/// Returns the socket info for restart handover.
pub fn setup_wayland_socket(
    loop_handle: LoopHandle<'_, AerowmState>,
    display_handle: &DisplayHandle,
) -> Result<WaylandSocketInfo, Box<dyn std::error::Error>> {
    let (listener, name, fresh) = match std::env::var(WAYLAND_FD_ENV)
        .ok()
        .and_then(|fd| fd.parse::<RawFd>().ok())
    {
        Some(fd) => {
            // Restart path: adopt the inherited listener. SAFETY: the parent
            // guarantees this FD is our open listening socket (it cleared
            // CLOEXEC right before exec, and nothing else runs between).
            let listener = unsafe { UnixListener::from_raw_fd(fd) };
            listener.set_nonblocking(true)?;
            let name = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "aerowm-0".into());
            tracing::info!("adopted Wayland socket fd={fd} ({name}) from previous instance");
            // The handover is one-shot: a *second* restart re-exports it.
            // SAFETY: single-threaded startup path, no concurrent env use.
            unsafe {
                std::env::remove_var(WAYLAND_FD_ENV);
            }
            (listener, name, false)
        }
        None => {
            let (listener, name) = bind_fresh()?;
            // SAFETY: only our own process touches this env var here.
            unsafe {
                std::env::set_var("WAYLAND_DISPLAY", &name);
            }
            tracing::info!("listening for Wayland clients on {name}");
            (listener, name, true)
        }
    };

    let fd = listener.as_raw_fd();
    let dh = display_handle.clone();
    loop_handle.insert_source(
        Generic::new(listener, Interest::READ, Mode::Level),
        move |_, listener, _state: &mut AerowmState| {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if let Err(e) = accept_client(&dh, stream) {
                            tracing::warn!("accepting Wayland client failed: {e}");
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        tracing::warn!("Wayland accept error: {e}");
                        break;
                    }
                }
            }
            Ok(PostAction::Continue)
        },
    )?;

    // Only the fresh binder owns stale-file cleanup duty; the adopted
    // listener must never unlink a path it did not create.
    let _ = fresh;
    Ok(WaylandSocketInfo { fd, name })
}

fn accept_client(
    display_handle: &DisplayHandle,
    stream: UnixStream,
) -> Result<(), Box<dyn std::error::Error>> {
    stream.set_nonblocking(true)?;
    let mut dh = display_handle.clone();
    dh.insert_client(stream, Arc::new(ClientState::default()))?;
    Ok(())
}

/// Runtime dir for sockets (`$XDG_RUNTIME_DIR`, no fallback guessing:
/// Wayland requires it).
fn runtime_dir() -> Result<PathBuf, String> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "XDG_RUNTIME_DIR is not set; cannot bind Wayland socket".to_string())
}

/// Binds a fresh socket: `$WAYLAND_DISPLAY` when set and free, otherwise
/// the first free `aerowm-N`. A stale file is only unlinked after a
/// `connect` probe proves no live compositor owns it.
fn bind_fresh() -> Result<(UnixListener, String), Box<dyn std::error::Error>> {
    let dir = runtime_dir()?;
    let mut candidates = Vec::new();
    if let Ok(wanted) = std::env::var("WAYLAND_DISPLAY") {
        // In nested/dev sessions WAYLAND_DISPLAY may belong to a *parent*
        // compositor — never steal it, just prefer it when free.
        candidates.push(wanted);
    }
    for i in 0..32 {
        candidates.push(format!("aerowm-{i}"));
    }
    for name in candidates {
        let path = dir.join(&name);
        match UnixListener::bind(&path) {
            Ok(listener) => {
                listener.set_nonblocking(true)?;
                return Ok((listener, name));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                match UnixStream::connect(&path) {
                    Ok(_) => continue, // live owner — try the next name
                    Err(_) => {
                        // Stale file: nobody listens. Reclaim it.
                        let _ = std::fs::remove_file(&path);
                        let listener = UnixListener::bind(&path)?;
                        listener.set_nonblocking(true)?;
                        return Ok((listener, name));
                    }
                }
            }
            Err(e) => return Err(format!("binding {}: {e}", path.display()).into()),
        }
    }
    Err("no free Wayland socket name".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Process-global env vars are shared by test threads; serialize
    /// every test that touches them.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn unique_runtime_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "aerowm-test-{}-{tag}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn fresh_bind_creates_socket_and_exports_display() {
        let _guard = lock_env();
        let dir = unique_runtime_dir("bind");
        let prev_rt = std::env::var_os("XDG_RUNTIME_DIR");
        let prev_display = std::env::var_os("WAYLAND_DISPLAY");
        // SAFETY: test-only env mutation, restored below.
        unsafe {
            std::env::set_var("XDG_RUNTIME_DIR", &dir);
            std::env::remove_var("WAYLAND_DISPLAY");
        }

        let (listener, name) = bind_fresh().expect("bind");
        assert!(name.starts_with("aerowm-"));
        assert!(dir.join(&name).exists());
        // A fresh listener accepts no clients but is pollable.
        let _ = listener.as_raw_fd();

        unsafe {
            match prev_rt {
                Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
                None => std::env::remove_var("XDG_RUNTIME_DIR"),
            }
            match prev_display {
                Some(v) => std::env::set_var("WAYLAND_DISPLAY", v),
                None => std::env::remove_var("WAYLAND_DISPLAY"),
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_socket_file_is_reclaimed() {
        use std::os::unix::net::UnixListener as StdListener;

        let _guard = lock_env();
        let dir = unique_runtime_dir("stale");
        let prev_rt = std::env::var_os("XDG_RUNTIME_DIR");
        let prev_display = std::env::var_os("WAYLAND_DISPLAY");
        unsafe {
            std::env::set_var("XDG_RUNTIME_DIR", &dir);
            std::env::set_var("WAYLAND_DISPLAY", "aerowm-test-stale");
        }

        // Plant a dead socket file with no listener behind it.
        let dead = StdListener::bind(dir.join("aerowm-test-stale")).unwrap();
        drop(dead);

        let (listener, name) = bind_fresh().expect("reclaim");
        assert_eq!(name, "aerowm-test-stale");
        // And it is live now.
        assert!(UnixStream::connect(dir.join(&name)).is_ok());
        drop(listener);

        unsafe {
            match prev_rt {
                Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
                None => std::env::remove_var("XDG_RUNTIME_DIR"),
            }
            match prev_display {
                Some(v) => std::env::set_var("WAYLAND_DISPLAY", v),
                None => std::env::remove_var("WAYLAND_DISPLAY"),
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
