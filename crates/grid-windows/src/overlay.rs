//! The grid overlay: one click-through window per monitor with per-pixel alpha.
//!
//! The look is "frosted glass": the screen behind is blurred (optional) and tinted, cells are
//! rounded panels, and the target gets a glowing outline. The overlay sits directly below the
//! window being dragged, so that window stays sharp and visible above it. Drawing is done in
//! software into a premultiplied BGRA bitmap that is handed to `UpdateLayeredWindow`. The tint
//! and cells only change when the drag starts, so they are drawn once into `base` and handed to
//! the (still hidden) windows then. Each update after that redraws and sends only the area
//! around the highlights.

use std::cell::RefCell;
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use grid_core::geometry::Rect;
use grid_core::settings::{Color, Overlay as Style};
use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, CreateCompatibleDC, DeleteDC, DeleteObject, HBITMAP, HDC, HGDIOBJ,
    SelectObject,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SetWindowPos, ShowWindow, ULW_ALPHA, UPDATELAYEREDWINDOWINFO,
    UpdateLayeredWindowIndirect, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::bitmap;
use crate::monitors::Monitor;
use crate::wide;

const CLASS: &str = "Grid.Overlay";

/// What the overlay shows. Rectangles are in global screen pixels, already inset by the gaps
/// (so they line up with where windows go).
#[derive(Clone, PartialEq)]
pub struct View {
    pub zones: Vec<Rect>,
    pub start: Option<Rect>,
    pub target: Option<Rect>,
    /// No gaps are set, so cells are drawn slightly inset to keep them apart.
    pub gapless: bool,
    pub style: Style,
}

/// One monitor's overlay window and its bitmap.
struct Surface {
    hwnd: HWND,
    bounds: Rect,
    scale: f32,
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    /// Tint and cells, drawn by `prepare` (or on first show).
    base: Option<Vec<u32>>,
    /// The area (in bitmap pixels) the highlights were last drawn over.
    drawn: Option<Rect>,
}

impl Surface {
    fn pixels(&mut self) -> &mut [u32] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.bounds.w * self.bounds.h) as usize) }
    }
}

#[derive(Default)]
struct State {
    surfaces: Vec<Surface>,
    visible: bool,
    view: Option<View>,
    /// The window being dragged; the overlay stays just below it.
    below: Option<HWND>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}

pub fn register_class(instance: HINSTANCE) {
    let class = wide(CLASS);
    let wc = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: class.as_ptr(),
        ..unsafe { zeroed() }
    };
    unsafe { RegisterClassW(&wc) };
}

/// Creates hidden overlay windows covering each monitor, to be shown just below `dragged`.
pub fn create(monitors: &[Monitor], blur: bool, dragged: HWND) {
    destroy();
    let class = wide(CLASS);
    let instance = unsafe { GetModuleHandleW(null()) };
    let surfaces = monitors
        .iter()
        .filter_map(|m| {
            let b = m.bounds;
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    class.as_ptr(),
                    null(),
                    WS_POPUP,
                    b.x,
                    b.y,
                    b.w,
                    b.h,
                    null_mut(),
                    null_mut(),
                    instance,
                    null(),
                )
            };
            if hwnd.is_null() {
                return None;
            }
            let Some((dc, bitmap, old, bits)) = create_bitmap(b.w, b.h) else {
                unsafe { DestroyWindow(hwnd) };
                return None;
            };
            if blur {
                enable_blur(hwnd);
            }
            Some(Surface { hwnd, bounds: b, scale: m.scale, dc, bitmap, old, bits, base: None, drawn: None })
        })
        .collect();
    STATE.with(|s| *s.borrow_mut() = State { surfaces, visible: false, view: None, below: Some(dragged) });
}

/// Draws the parts of `view` that stay the same during a drag (tint and cells) and hands them to
/// the hidden windows, so the first `show` only has to add the highlights.
pub fn prepare(view: &View) {
    STATE.with(|s| {
        let Ok(mut s) = s.try_borrow_mut() else { return };
        for surface in &mut s.surfaces {
            let base = draw_base(surface.bounds, surface.scale, view);
            surface.pixels().copy_from_slice(&base);
            surface.base = Some(base);
            surface.drawn = None;
            push(surface, None);
        }
    });
}

