//! Grid's look for the windows it draws itself (splash screen, tray menu): the font, light or
//! dark, the colors and a few drawing helpers. Matches the settings window's theme
//! (`grid-settings/ui/theme.slint`).

use grid_core::settings::Appearance;
use windows_sys::Win32::Foundation::RECT;

/// Plus Jakarta Sans (SIL Open Font License), the same font file the settings window uses.
const FONT: &[u8] = include_bytes!("../../grid-settings/ui/fonts/PlusJakartaSans.ttf");

/// The glass look's colors (0xRRGGBB) and how opaque they are, for light or dark mode. Same as
/// `Glass` in theme.slint.
pub struct Colors {
    /// Laid over the blurred desktop: Grid's default overlay tint in dark mode, a pale
    /// blue-gray in light mode.
    pub tint: u32,
    pub tint_alpha: f32,
    pub text: u32,
    /// Secondary text, such as the version number.
    pub subtle: u32,
    /// The selected item: Grid's default colors (#60CDFF dark, #0078D4 light).
    pub selected: u32,
    pub selected_alpha: f32,
    pub divider: u32,
    pub divider_alpha: f32,
}

pub fn colors(dark: bool) -> Colors {
    if dark {
        Colors {
            tint: 0x0F_17_2A,
            tint_alpha: 0x73 as f32 / 255.0,
            text: 0xFF_FF_FF,
            subtle: 0x9A_A4_B5,
            selected: 0x60_CD_FF,
            selected_alpha: 0x29 as f32 / 255.0,
            divider: 0xFF_FF_FF,
            divider_alpha: 0x17 as f32 / 255.0,
        }
    } else {
        Colors {
            tint: 0xEE_F3_F9,
            tint_alpha: 0x8C as f32 / 255.0,
            text: 0x0F_17_2A,
            subtle: 0x5B_66_78,
            selected: 0x00_78_D4,
            selected_alpha: 0x1F as f32 / 255.0,
            divider: 0x0F_17_2A,
            divider_alpha: 0x14 as f32 / 255.0,
        }
    }
}

/// Whether Grid's windows should be dark, following Windows' app mode for `System`.
pub fn is_dark(appearance: Appearance) -> bool {
    match appearance {
        Appearance::Dark => true,
        Appearance::Light => false,
        Appearance::System => crate::ui::system_dark(),
    }
}

/// `a` blended toward `b` by `t` (0 to 1), for 0xRRGGBB colors.
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |s: u32| {
        let (x, y) = (((a >> s) & 0xFF) as f32, ((b >> s) & 0xFF) as f32);
        ((x + (y - x) * t + 0.5) as u32) << s
    };
    ch(16) | ch(8) | ch(0)
}

/// `color` (0xRRGGBB) at `alpha` (0 to 1), premultiplied, as 0xAARRGGBB.
pub fn premultiply(color: u32, alpha: f32) -> u32 {
    let ch = |s: u32| ((((color >> s) & 0xFF) as f32 * alpha + 0.5) as u32) << s;
    ((alpha * 255.0 + 0.5) as u32) << 24 | ch(16) | ch(8) | ch(0)
}

/// Premultiplied `src` drawn over premultiplied `dst`.
pub fn over(src: u32, dst: u32) -> u32 {
    let inv = 255 - (src >> 24);
    let ch = |s: u32| (((src >> s) & 0xFF) + ((dst >> s) & 0xFF) * inv / 255).min(255) << s;
    ch(24) | ch(16) | ch(8) | ch(0)
}

/// How much of pixel (`x`, `y`) is inside a `w`×`h` rectangle at (0, 0) with rounded corners.
pub fn corner_cover(x: i32, y: i32, w: i32, h: i32, radius: f32) -> f32 {
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let cx = px.clamp(radius, w as f32 - radius);
    let cy = py.clamp(radius, h as f32 - radius);
    let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() - radius;
    (0.5 - d).clamp(0.0, 1.0)
}

