//! Windows GUI apps must not spawn console windows for helper subprocesses.
//! `CREATE_NO_WINDOW` (0x08000000) tells CreateProcess to skip allocating a
//! console, so helper commands (where.exe, npm, git, taskkill, …) run
//! invisibly instead of flashing a black terminal window.

/// Apply `CREATE_NO_WINDOW` to a `std::process::Command` on Windows.
/// No-op on other platforms.
#[cfg(target_os = "windows")]
pub fn no_window(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x08000000);
}

/// No-op on non-Windows platforms.
#[cfg(not(target_os = "windows"))]
#[inline]
pub fn no_window(_cmd: &mut std::process::Command) {}
