//! Alt + the "?" key, from a low-level keyboard hook. The key is found by where it is, not what it types: scan code
//! 0x35 is the key left of right Shift on every layout ("/?" on US keyboards). The hook keeps the key to itself, so
//! a text box in the foreground doesn't get a "/" typed into it.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_CONTROL,
    VK_LWIN, VK_RWIN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, LLKHF_ALTDOWN, LLKHF_EXTENDED, PostMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

/// The key left of right Shift. With the extended flag it is the keypad's "/", which doesn't count.
const SCAN_SLASH: u32 = 0x35;
/// An unassigned virtual key. Pressed and released after a swallowed Alt+?, so that the Alt release that follows
/// isn't a lone Alt tap, which would put the foreground window's menu bar in focus.
const VK_MASK: VIRTUAL_KEY = VIRTUAL_KEY(0xE8);

static TARGET: AtomicIsize = AtomicIsize::new(0);
static MESSAGE: AtomicIsize = AtomicIsize::new(0);
/// The key went down as Alt+? and hasn't come up yet: its repeats and its release are ours too, Alt or not.
static HELD: AtomicBool = AtomicBool::new(false);

fn down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) < 0 }
}

fn mask_alt() {
    let key = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_MASK, dwFlags: flags, ..Default::default() } },
    };
    unsafe { SendInput(&[key(Default::default()), key(KEYEVENTF_KEYUP)], size_of::<INPUT>() as i32) };
}

unsafe extern "system" fn proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let k = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if k.scanCode == SCAN_SLASH && (k.flags & LLKHF_EXTENDED).0 == 0 {
            match wparam.0 as u32 {
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    let held = HELD.load(Ordering::Relaxed);
                    // Alt, and neither Ctrl (AltGr is Ctrl+Alt) nor Win. Holding the key repeats: the send limit
                    // sorts that out. Keys other programs inject can come without the Alt flag, but still as a
                    // system key.
                    let alt = (k.flags & LLKHF_ALTDOWN).0 != 0 || wparam.0 as u32 == WM_SYSKEYDOWN;
                    let alt_slash = alt && !down(VK_CONTROL) && !down(VK_LWIN) && !down(VK_RWIN);
                    if alt_slash {
                        // Only a post: a hook that takes long gets dropped by Windows.
                        let hwnd = HWND(TARGET.load(Ordering::Relaxed) as *mut _);
                        unsafe {
                            let _ =
                                PostMessageW(Some(hwnd), MESSAGE.load(Ordering::Relaxed) as u32, WPARAM(0), LPARAM(0));
                        }
                        if !held {
                            HELD.store(true, Ordering::Relaxed);
                            mask_alt();
                        }
                    }
                    if alt_slash || held {
                        return LRESULT(1);
                    }
                }
                WM_KEYUP | WM_SYSKEYUP if HELD.swap(false, Ordering::Relaxed) => return LRESULT(1),
                _ => {}
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

pub struct Hook(HHOOK);

impl Hook {
    /// Posts `message` to `hwnd` on every Alt+?, and keeps the key from the foreground app. Needs a message loop on
    /// this thread.
    pub fn install(hwnd: HWND, message: u32) -> windows::core::Result<Self> {
        TARGET.store(hwnd.0 as isize, Ordering::Relaxed);
        MESSAGE.store(message as isize, Ordering::Relaxed);
        let module = unsafe { GetModuleHandleW(None)? };
        Ok(Self(unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(proc), Some(module.into()), 0)? }))
    }
}

impl Drop for Hook {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWindowsHookEx(self.0);
        }
    }
}
