use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

pub struct PtySession {
    pub writer: Arc<Mutex<Box<dyn Write + Send>>>,
    pub master: Box<dyn portable_pty::MasterPty + Send>,
    pub child: Box<dyn portable_pty::Child + Send + Sync>,
    pub cancelled: Arc<AtomicBool>,
}

impl Drop for PtySession {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        let _ = self.child.kill();
    }
}

pub struct PtyStore {
    pub sessions: Mutex<HashMap<String, PtySession>>,
}

impl PtyStore {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }
}

/// Write a tiny Node.js CJS patch to %TEMP% that forces isTTY=true on all stdio
/// streams. Returns the file path, or None if writing failed.
fn write_tty_patch() -> Option<String> {
    let tmp = std::env::temp_dir().join("agent_manager_tty_patch.cjs");
    let code = r#"'use strict';
// Force full TTY detection for TUI apps (e.g. Claude Code) inside ConPTY.
// ConPTY sets up a real PTY at the OS level, but Node.js sometimes doesn't
// propagate isTTY correctly through its stream layer, so we patch all three
// levels: stream properties, native tty module, and process.env hints.

// 1. Patch stream-level isTTY and setRawMode
['stdin','stdout','stderr'].forEach(s => {
  const stream = process[s];
  if (stream) {
    stream.isTTY = true;
    if (typeof stream.setRawMode !== 'function') {
      stream.setRawMode = function(mode) { return this; };
    }
    // Also patch _handle if present (internal Node.js detail)
    if (stream._handle && stream._handle.setRawMode == null) {
      stream._handle.setRawMode = () => {};
    }
  }
});

// 2. Patch native tty.isatty() — Claude Code calls this directly
try {
  const tty = require('tty');
  const _isatty = tty.isatty.bind(tty);
  tty.isatty = function(fd) {
    if (fd === 0 || fd === 1 || fd === 2) return true;
    return _isatty(fd);
  };
} catch(e) {}

// 3. Set TERM_PROGRAM so Claude Code recognises a known terminal
if (!process.env.TERM_PROGRAM) {
  process.env.TERM_PROGRAM = 'agent-manager';
}
if (!process.env.TERM) {
  process.env.TERM = 'xterm-256color';
}
// Make sure CI is NOT set — it forces non-interactive mode in many CLIs
delete process.env.CI;
"#;
    std::fs::write(&tmp, code).ok()?;
    tmp.to_str().map(|s| s.to_string())
}

/// Find powershell.exe — prefer pwsh (PowerShell 7) then fall back to Windows PowerShell.
#[cfg(windows)]
fn find_powershell() -> String {
    for candidate in &[
        r"C:\Program Files\PowerShell\7\pwsh.exe",
        r"C:\Program Files\PowerShell\7-preview\pwsh.exe",
    ] {
        if std::path::Path::new(candidate).exists() {
            return candidate.to_string();
        }
    }
    // Try PATH via where.exe
    if let Ok(out) = std::process::Command::new("where.exe").arg("pwsh").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            if let Some(line) = s.lines().next() {
                return line.trim().to_string();
            }
        }
    }
    "powershell.exe".to_string()
}

/// Resolve a command to (executable, leading_args, auto_run) suitable for PTY spawn.
/// `auto_run` is an optional string to write to stdin after the shell starts
/// (used when we launch an interactive shell that needs a command typed into it).
fn resolve_for_pty(command: &str, _args: &[String]) -> (String, Vec<String>, Option<String>) {
    #[cfg(windows)]
    {
        let bare = std::path::Path::new(command)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(command);

        // If there is a .ps1 npm wrapper, launch PowerShell interactively and
        // auto-type the command. This is the only way to give Node a real TTY:
        // PowerShell must be the interactive host process.
        let has_ps1: bool = std::env::var_os("APPDATA")
            .map(|d| std::path::Path::new(&d).join("npm").join(format!("{}.ps1", bare)).exists())
            .unwrap_or(false);

        if has_ps1 {
            let ps = find_powershell();
            // Launch interactive PowerShell, then write the command to its stdin.
            return (
                ps,
                vec!["-NoLogo".to_string()],
                Some(format!("{}\r\n", bare)),
            );
        }

        // Fallback: resolve to node + js entry with tty patch
        if let Some((node_exe, js_entry)) = crate::commands::resolve_npm_global_to_node(bare) {
            let patch_path = write_tty_patch();
            if let Some(patch) = patch_path {
                return (node_exe, vec![
                    "--require".to_string(),
                    patch,
                    js_entry,
                ], None);
            }
            return (node_exe, vec![js_entry], None);
        }

        // Any .cmd/.bat: run via PowerShell so the session stays alive
        let lower = command.to_lowercase();
        if lower.ends_with(".cmd") || lower.ends_with(".bat") {
            let ps = find_powershell();
            return (
                ps,
                vec![
                    "-NoLogo".to_string(),
                    "-NoExit".to_string(),
                    "-Command".to_string(),
                    format!("& '{}'", command),
                ],
                None,
            );
        }
    }
    (command.to_string(), vec![], None)
}


