//! Alt + the "?" key, from a low-level keyboard hook. The key is found by where it is, not what it types: scan code
//! 0x35 is the key left of right Shift on every layout ("/?" on US keyboards). The key still reaches the
//! foreground app: the hook only watches.

use std::sync::atomic::{AtomicIsize, Ordering};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_RWIN};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, LLKHF_ALTDOWN, LLKHF_EXTENDED, PostMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
};

/// The key left of right Shift. With the extended flag it is the keypad's "/", which doesn't count.
const SCAN_SLASH: u32 = 0x35;

static TARGET: AtomicIsize = AtomicIsize::new(0);
static MESSAGE: AtomicIsize = AtomicIsize::new(0);

fn down(vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) < 0 }
}

unsafe extern "system" fn proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN) {
        let k = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        // Alt, and neither Ctrl (AltGr is Ctrl+Alt) nor Win. Holding the key repeats: the send limit sorts that out.
        if k.scanCode == SCAN_SLASH
            && (k.flags & LLKHF_EXTENDED).0 == 0
            && (k.flags & LLKHF_ALTDOWN).0 != 0
            && !down(VK_CONTROL)
            && !down(VK_LWIN)
            && !down(VK_RWIN)
        {
            // Only a post: a hook that takes long gets dropped by Windows.
            let hwnd = HWND(TARGET.load(Ordering::Relaxed) as *mut _);
            unsafe {
                let _ = PostMessageW(Some(hwnd), MESSAGE.load(Ordering::Relaxed) as u32, WPARAM(0), LPARAM(0));
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

pub struct Hook(HHOOK);

impl Hook {
    /// Posts `message` to `hwnd` on every Alt+?. Needs a message loop on this thread.
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
