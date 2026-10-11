//! The tray icon. It greys out while "stop receiving" is on.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject,
};
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, DestroyIcon, HICON, ICONINFO, SM_CXSMICON};

const ID: u32 = 1;

pub struct Tray {
    hwnd: HWND,
    message: u32,
    on: HICON,
    off: HICON,
    tip: String,
}

/// Straight RGBA8 of a PNG.
fn decode(bytes: &[u8]) -> (Vec<u8>, usize, usize) {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut r = dec.read_info().expect("bundled icon");
    let mut buf = vec![0; r.output_buffer_size().expect("icon size")];
    let info = r.next_frame(&mut buf).expect("bundled icon");
    (buf, info.width as usize, info.height as usize)
}

/// An icon of `size`×`size` from a bigger square PNG, scaled down by averaging.
fn icon(png: &[u8], size: usize) -> HICON {
    let (rgba, w, _) = decode(png);
    let mut bgra = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let (x0, x1) = (x * w / size, ((x + 1) * w / size).max(x * w / size + 1));
            let (y0, y1) = (y * w / size, ((y + 1) * w / size).max(y * w / size + 1));
            let mut acc = [0f32; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let p = &rgba[(sy * w + sx) * 4..][..4];
                    let a = p[3] as f32;
                    acc[0] += p[0] as f32 * a;
                    acc[1] += p[1] as f32 * a;
                    acc[2] += p[2] as f32 * a;
                    acc[3] += a;
                }
            }
            let n = ((x1 - x0) * (y1 - y0)) as f32;
            let o = (y * size + x) * 4;
            if acc[3] > 0.0 {
                bgra[o] = (acc[2] / acc[3]).round() as u8;
                bgra[o + 1] = (acc[1] / acc[3]).round() as u8;
                bgra[o + 2] = (acc[0] / acc[3]).round() as u8;
            }
            bgra[o + 3] = (acc[3] / n).round() as u8;
        }
    }
    unsafe {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size as i32,
                biHeight: -(size as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let color = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0).expect("icon bitmap");
        std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits as *mut u8, bgra.len());
        // The alpha channel does the masking; the mask just has to exist.
        let mask = CreateBitmap(size as i32, size as i32, 1, 1, None);
        let icon = CreateIconIndirect(&ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        })
        .expect("icon");
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

impl Tray {
    /// Not shown until `show`. Clicks come to `hwnd` as `message`.
    pub fn new(hwnd: HWND, message: u32, tip: &str) -> Self {
        let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) }.max(16) as usize;
        Self {
            hwnd,
            message,
            on: icon(include_bytes!("../../assets/tray.png"), size),
            off: icon(include_bytes!("../../assets/tray-off.png"), size),
            tip: tip.to_owned(),
        }
    }

    fn data(&self, off: bool) -> NOTIFYICONDATAW {
        let mut d = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ID,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: self.message,
            hIcon: if off { self.off } else { self.on },
            ..Default::default()
        };
        for (dst, src) in d.szTip.iter_mut().zip(self.tip.encode_utf16().take(127)) {
            *dst = src;
        }
        d
    }

    /// Adds the icon (again, after Explorer restarts).
    pub fn show(&self, off: bool) {
        unsafe {
            if !Shell_NotifyIconW(NIM_ADD, &self.data(off)).as_bool() {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &self.data(off));
            }
        }
    }

    pub fn set_off(&self, off: bool) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &self.data(off));
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.data(false));
            let _ = DestroyIcon(self.on);
            let _ = DestroyIcon(self.off);
        }
    }
}
