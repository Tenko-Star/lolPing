//! One "?" on screen: a small layered window, topmost, click-through, never focused, that redraws itself each frame.

use crate::anim::{self, Canvas, Sprite};
use std::time::Instant;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, HBITMAP, HDC, HGDIOBJ, SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, SW_SHOWNOACTIVATE, ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::PCWSTR;

pub struct Fx {
    hwnd: HWND,
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    canvas: Canvas,
    pos: POINT,
    born: Instant,
    shown: bool,
}

impl Fx {
    /// A "?" whose spot is (x, y) in physical screen pixels, at display scale `k`. Shows on its first frame.
    pub fn new(class: PCWSTR, instance: HINSTANCE, x: f64, y: f64, k: f64) -> Option<Self> {
        let canvas = Canvas::new(k);
        let (ox, oy) = canvas.origin();
        let pos = POINT { x: (x - ox).round() as i32, y: (y - oy).round() as i32 };
        let (w, h) = (canvas.w as i32, canvas.h as i32);
        unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class,
                PCWSTR::null(),
                WS_POPUP,
                pos.x,
                pos.y,
                w,
                h,
                None,
                None,
                Some(instance),
                None,
            )
            .ok()?;
            let dc = CreateCompatibleDC(None);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // top-down, like the canvas
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let Ok(bmp) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
                let _ = DeleteDC(dc);
                let _ = DestroyWindow(hwnd);
                return None;
            };
            let old = SelectObject(dc, bmp.into());
            Some(Self { hwnd, dc, bmp, old, bits: bits as *mut u8, canvas, pos, born: Instant::now(), shown: false })
        }
    }

    /// Draws the next frame. False once the ping is over: drop it then.
    pub fn frame(&mut self, sprite: &Sprite) -> bool {
        let t = self.born.elapsed().as_secs_f64();
        if t >= anim::LIFETIME {
            return false;
        }
        self.canvas.draw(t, sprite);
        let size = SIZE { cx: self.canvas.w as i32, cy: self.canvas.h as i32 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            std::ptr::copy_nonoverlapping(self.canvas.px.as_ptr(), self.bits, self.canvas.px.len());
            let _ = UpdateLayeredWindow(
                self.hwnd,
                None,
                Some(&self.pos),
                Some(&size),
                Some(self.dc),
                Some(&POINT::default()),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
            if !self.shown {
                self.shown = true;
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
        }
        true
    }
}

impl Drop for Fx {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bmp.into());
            let _ = DeleteDC(self.dc);
        }
    }
}
