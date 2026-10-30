//! The one screen read this app does: a small square of pixels under the
//! overlay, reduced to a median colour and discarded at once. Nothing is stored
//! or sent anywhere.

/// Median per-channel value of tightly packed BGRA pixels.
fn median_rgb_bgra(buf: &[u8]) -> Option<[u8; 3]> {
    let n = buf.len() / 4;
    if n == 0 {
        return None;
    }
    let mut out = [0u8; 3];
    for (channel, slot) in out.iter_mut().enumerate() {
        // BGRA → RGB order: channel 0 (R) lives at byte 2.
        let offset = 2 - channel;
        let mut values: Vec<u8> = buf.chunks_exact(4).map(|px| px[offset]).collect();
        values.sort_unstable();
        *slot = if n % 2 == 1 {
            values[n / 2]
        } else {
            ((u16::from(values[n / 2 - 1]) + u16::from(values[n / 2])) / 2) as u8
        };
    }
    Some(out)
}

#[cfg(windows)]
pub fn sample_bg(x: i32, y: i32, size: i32) -> Option<[u8; 3]> {
    use windows_sys::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
        ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
    };

    if size <= 0 {
        return None;
    }
    // SAFETY: plain GDI calls; every handle acquired here is released on all paths.
    unsafe {
        let screen = GetDC(std::ptr::null_mut());
        if screen.is_null() {
            return None;
        }
        let mem = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, size, size);
        let mut result = None;
        if !mem.is_null() && !bmp.is_null() {
            let old = SelectObject(mem, bmp);
            if BitBlt(mem, 0, 0, size, size, screen, x, y, SRCCOPY) != 0 {
                let mut info: BITMAPINFO = std::mem::zeroed();
                info.bmiHeader = BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: size,
                    biHeight: -size, // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..std::mem::zeroed()
                };
                let mut buf = vec![0u8; (size * size * 4) as usize];
                let lines = GetDIBits(mem, bmp, 0, size as u32, buf.as_mut_ptr().cast(), &mut info, DIB_RGB_COLORS);
                if lines != 0 {
                    result = median_rgb_bgra(&buf);
                }
            }
            SelectObject(mem, old);
        }
        if !bmp.is_null() {
            DeleteObject(bmp);
        }
        if !mem.is_null() {
            DeleteDC(mem);
        }
        ReleaseDC(std::ptr::null_mut(), screen);
        result
    }
}

#[cfg(not(windows))]
pub fn sample_bg(_x: i32, _y: i32, _size: i32) -> Option<[u8; 3]> {
    None
}

/// Strip the drop shadow, Windows 11 rounded corners and border so nothing but our own
/// pixels is visible. (Transparency itself comes from the wgpu surface's
/// pre-multiplied alpha.)
#[cfg(windows)]
pub fn make_transparent(title: &str) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
    use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW;

    const DWMWA_NCRENDERING_POLICY: u32 = 2;
    const DWMNCRP_DISABLED: u32 = 1;
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWA_BORDER_COLOR: u32 = 34;
    const DWMWCP_DONOTROUND: u32 = 1;
    const DWMWA_COLOR_NONE: u32 = 0xFFFF_FFFE;

    let wide: Vec<u16> = std::ffi::OsStr::new(title).encode_wide().chain(Some(0)).collect();
    // SAFETY: `wide` is NUL-terminated; the attribute pointers reference live locals of the right size.
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), wide.as_ptr());
        if hwnd.is_null() {
            return false;
        }
        let no_frame = DWMNCRP_DISABLED; // no drop shadow
        DwmSetWindowAttribute(hwnd, DWMWA_NCRENDERING_POLICY, (&raw const no_frame).cast(), 4);
        let corner = DWMWCP_DONOTROUND;
        let border = DWMWA_COLOR_NONE;
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, (&raw const corner).cast(), 4);
        DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, (&raw const border).cast(), 4);
        true
    }
}

#[cfg(not(windows))]
pub fn make_transparent(_title: &str) -> bool {
    true
}

/// Keep this app's own window out of screen captures, so the sample sees what
/// is *behind* the overlay rather than the overlay's own text and badge.
#[cfg(windows)]
pub fn exclude_from_capture(title: &str) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, SetWindowDisplayAffinity};

    const WDA_EXCLUDEFROMCAPTURE: u32 = 0x0000_0011;
    let wide: Vec<u16> = std::ffi::OsStr::new(title).encode_wide().chain(Some(0)).collect();
    // SAFETY: `wide` is NUL-terminated and outlives the call.
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), wide.as_ptr());
        !hwnd.is_null() && SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) != 0
    }
}

#[cfg(not(windows))]
pub fn exclude_from_capture(_title: &str) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(r: u8, g: u8, b: u8) -> [u8; 4] {
        [b, g, r, 255]
    }

    #[test]
    fn median_of_odd_count() {
        let buf: Vec<u8> = [px(10, 0, 0), px(30, 0, 0), px(20, 0, 0)].concat();
        assert_eq!(median_rgb_bgra(&buf), Some([20, 0, 0]));
    }

    #[test]
    fn median_of_even_count_truncates() {
        let buf: Vec<u8> = [px(0, 0, 1), px(0, 0, 2)].concat();
        assert_eq!(median_rgb_bgra(&buf), Some([0, 0, 1]));
    }

    #[test]
    fn empty_is_none() {
        assert_eq!(median_rgb_bgra(&[]), None);
    }
}