#[tauri::command]
pub fn pty_start(
    id: String,
    command: String,
    args: Vec<String>,
    cwd: String,
    env_vars: std::collections::HashMap<String, String>,
    cols: u16,
    rows: u16,
    store: tauri::State<'_, PtyStore>,
    app: AppHandle,
) -> Result<(), String> {
    // Stop any existing session before reusing its event channel.
    let previous = {
        let mut sessions = store.sessions.lock().unwrap();
        sessions.remove(&id)
    };
    drop(previous);

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| format!("Failed to open PTY: {e}"))?;

    // Resolve the real executable for PTY:
    // On Windows, .cmd wrappers must be unwrapped — cmd.exe /c exits immediately
    // after the child finishes, so the PTY gets no interactive session.
    // Instead, read the .cmd to find the actual node + script path.
    let (exe, leading_args, auto_run) = resolve_for_pty(&command, &args);

    let mut cmd = CommandBuilder::new(&exe);
    for a in &leading_args {
        cmd.arg(a);
    }
    // Only pass original args when NOT in auto_run (shell-host) mode.
    // In shell-host mode the command is typed via stdin, not as args.
    if auto_run.is_none() {
        for arg in &args {
            cmd.arg(arg);
        }
    }
    if !cwd.is_empty() {
        cmd.cwd(&cwd);
    }
    for (k, v) in &env_vars {
        cmd.env(k, v);
    }

    // Force color / TTY hints
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("TERM_PROGRAM", "agent-manager");
    #[cfg(windows)]
    {
        cmd.env("WT_SESSION", "1");
        cmd.env("FORCE_COLOR", "3");
        cmd.env("FORCE_TTY", "1");
        // Explicitly unset CI — many CLIs (including Claude Code) detect CI=true
        // and switch to non-interactive / plain-text mode, killing the TUI.
        cmd.env_remove("CI");
        cmd.env_remove("CONTINUOUS_INTEGRATION");
        cmd.env_remove("BUILD_ID");
        cmd.env_remove("GITHUB_ACTIONS");
    }

    let child = pair.slave
        .spawn_command(cmd)
        .map_err(|e| format!("Failed to spawn: {e}"))?;

    let raw_writer = pair.master.take_writer()
        .map_err(|e| format!("Failed to take writer: {e}"))?;
    let writer = Arc::new(Mutex::new(raw_writer));

    // If auto_run is set, wait for the shell prompt then write the command.
    // 800ms is enough for PowerShell 7 to print its first prompt.
    if let Some(cmd_text) = auto_run {
        let w = Arc::clone(&writer);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(800));
            if let Ok(mut wg) = w.lock() {
                let _ = wg.write_all(cmd_text.as_bytes());
                let _ = wg.flush();
            }
        });
    }

    let mut reader = pair.master.try_clone_reader()
        .map_err(|e| format!("Failed to clone reader: {e}"))?;

    // Background thread: read PTY output → emit Tauri event.
    // On Windows ConPTY, Ok(0) is NOT EOF — it just means no data yet.
    // Only an Err signals that the process has truly exited and the PTY closed.
    let id_clone = id.clone();
    let app_clone = app.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    let reader_cancelled = Arc::clone(&cancelled);
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while !reader_cancelled.load(Ordering::Acquire) {
            match reader.read(&mut buf) {
                Ok(0) => {
                    // Spurious empty read — spin briefly and retry
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(_) => {
                    if !reader_cancelled.load(Ordering::Acquire) {
                        let _ = app_clone.emit(&format!("pty-exit-{}", id_clone), ());
                    }
                    break;
                }
                Ok(n) => {
                    if !reader_cancelled.load(Ordering::Acquire) {
                        // Preserve byte boundaries. xterm's streaming UTF-8 decoder
                        // handles multibyte characters split across PTY reads.
                        let _ = app_clone.emit(
                            &format!("pty-data-{}", id_clone),
                            buf[..n].to_vec(),
                        );
                    }
                }
            }
        }
    });

    let session = PtySession {
        writer,
        master: pair.master,
        child,
        cancelled,
    };
    store.sessions.lock().unwrap().insert(id, session);
    Ok(())
}

#[tauri::command]
pub fn pty_resolve_debug(command: String) -> String {
    let (exe, args, auto_run) = resolve_for_pty(&command, &[]);
    format!("exe={:?}  args={:?}  auto_run={:?}", exe, args, auto_run)
}

#[tauri::command]
pub fn pty_write(id: String, data: String, store: tauri::State<'_, PtyStore>) -> Result<(), String> {
    let sessions = store.sessions.lock().unwrap();
    let session = sessions.get(&id).ok_or("PTY session not found")?;
    let mut w = session.writer.lock().map_err(|e| e.to_string())?;
    w.write_all(data.as_bytes()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn pty_resize(id: String, cols: u16, rows: u16, store: tauri::State<'_, PtyStore>) -> Result<(), String> {
    let sessions = store.sessions.lock().unwrap();
    let session = sessions.get(&id).ok_or("PTY session not found")?;
    session.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn pty_stop(id: String, store: tauri::State<'_, PtyStore>) -> Result<(), String> {
    store.sessions.lock().unwrap().remove(&id);
    Ok(())
}
