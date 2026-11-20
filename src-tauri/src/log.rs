//! Journal développeur : %APPDATA%\app.cap.focus\cap.log (jamais affiché à l'utilisateur, i18n-ok).
//! Une ligne par événement, horodatée en heure locale. Au-delà de 512 Ko, l'ancien passe en cap.old.log.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());
const MAX: u64 = 512 * 1024;

pub fn dir() -> PathBuf {
    std::env::var("APPDATA").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(".")).join("app.cap.focus")
}

pub fn path() -> PathBuf {
    dir().join("cap.log")
}

#[cfg(windows)]
fn stamp() -> String {
    unsafe {
        let mut t: windows_sys::Win32::Foundation::SYSTEMTIME = std::mem::zeroed();
        windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut t);
        format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds)
    }
}

#[cfg(not(windows))]
fn stamp() -> String {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    format!("{ms}")
}

/// Écrit une ligne. Ne panique jamais, ne bloque jamais longtemps.
pub fn line(msg: &str) {
    let Ok(_g) = LOCK.try_lock().or_else(|_| LOCK.lock()) else { return };
    let p = path();
    let _ = std::fs::create_dir_all(dir());
    if std::fs::metadata(&p).map(|m| m.len() > MAX).unwrap_or(false) {
        let _ = std::fs::rename(&p, dir().join("cap.old.log"));
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{} {}", stamp(), msg);
    }
}

#[macro_export]
macro_rules! clog {
    ($($a:tt)*) => { $crate::log::line(&format!($($a)*)) };
}

/// Plantage natif (violation d'accès, etc.) : on note le code et l'adresse avant que Windows ferme Cap.
#[cfg(windows)]
pub fn install_native_crash_hook() {
    use windows_sys::Win32::System::Diagnostics::Debug::{SetUnhandledExceptionFilter, EXCEPTION_POINTERS};
    unsafe extern "system" fn filter(info: *const EXCEPTION_POINTERS) -> i32 {
        unsafe {
            let (code, addr) = if !info.is_null() && !(*info).ExceptionRecord.is_null() {
                let r = &*(*info).ExceptionRecord;
                (r.ExceptionCode as u32, r.ExceptionAddress as usize)
            } else {
                (0, 0)
            };
            let base = windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null()) as usize;
            line(&format!("PLANTAGE NATIF code=0x{code:08X} adresse=0x{addr:X} (cap.exe+0x{:X})", addr.wrapping_sub(base)));
        }
        0 // EXCEPTION_CONTINUE_SEARCH
    }
    unsafe {
        SetUnhandledExceptionFilter(Some(filter));
    }
}

#[cfg(not(windows))]
pub fn install_native_crash_hook() {}
