//! The Windows app: a hidden window that owns the tray icon, hears the hotkey and the network, and puts each "?" on
//! screen.

mod hook;
mod overlay;
mod prefs;
mod sound;
mod tray;

use crate::anim::{self, Sprite};
use crate::net::{self, Net};
use crate::proto::{KIND_MISSING, PORT};
use crate::spot::{Bursts, Rect};
use overlay::Fx;
use prefs::Prefs;
use sound::Sound;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Globalization::GetUserDefaultUILanguage;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW,
    GetCursorPos, GetMessageW, KillTimer, MB_ICONINFORMATION, MB_OK, MF_CHECKED, MF_GRAYED, MF_SEPARATOR, MF_STRING,
    MSG, MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, SetForegroundWindow,
    SetTimer, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WINDOW_EX_STYLE, WM_APP,
    WM_CONTEXTMENU, WM_DESTROY, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW,
    WS_OVERLAPPED,
};
use windows::core::{HSTRING, PCWSTR, w};

const WM_TRAY: u32 = WM_APP + 1;
const WM_ALT_SLASH: u32 = WM_APP + 2;
/// A ping to show: the origin in wParam, 1 in lParam when it is our own.
const WM_SHOW: u32 = WM_APP + 3;
const FRAME_TIMER: usize = 1;
/// Debug builds only: `--auto-send <ms>` presses Alt+? every so often, for testing without a keyboard.
const AUTO_SEND_TIMER: usize = 2;
const FRAME_MS: u32 = 15;
/// Safety ceiling, as in the original: past this many on screen, the oldest go early.
const MAX_ON_SCREEN: usize = 60;
const MAIN_CLASS: PCWSTR = w!("lolPingLite");
const FX_CLASS: PCWSTR = w!("lolPingLiteFx");

/// Explorer's "the taskbar is back" broadcast: the tray icon has to be added again.
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

struct Text {
    tip: &'static str,
    title: &'static str,
    master: fn(usize) -> String,
    member: fn(usize) -> String,
    no_network: fn(&str) -> String,
    stop_receiving: &'static str,
    show_own: &'static str,
    autostart: &'static str,
    quit: &'static str,
    already_running: &'static str,
}

const ZH: Text = Text {
    tip: "lolPing Lite（Alt + ?）",
    title: "lolPing Lite · Alt + ?",
    master: |n| format!("本机是主节点 · 在线 {n} 人"),
    member: |n| format!("在线 {n} 人"),
    no_network: |e| format!("网络不可用：{e}"),
    stop_receiving: "停止接收",
    show_own: "显示自己发出的“？”",
    autostart: "开机自启",
    quit: "退出",
    already_running: "lolPing Lite 已经在运行了，图标在任务栏右下角的托盘里。",
};

const EN: Text = Text {
    tip: "lolPing Lite (Alt + ?)",
    title: "lolPing Lite · Alt + ?",
    master: |n| format!("Master · {n} online"),
    member: |n| format!("{n} online"),
    no_network: |e| format!("No network: {e}"),
    stop_receiving: "Stop receiving",
    show_own: "Show my own \"?\"",
    autostart: "Start with Windows",
    quit: "Quit",
    already_running: "lolPing Lite is already running: its icon is in the tray.",
};

fn text() -> &'static Text {
    const LANG_CHINESE: u16 = 0x04;
    if unsafe { GetUserDefaultUILanguage() } & 0x3ff == LANG_CHINESE { &ZH } else { &EN }
}

/// Debug builds log everything to their console; release builds keep a small file for when something goes wrong.
struct Logger {
    file: Option<Mutex<std::fs::File>>,
}

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::max_level()
    }

    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let line = format!(
            "{:02}:{:02}:{:02} UTC {:5} {}\n",
            secs / 3600 % 24,
            secs / 60 % 60,
            secs % 60,
            r.level(),
            r.args()
        );
        match &self.file {
            Some(f) => {
                let _ = f.lock().unwrap().write_all(line.as_bytes());
            }
            None => eprint!("{line}"),
        }
    }

    fn flush(&self) {}
}

fn init_log() {
    let file = if cfg!(debug_assertions) {
        None
    } else {
        std::env::var_os("LOCALAPPDATA").and_then(|dir| {
            let dir = std::path::Path::new(&dir).join("lolPing Lite");
            std::fs::create_dir_all(&dir).ok()?;
            let path = dir.join("lolping-lite.log");
            // Started afresh once it passes 1 MB.
            let big = std::fs::metadata(&path).is_ok_and(|m| m.len() > 1 << 20);
            let f = std::fs::OpenOptions::new().create(true).append(!big).write(true).truncate(big).open(path).ok()?;
            Some(Mutex::new(f))
        })
    };
    let level = if cfg!(debug_assertions) { log::LevelFilter::Debug } else { log::LevelFilter::Info };
    if log::set_boxed_logger(Box::new(Logger { file })).is_ok() {
        log::set_max_level(level);
    }
}