/// Fills a rounded rectangle with `color` at `alpha`, over premultiplied `pixels` (`w` wide).
pub fn fill_rounded(pixels: &mut [u32], w: i32, r: RECT, radius: f32, color: u32, alpha: f32) {
    let (rw, rh) = (r.right - r.left, r.bottom - r.top);
    for y in r.top.max(0)..r.bottom.min(pixels.len() as i32 / w) {
        for x in r.left.max(0)..r.right.min(w) {
            let cover = corner_cover(x - r.left, y - r.top, rw, rh, radius);
            if cover > 0.0 {
                let p = &mut pixels[(y * w + x) as usize];
                *p = over(premultiply(color, alpha * cover), *p);
            }
        }
    }
}

thread_local! {
    /// swash's working state for laying out and drawing text, kept between calls: setting it up
    /// allocates, and the tray menu draws its items again on every hover change.
    static SHAPE: std::cell::RefCell<swash::shape::ShapeContext> = std::cell::RefCell::new(swash::shape::ShapeContext::new());
    static SCALE: std::cell::RefCell<swash::scale::ScaleContext> = std::cell::RefCell::new(swash::scale::ScaleContext::new());
}

/// The glyphs of `text` with their x positions (kerned), and the width of the whole line.
fn shape(text: &str, size: f32, weight: f32) -> (Vec<(swash::GlyphId, f32)>, f32) {
    let mut glyphs = Vec::new();
    let mut pen = 0.0;
    let Some(font) = swash::FontRef::from_index(FONT, 0) else { return (glyphs, pen) };
    SHAPE.with_borrow_mut(|context| {
        let mut shaper = context.builder(font).size(size).variations(&[("wght", weight)]).build();
        shaper.add_str(text);
        shaper.shape_with(|cluster| {
            for g in cluster.glyphs {
                glyphs.push((g.id, pen + g.x));
                pen += g.advance;
            }
        });
    });
    (glyphs, pen)
}

/// How wide `text` is, in pixels, as `draw_text` draws it.
pub fn text_width(text: &str, size: f32, weight: f32) -> f32 {
    shape(text, size, weight).1
}

/// How text is placed horizontally by `draw_text`.
#[derive(Clone, Copy, PartialEq)]
pub enum Align {
    /// `x` is the left edge.
    Left,
    /// `x` is the center.
    Center,
}

/// Draws one line of `text` in Grid's font over premultiplied `pixels` (`w` wide), vertically
/// centered on `center_y`. `size` is in pixels, `weight` like CSS (400 regular, 700 bold),
/// `color` 0xRRGGBB at `alpha` (0 to 1). Uses swash, the library Slint draws the settings
/// window's text with, so both look the same (GDI renders this font's small sizes poorly).
#[allow(clippy::too_many_arguments)]
pub fn draw_text(pixels: &mut [u32], w: i32, text: &str, x: f32, center_y: f32, align: Align, size: f32, weight: f32, color: u32, alpha: f32) {
    use swash::FontRef;
    use swash::scale::{Render, Source};
    use swash::zeno::{Format, Vector};

    let Some(font) = FontRef::from_index(FONT, 0) else { return };
    let variations = [("wght", weight)];
    let h = pixels.len() as i32 / w;
    let (glyphs, pen) = shape(text, size, weight);
    let left = if align == Align::Center { x - pen / 2.0 } else { x };
    let metrics = font.metrics(&[]).scale(size);
    let baseline = (center_y + (metrics.ascent - metrics.descent) / 2.0).round() as i32;

    SCALE.with_borrow_mut(|scale| {
        let mut scaler = scale.builder(font).size(size).hint(false).variations(&variations).build();
        for (id, gx) in glyphs {
            let gx = left + gx;
            let Some(image) = Render::new(&[Source::Outline])
                .format(Format::Alpha)
                .offset(Vector::new(gx.fract(), 0.0))
                .render(&mut scaler, id)
            else {
                continue;
            };
            let p = image.placement;
            let (x0, y0) = (gx.floor() as i32 + p.left, baseline - p.top);
            for row in 0..p.height as i32 {
                for col in 0..p.width as i32 {
                    let (px, py) = (x0 + col, y0 + row);
                    let cover = image.data[(row * p.width as i32 + col) as usize];
                    if cover > 0 && (0..w).contains(&px) && (0..h).contains(&py) {
                        let d = &mut pixels[(py * w + px) as usize];
                        *d = over(premultiply(color, alpha * cover as f32 / 255.0), *d);
                    }
                }
            }
        }
    });
}
