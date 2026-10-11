//! The tray's switches, kept in the registry under HKCU, plus "start with Windows".

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_DWORD, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
    RegSetKeyValueW,
};
use windows::core::{HSTRING, PCWSTR, w};

const KEY: PCWSTR = w!("Software\\lolPing Lite");
const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("lolPing Lite");

#[derive(Debug, Clone, Copy)]
pub struct Prefs {
    /// Show my own "?" on my screen too. Off by default: the fun is on everyone else's.
    pub show_own: bool,
    /// Show nobody else's "?" (as the master, still pass them on).
    pub stop_receiving: bool,
}

fn get(name: PCWSTR) -> Option<bool> {
    let mut v = 0u32;
    let mut size = 4u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            KEY,
            name,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    (r == ERROR_SUCCESS).then_some(v != 0)
}

fn set(name: PCWSTR, on: bool) {
    let v = on as u32;
    let r =
        unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, KEY, name, REG_DWORD.0, Some(&v as *const u32 as *const _), 4) };
    if r != ERROR_SUCCESS {
        log::warn!("can't save a setting: {r:?}");
    }
}

impl Prefs {
    pub fn load() -> Self {
        Self {
            show_own: get(w!("ShowOwn")).unwrap_or(false),
            stop_receiving: get(w!("StopReceiving")).unwrap_or(false),
        }
    }

    pub fn save(&self) {
        set(w!("ShowOwn"), self.show_own);
        set(w!("StopReceiving"), self.stop_receiving);
    }
}

/// This exe, quoted, as the Run key wants it.
fn command() -> String {
    let mut buf = vec![0u16; 1024];
    let n = unsafe { GetModuleFileNameW(None, &mut buf) } as usize;
    format!("\"{}\"", String::from_utf16_lossy(&buf[..n]))
}

/// Whether Windows starts this exe (from where it is now) at sign-in.
pub fn autostart() -> bool {
    let mut buf = vec![0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            RUN_VALUE,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };
    if r != ERROR_SUCCESS {
        return false;
    }
    let n = (size as usize / 2).saturating_sub(1);
    String::from_utf16_lossy(&buf[..n]).eq_ignore_ascii_case(&command())
}

pub fn set_autostart(on: bool) {
    let r = if on {
        let cmd = HSTRING::from(command());
        let bytes = (cmd.len() + 1) * 2;
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                RUN_VALUE,
                REG_SZ.0,
                Some(cmd.as_ptr() as *const _),
                bytes as u32,
            )
        }
    } else {
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE) }
    };
    if r != ERROR_SUCCESS {
        log::warn!("can't change autostart: {r:?}");
    }
}