struct App {
    hwnd: HWND,
    instance: HINSTANCE,
    text: &'static Text,
    net: Result<Net, String>,
    tray: tray::Tray,
    prefs: Prefs,
    /// On screen, each with the dpi it was drawn for.
    fx: Vec<(u32, Fx)>,
    art: (Vec<u8>, usize, usize),
    sprites: HashMap<u32, Sprite>,
    bursts: Bursts,
    rng: fastrand::Rng,
    sound: Option<Sound>,
    start: Instant,
    ticking: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` on the app, unless it is busy further up the stack (a menu's message loop) or gone.
fn with<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok()?.as_mut().map(f))
}

fn dpi(monitor: HMONITOR) -> u32 {
    let (mut x, mut y) = (96, 96);
    unsafe {
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y);
    }
    x.max(48)
}

impl App {
    /// The display under the mouse: where a new burst lands.
    fn cursor_display() -> Rect {
        unsafe {
            let mut p = POINT::default();
            let _ = GetCursorPos(&mut p);
            let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST), &mut info);
            let r = info.rcMonitor;
            Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
        }
    }

    fn show(&mut self, origin: u64, own: bool) {
        log::debug!("ping from {origin:016x}{}", if own { " (mine)" } else { "" });
        if (own && !self.prefs.show_own) || (!own && self.prefs.stop_receiving) {
            return;
        }
        let display = Self::cursor_display();
        let k_here =
            dpi(unsafe { MonitorFromPoint(POINT { x: display.left, y: display.top }, MONITOR_DEFAULTTONEAREST) });
        let now = self.start.elapsed().as_millis() as u64;
        let (_, (x, y)) = self.bursts.place(origin, now, display, anim::SIZE * k_here as f64 / 96.0, &mut self.rng);
        let d = dpi(unsafe { MonitorFromPoint(POINT { x: x as i32, y: y as i32 }, MONITOR_DEFAULTTONEAREST) });
        let k = d as f64 / 96.0;
        let Some(mut fx) = Fx::new(FX_CLASS, self.instance, x, y, k) else {
            log::warn!("can't open a window for a ping");
            return;
        };
        let (art, w, h) = &self.art;
        let sprite = self.sprites.entry(d).or_insert_with(|| Sprite::new(art, *w, *h, k));
        fx.frame(sprite);
        if self.fx.len() >= MAX_ON_SCREEN {
            self.fx.remove(0);
        }
        self.fx.push((d, fx));
        if let Some(s) = &mut self.sound {
            s.play();
        }
        if !self.ticking {
            self.ticking = true;
            unsafe { SetTimer(Some(self.hwnd), FRAME_TIMER, FRAME_MS, None) };
        }
    }

    fn frame(&mut self) {
        let sprites = &self.sprites;
        self.fx.retain_mut(|(d, fx)| sprites.get(d).is_some_and(|s| fx.frame(s)));
        let playing = self.sound.as_mut().is_some_and(Sound::reap);
        if self.fx.is_empty() && !playing && self.ticking {
            self.ticking = false;
            unsafe {
                let _ = KillTimer(Some(self.hwnd), FRAME_TIMER);
            }
        }
    }
}

const CMD_STOP: usize = 10;
const CMD_OWN: usize = 11;
const CMD_AUTOSTART: usize = 12;
const CMD_QUIT: usize = 13;

/// The tray menu. Built fresh each time, and shown with no borrow of the app held: it runs its own message loop.
fn menu(hwnd: HWND) {
    let Some((text, status, prefs)) = with(|a| {
        let status = match &a.net {
            Ok(n) => {
                let s = n.status();
                if s.is_master { (a.text.master)(s.online) } else { (a.text.member)(s.online) }
            }
            Err(e) => (a.text.no_network)(e),
        };
        (a.text, status, a.prefs)
    }) else {
        return;
    };
    let autostart = prefs::autostart();
    let check = |on: bool| if on { MF_STRING | MF_CHECKED } else { MF_STRING };
    let cmd = unsafe {
        let Ok(m) = CreatePopupMenu() else { return };
        let _ = AppendMenuW(m, MF_STRING | MF_GRAYED, 0, &HSTRING::from(text.title));
        let _ = AppendMenuW(m, MF_STRING | MF_GRAYED, 0, &HSTRING::from(status));
        let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(m, check(prefs.stop_receiving), CMD_STOP, &HSTRING::from(text.stop_receiving));
        let _ = AppendMenuW(m, check(prefs.show_own), CMD_OWN, &HSTRING::from(text.show_own));
        let _ = AppendMenuW(m, check(autostart), CMD_AUTOSTART, &HSTRING::from(text.autostart));
        let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(m, MF_STRING, CMD_QUIT, &HSTRING::from(text.quit));
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        // Without this the menu won't close when clicking elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, p.x, p.y, None, hwnd, None).0 as usize;
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(m);
        cmd
    };
    match cmd {
        CMD_STOP | CMD_OWN => {
            with(|a| {
                if cmd == CMD_STOP {
                    a.prefs.stop_receiving = !a.prefs.stop_receiving;
                    a.tray.set_off(a.prefs.stop_receiving);
                } else {
                    a.prefs.show_own = !a.prefs.show_own;
                }
                a.prefs.save();
            });
        }
        CMD_AUTOSTART => prefs::set_autostart(!autostart),
        CMD_QUIT => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        _ => {}
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            if matches!(lparam.0 as u32 & 0xffff, WM_RBUTTONUP | WM_LBUTTONUP | WM_CONTEXTMENU) {
                menu(hwnd);
            }
        }
        WM_ALT_SLASH => {
            with(|a| {
                if let Ok(n) = &a.net {
                    n.send(KIND_MISSING);
                }
            });
        }
        WM_SHOW => {
            with(|a| a.show(wparam.0 as u64, lparam.0 == 1));
        }
        WM_TIMER if wparam.0 == FRAME_TIMER => {
            with(App::frame);
        }
        WM_TIMER if wparam.0 == AUTO_SEND_TIMER => unsafe {
            let _ = PostMessageW(Some(hwnd), WM_ALT_SLASH, WPARAM(0), LPARAM(0));
        },
        WM_DESTROY => unsafe { PostQuitMessage(0) },
        m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            with(|a| a.tray.show(a.prefs.stop_receiving));
        }
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

unsafe extern "system" fn fx_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

pub fn run() {
    init_log();
    let text = text();
    unsafe {
        // Physical pixels everywhere: each "?" is drawn sharp at its display's own scale.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // One copy per user session: a second one could not have the port anyway.
        let _mutex = CreateMutexW(None, true, w!("Local\\lolPingLite"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            MessageBoxW(None, &HSTRING::from(text.already_running), w!("lolPing Lite"), MB_OK | MB_ICONINFORMATION);
            return;
        }

        let instance: HINSTANCE = GetModuleHandleW(None).expect("module").into();
        for (class, proc) in [(MAIN_CLASS, wndproc as _), (FX_CLASS, fx_wndproc as _)] {
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(proc),
                hInstance: instance,
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassExW(&wc);
        }
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
        // Never shown: it is there for the tray, the hook and the network to talk to.
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0),
            MAIN_CLASS,
            w!("lolPing Lite"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )
        .expect("main window");

        let target = hwnd.0 as isize;
        let net = net::start(move |origin, own| {
            let _ = PostMessageW(Some(HWND(target as *mut _)), WM_SHOW, WPARAM(origin as usize), LPARAM(own as isize));
        })
        .map_err(|e| {
            log::error!("can't open UDP port {PORT}: {e}");
            format!("UDP {PORT}: {e}")
        });

        let prefs = Prefs::load();
        let tray = tray::Tray::new(hwnd, WM_TRAY, text.tip);
        tray.show(prefs.stop_receiving);
        let sound = Sound::new();
        if sound.is_none() {
            log::warn!("the bundled sound can't be read");
        }
        APP.with(|a| {
            *a.borrow_mut() = Some(App {
                hwnd,
                instance,
                text,
                net,
                tray,
                prefs,
                fx: Vec::new(),
                art: anim::art(),
                sprites: HashMap::new(),
                bursts: Bursts::default(),
                rng: fastrand::Rng::new(),
                sound,
                start: Instant::now(),
                ticking: false,
            })
        });
        let hook = hook::Hook::install(hwnd, WM_ALT_SLASH);
        if let Err(e) = &hook {
            log::error!("can't watch the keyboard: {e}");
        }
        log::info!("running");
        if cfg!(debug_assertions) {
            let args: Vec<String> = std::env::args().collect();
            if let Some(ms) = args.iter().position(|a| a == "--auto-send").and_then(|i| args.get(i + 1)?.parse().ok()) {
                SetTimer(Some(hwnd), AUTO_SEND_TIMER, ms, None);
            }
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        drop(hook);
        // The tray icon, the windows and the sound go with the app; dropping the network handle stops its thread.
        APP.with(|a| a.borrow_mut().take());
        log::info!("quit");
    }
}