/// Shows the overlay with `view`, redrawing only if something changed.
pub fn show(view: View) {
    STATE.with(|s| {
        let Ok(mut s) = s.try_borrow_mut() else { return };
        if s.visible && s.view.as_ref() == Some(&view) {
            return;
        }
        let newly_visible = !s.visible;
        let below = s.below.unwrap_or(null_mut());
        for surface in &mut s.surfaces {
            render(surface, &view);
            unsafe {
                if newly_visible {
                    ShowWindow(surface.hwnd, SW_SHOWNOACTIVATE);
                }
                // Kept just below the dragged window, which may have been raised since.
                SetWindowPos(
                    surface.hwnd,
                    below,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                );
            }
        }
        s.view = Some(view);
        s.visible = true;
    });
}

pub fn hide() {
    STATE.with(|s| {
        let Ok(mut s) = s.try_borrow_mut() else { return };
        s.visible = false;
        for surface in &s.surfaces {
            unsafe { ShowWindow(surface.hwnd, SW_HIDE) };
        }
    });
}

pub fn destroy() {
    let surfaces = STATE.with(|s| std::mem::take(&mut *s.borrow_mut()).surfaces);
    for s in surfaces {
        unsafe {
            DestroyWindow(s.hwnd);
            SelectObject(s.dc, s.old);
            DeleteObject(s.bitmap);
            DeleteDC(s.dc);
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// A 32-bit bitmap selected into a memory DC.
fn create_bitmap(w: i32, h: i32) -> Option<(HDC, HBITMAP, HGDIOBJ, *mut u32)> {
    let (bitmap, bits) = bitmap::create(w, h)?;
    unsafe {
        let dc = CreateCompatibleDC(null_mut());
        let old = SelectObject(dc, bitmap);
        Some((dc, bitmap, old, bits))
    }
}

/// Blurs whatever is behind the window. Uses `SetWindowCompositionAttribute`, which Windows
/// doesn't document; if it is missing the overlay is simply not blurred. (Its acrylic mode was
/// tried for a stronger blur; on these layered windows it looks the same.)
fn enable_blur(hwnd: HWND) {
    #[repr(C)]
    struct AccentPolicy {
        state: u32,
        flags: u32,
        gradient: u32,
        animation: u32,
    }
    #[repr(C)]
    struct CompositionData {
        attribute: u32,
        data: *mut c_void,
        size: usize,
    }
    const ACCENT_ENABLE_BLURBEHIND: u32 = 3;
    const WCA_ACCENT_POLICY: u32 = 19;
    type SetAttribute = unsafe extern "system" fn(HWND, *mut CompositionData) -> i32;
    unsafe {
        let user32 = GetModuleHandleW(wide("user32.dll").as_ptr());
        let Some(set) = GetProcAddress(user32, c"SetWindowCompositionAttribute".as_ptr().cast()) else { return };
        let set: SetAttribute = std::mem::transmute(set);
        let mut accent = AccentPolicy { state: ACCENT_ENABLE_BLURBEHIND, flags: 0, gradient: 0, animation: 0 };
        let mut data = CompositionData {
            attribute: WCA_ACCENT_POLICY,
            data: &mut accent as *mut AccentPolicy as *mut c_void,
            size: size_of::<AccentPolicy>(),
        };
        set(hwnd, &mut data);
    }
}

/// Redraws the highlights of `view` and sends the changed area to the window.
fn render(surface: &mut Surface, view: &View) {
    let (bounds, scale) = (surface.bounds, surface.scale);
    let Some(base) = surface.base.take() else {
        // Not prepared: draw everything.
        let mut base = None;
        compose(&mut base, surface.pixels(), bounds, scale, view);
        surface.base = base;
        surface.drawn = highlight_area(bounds, scale, view);
        push(surface, None);
        return;
    };
    let old = surface.drawn;
    let new = redraw(surface.pixels(), &base, old, bounds, scale, view);
    surface.base = Some(base);
    surface.drawn = new;
    let dirty = match (old, new) {
        (Some(a), Some(b)) => a.union(&b),
        (a, b) => match a.or(b) {
            Some(r) => r,
            None => return,
        },
    };
    push(surface, Some(dirty));
}

/// Updates `pixels`, which show the grid from `base` with highlights drawn over `old`, to show
/// `view`. Returns the area the new highlights cover.
fn redraw(pixels: &mut [u32], base: &[u32], old: Option<Rect>, size: Rect, scale: f32, view: &View) -> Option<Rect> {
    if let Some(r) = old {
        // Put back the grid where the previous highlights were.
        for y in r.y..r.bottom() {
            let row = (y * size.w + r.x) as usize..(y * size.w + r.right()) as usize;
            pixels[row.clone()].copy_from_slice(&base[row]);
        }
    }
    draw_highlights(pixels, size, scale, view);
    highlight_area(size, scale, view)
}

/// Hands the bitmap to the window; only `dirty` (bitmap pixels) if given.
fn push(surface: &Surface, dirty: Option<Rect>) {
    let bounds = surface.bounds;
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let (dst, src, size) = (POINT { x: bounds.x, y: bounds.y }, POINT { x: 0, y: 0 }, SIZE { cx: bounds.w, cy: bounds.h });
    let dirty = dirty.map(|r| windows_sys::Win32::Foundation::RECT { left: r.x, top: r.y, right: r.right(), bottom: r.bottom() });
    let info = UPDATELAYEREDWINDOWINFO {
        cbSize: size_of::<UPDATELAYEREDWINDOWINFO>() as u32,
        hdcDst: null_mut(),
        pptDst: &dst,
        psize: &size,
        hdcSrc: surface.dc,
        pptSrc: &src,
        crKey: 0,
        pblend: &blend,
        dwFlags: ULW_ALPHA,
        prcDirty: dirty.as_ref().map_or(null(), |r| r as *const _),
    };
    unsafe { UpdateLayeredWindowIndirect(surface.hwnd, &info) };
}

/// Draws `view` for the monitor at `size` into `pixels`. The tint and cells don't change during
/// a drag, so they are drawn into `base` the first time and copied from it after that.
fn compose(base: &mut Option<Vec<u32>>, pixels: &mut [u32], size: Rect, scale: f32, view: &View) {
    let base = base.get_or_insert_with(|| draw_base(size, scale, view));
    pixels.copy_from_slice(base);
    draw_highlights(pixels, size, scale, view);
}

/// Cell inset and corner radius, in pixels.
fn cell_metrics(scale: f32, view: &View) -> (i32, f32) {
    let px = |v: f32| (v * scale).round().max(1.0) as i32;
    // Without gaps the cells would touch; a small inset keeps the rounded panels apart.
    (if view.gapless { px(3.0) } else { 0 }, px(8.0) as f32)
}

/// The tint and the cells of `view`, for the monitor at `size`.
fn draw_base(size: Rect, scale: f32, view: &View) -> Vec<u32> {
    let local = |r: Rect| Rect::new(r.x - size.x, r.y - size.y, r.w, r.h);
    let style = &view.style;
    let px = |v: f32| (v * scale).round().max(1.0) as i32;
    let (w, h) = (size.w, size.h);
    let (inset, radius) = cell_metrics(scale, view);
    // Frosted panels only with blur; without it, cells are just outlines.
    let panel = style.blur.then_some((Color::rgb(255, 255, 255), 0.06));
    let mut fresh = vec![0u32; (w * h) as usize];
    let mut canvas = Canvas { pixels: &mut fresh, w, h };
    canvas.fill_all(style.tint_color, style.opacity * 0.6);
    for zone in view.zones.iter().map(|z| local(*z)) {
        let cell = Rect::new(zone.x + inset, zone.y + inset, zone.w - 2 * inset, zone.h - 2 * inset);
        canvas.rounded(cell, radius, panel, Some((style.line_color, 0.45, px(1.0))));
    }
    fresh
}

/// The area (in bitmap pixels, clipped to the monitor) that `draw_highlights` draws over.
fn highlight_area(size: Rect, scale: f32, view: &View) -> Option<Rect> {
    let local = |r: Rect| Rect::new(r.x - size.x, r.y - size.y, r.w, r.h);
    let px = |v: f32| (v * scale).round().max(1.0) as i32;
    let start = view.start.map(local);
    // The glow and the outline reach this far outside the target.
    let target = view.target.map(|t| local(t).inflate(px(16.0) + px(2.0) + 1));
    let r = match (start, target) {
        (Some(a), Some(b)) => a.union(&b),
        (a, b) => a.or(b)?,
    };
    let (x0, y0) = (r.x.max(0), r.y.max(0));
    let (x1, y1) = (r.right().min(size.w), r.bottom().min(size.h));
    (x0 < x1 && y0 < y1).then(|| Rect::new(x0, y0, x1 - x0, y1 - y0))
}

/// Draws the start cell and the target of `view` over the grid in `pixels`.
fn draw_highlights(pixels: &mut [u32], size: Rect, scale: f32, view: &View) {
    let local = |r: Rect| Rect::new(r.x - size.x, r.y - size.y, r.w, r.h);
    let style = &view.style;
    let px = |v: f32| (v * scale).round().max(1.0) as i32;
    let (inset, radius) = cell_metrics(scale, view);
    let mut canvas = Canvas { pixels, w: size.w, h: size.h };

    if let Some(start) = view.start.map(local) {
        let cell = Rect::new(start.x + inset, start.y + inset, start.w - 2 * inset, start.h - 2 * inset);
        canvas.rounded(cell, radius, Some((style.start_color, 0.45)), Some((style.line_color, 0.9, px(2.0))));
    }
    if let Some(target) = view.target.map(local) {
        // The window covers the target; a soft glow and a crisp edge go around it.
        canvas.glow(target, px(16.0) as f32, style.target_color, 0.55);
        canvas.rounded(target.inflate(px(2.0)), px(8.0) as f32, None, Some((style.target_color, 1.0, px(2.0))));
    }
}

trait Inflate {
    fn inflate(self, by: i32) -> Self;
}

impl Inflate for Rect {
    fn inflate(self, by: i32) -> Rect {
        Rect::new(self.x - by, self.y - by, self.w + 2 * by, self.h + 2 * by)
    }
}

/// Premultiplied BGRA pixels (`0xAARRGGBB`) with simple anti-aliased shapes.
struct Canvas<'a> {
    pixels: &'a mut [u32],
    w: i32,
    h: i32,
}

/// Premultiplied pixel for `color` at `alpha` (0–1).
fn premultiply(color: Color, alpha: f32) -> u32 {
    let a = alpha.clamp(0.0, 1.0);
    let ch = |v: u8| (v as f32 * a + 0.5) as u32;
    ((a * 255.0 + 0.5) as u32) << 24 | ch(color.r) << 16 | ch(color.g) << 8 | ch(color.b)
}

/// `src` drawn over `dst` (both premultiplied).
fn over(src: u32, dst: u32) -> u32 {
    let inv = 255 - (src >> 24);
    let mix = |shift: u32| {
        let s = (src >> shift) & 0xFF;
        let d = (dst >> shift) & 0xFF;
        (s + (d * inv + 127) / 255).min(255) << shift
    };
    mix(24) | mix(16) | mix(8) | mix(0)
}

/// Signed distance from pixel centre (`x`, `y`) to a rounded rectangle; negative inside.
fn rounded_distance(r: Rect, radius: f32, x: i32, y: i32) -> f32 {
    let (cx, cy) = (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0);
    let radius = radius.min(r.w as f32 / 2.0).min(r.h as f32 / 2.0);
    let qx = (x as f32 + 0.5 - cx).abs() - (r.w as f32 / 2.0 - radius);
    let qy = (y as f32 + 0.5 - cy).abs() - (r.h as f32 / 2.0 - radius);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - radius
}

impl Canvas<'_> {
    fn fill_all(&mut self, color: Color, alpha: f32) {
        self.pixels.fill(premultiply(color, alpha));
    }


    /// Calls `f` for each pixel of `r` outside `hole`, and `inside` for each pixel in `hole`
    /// (both clipped to the canvas). Shapes only vary near their edges, so the middle is handled
    /// separately and much faster.
    fn each_split(
        &mut self,
        r: Rect,
        hole: Rect,
        mut f: impl FnMut(&mut u32, i32, i32),
        mut inside: Option<impl FnMut(&mut u32)>,
    ) {
        let (x0, y0) = (r.x.max(0), r.y.max(0));
        let (x1, y1) = (r.right().min(self.w), r.bottom().min(self.h));
        // Entirely off this monitor (e.g. a cell on another monitor): nothing to draw.
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let (hx0, hx1) = (hole.x.clamp(x0, x1), hole.right().clamp(x0, x1).max(hole.x.clamp(x0, x1)));
        for y in y0..y1 {
            let row = (y * self.w) as usize;
            let in_hole_row = hole.w > 0 && y >= hole.y && y < hole.bottom();
            let (a, b) = if in_hole_row { (hx0, hx1) } else { (x1, x1) };
            for x in (x0..a).chain(b..x1) {
                f(&mut self.pixels[row + x as usize], x, y);
            }
            if let Some(inside) = inside.as_mut() {
                for p in &mut self.pixels[row + a as usize..row + b as usize] {
                    inside(p);
                }
            }
        }
    }

    /// A rounded rectangle with an optional fill and an optional inside border `(color, alpha, width)`.
    fn rounded(&mut self, r: Rect, radius: f32, fill: Option<(Color, f32)>, stroke: Option<(Color, f32, i32)>) {
        if r.w <= 0 || r.h <= 0 {
            return;
        }
        // Away from the corners and the border, every pixel is just the fill.
        let band = radius.ceil() as i32 + stroke.map_or(0, |s| s.2) + 1;
        let hole = Rect::new(r.x + band, r.y + band, (r.w - 2 * band).max(0), (r.h - 2 * band).max(0));
        let solid = fill.map(|(c, a)| premultiply(c, a));
        // What's underneath is nearly always the same plain tint, so the last blend is reused.
        let inside = solid.map(|s| {
            let mut last = (over(s, 0), 0);
            move |p: &mut u32| {
                if *p != last.1 {
                    last = (over(s, *p), *p);
                }
                *p = last.0;
            }
        });
        self.each_split(r, hole, |p, x, y| {
            let d = rounded_distance(r, radius, x, y);
            let inside = (0.5 - d).clamp(0.0, 1.0);
            if inside <= 0.0 {
                return;
            }
            if let Some((c, a)) = fill {
                *p = over(premultiply(c, a * inside), *p);
            }
            if let Some((c, a, width)) = stroke {
                let edge = inside - (0.5 - (d + width as f32)).clamp(0.0, 1.0);
                if edge > 0.0 {
                    *p = over(premultiply(c, a * edge), *p);
                }
            }
        }, inside);
    }

    /// A soft glow fading out over `size` pixels around the outside of `r`.
    fn glow(&mut self, r: Rect, size: f32, color: Color, alpha: f32) {
        let s = size.ceil() as i32;
        // Nothing is drawn inside `r` (away from its rounded corners).
        let hole = Rect::new(r.x + 5, r.y + 5, (r.w - 10).max(0), (r.h - 10).max(0));
        self.each_split(r.inflate(s), hole, |p, x, y| {
            let d = rounded_distance(r, 4.0, x, y);
            if d > 0.0 && d < size {
                let t = 1.0 - d / size;
                *p = over(premultiply(color, alpha * t * t), *p);
            }
        }, None::<fn(&mut u32)>);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grid_core::geometry::Grid;
    use grid_core::zones::{apply_gaps, grid_zones};

    /// Each monitor's overlay is handed every cell, including the other monitors' ones, which
    /// lie off its edges. Those must be skipped, not crash.
    #[test]
    fn shapes_off_the_monitor_are_skipped() {
        let size = Rect::new(0, 0, 100, 100);
        let off = [Rect::new(150, 10, 40, 40), Rect::new(-90, 10, 40, 40), Rect::new(10, 300, 40, 40)];
        let view = View {
            zones: off.to_vec(),
            start: Some(off[0]),
            target: Some(off[1]),
            gapless: true,
            style: Style::default(),
        };
        let mut pixels = vec![0u32; 100 * 100];
        compose(&mut None, &mut pixels, size, 1.0, &view);
    }

    /// Redrawing only around the highlights must give the same picture as drawing everything,
    /// including where the previous highlight was, and near a monitor edge.
    #[test]
    fn partial_redraw_matches_full_draw() {
        let size = Rect::new(100, 50, 400, 300);
        let zones = grid_zones(size, Grid { cols: 8, rows: 8 }, false);
        let mut view = View { zones: zones.clone(), start: None, target: None, gapless: true, style: Style::default() };
        view.style.blur = true;
        let base = draw_base(size, 1.5, &view);
        let mut pixels = base.clone();
        let mut drawn = None;
        for (start, end) in [(18, 18), (18, 27), (0, 0), (63, 63), (9, 54)] {
            view.start = Some(zones[start]);
            view.target = Some(zones[start].union(&zones[end]));
            drawn = redraw(&mut pixels, &base, drawn, size, 1.5, &view);
            let mut full = vec![0; pixels.len()];
            compose(&mut None, &mut full, size, 1.5, &view);
            assert!(pixels == full, "cells {start}..{end}");
        }
    }

    /// Times the real steps of starting a drag and entering grid mode, with real overlay windows
    /// on this machine's monitors. Run with `--ignored --nocapture`, ideally with `--release`.
    #[test]
    #[ignore]
    fn drag_start_timing() {
        use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        register_class(unsafe { GetModuleHandleW(null()) });
        for blur in [false, true] {
            for round in 0..3 {
                let t = std::time::Instant::now();
                let monitors = crate::monitors::list();
                let list = t.elapsed();
                let zones: Vec<Rect> = monitors.iter().flat_map(|m| grid_zones(m.work, Grid { cols: 8, rows: 8 }, false)).collect();
                let style = Style { blur, ..Style::default() };
                let t = std::time::Instant::now();
                create(&monitors, blur, null_mut());
                let created = t.elapsed();
                let t = std::time::Instant::now();
                prepare(&View { zones: zones.clone(), start: None, target: None, gapless: true, style: style.clone() });
                let prepared = t.elapsed();
                let mut view = View { zones: zones.clone(), start: Some(zones[18]), target: Some(zones[18]), gapless: true, style };
                let t = std::time::Instant::now();
                show(view.clone());
                let first = t.elapsed();
                view.target = Some(zones[18].union(&zones[27]));
                let t = std::time::Instant::now();
                show(view);
                let next = t.elapsed();
                destroy();
                println!(
                    "blur {blur} #{round}: list {list:?}, create {created:?}, prepare {prepared:?}, first show {first:?}, next show {next:?}"
                );
            }
        }
    }

    /// Writes a sample overlay over a checkerboard "desktop" to `GRID_OVERLAY_PREVIEW` (a .bmp),
    /// to look at the design without dragging a window. Run with `--ignored`.
    #[test]
    #[ignore]
    fn preview() {
        let Ok(path) = std::env::var("GRID_OVERLAY_PREVIEW") else { return };
        // Optional, to compare looks: GRID_OVERLAY_OPACITY (0–1).
        let mut style = Style::default();
        if let Some(o) = std::env::var("GRID_OVERLAY_OPACITY").ok().and_then(|o| o.parse().ok()) {
            style.opacity = o;
        }
        let size = Rect::new(0, 0, 1440, 900);
        let gap = |r: Rect| apply_gaps(r, &[size], 10, 10);
        let zones: Vec<Rect> = grid_zones(size, Grid { cols: 8, rows: 8 }, false).into_iter().map(gap).collect();
        let view = View {
            start: Some(zones[18]),
            target: Some(gap(grid_zones(size, Grid { cols: 8, rows: 8 }, false)[18].union(&grid_zones(size, Grid { cols: 8, rows: 8 }, false)[36]))),
            zones,
            gapless: false,
            style,
        };
        let mut pixels = vec![0u32; (size.w * size.h) as usize];
        compose(&mut None, &mut pixels, size, 1.0, &view);
        // Blend over a stand-in desktop and save as a 24-bit bottom-up BMP.
        let mut bmp = Vec::new();
        let row = (size.w * 3 + 3) & !3;
        let data = (row * size.h) as u32;
        bmp.extend(b"BM");
        for v in [54 + data, 0, 54, 40] {
            bmp.extend(v.to_le_bytes());
        }
        bmp.extend(size.w.to_le_bytes());
        bmp.extend(size.h.to_le_bytes());
        bmp.extend(1u16.to_le_bytes());
        bmp.extend(24u16.to_le_bytes());
        for v in [0u32, data, 2835, 2835, 0, 0] {
            bmp.extend(v.to_le_bytes());
        }
        for y in (0..size.h).rev() {
            for x in 0..size.w {
                let p = pixels[(y * size.w + x) as usize];
                let bg: u32 = if (x / 60 + y / 60) % 2 == 0 { 0xD8D8D8 } else { 0x5A7FA8 };
                let c = over(p, 0xFF00_0000 | bg);
                bmp.extend([c as u8, (c >> 8) as u8, (c >> 16) as u8]);
            }
            bmp.resize(bmp.len() + (row - size.w * 3) as usize, 0);
        }
        std::fs::write(path, bmp).unwrap();
    }
}