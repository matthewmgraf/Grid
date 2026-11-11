//! Grid's logo. This is the one place it is drawn (exe icon, tray and title-bar icon, splash
//! screen and About tab all use it), so replacing it only means changing `draw`. It lives in
//! `grid-core` because it is plain drawing code, shared by the engine and the settings window.
//!
//! One frosted 16:9 cell with rounded corners on a dark rounded square, like the overlay with
//! blur on at full opacity.

/// Cell width : height.
const CELL_W: f32 = 16.0;
const CELL_H: f32 = 9.0;
/// Margin around the cell, in the same units.
const GAP: f32 = 2.6;

const BACKGROUND: (f32, f32, f32) = (0x1A as f32, 0x1E as f32, 0x2A as f32);
const FILL_TOP: (f32, f32, f32) = (0x30 as f32, 0x34 as f32, 0x3F as f32);
const FILL_BOTTOM: (f32, f32, f32) = (0x26 as f32, 0x2B as f32, 0x37 as f32);
/// Light blue outline all the way around each cell.
const BORDER: (f32, f32, f32) = (0x3F as f32, 0xA9 as f32, 0xE0 as f32);

/// The logo as `size`×`size` premultiplied pixels (`0xAARRGGBB`, row by row, top first).
pub fn draw(size: i32) -> Vec<u32> {
    let s = size as f32;
    // The cell spans the width, less the margins, and sits in the middle vertically.
    let unit = s / (CELL_W + 2.0 * GAP);
    let (cw, ch) = (CELL_W * unit, CELL_H * unit);
    let (cx, cy) = (GAP * unit, (s - ch) / 2.0);
    let cell_radius = (ch * 0.12).max(1.0);
    let border = (s / 128.0).max(1.0);

    let mut pixels = vec![0u32; (size * size) as usize];
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            // Background square.
            let bg = coverage(rounded_distance(px, py, 0.0, 0.0, s, s, s * 0.18));
            if bg <= 0.0 {
                continue;
            }
            let mut color = BACKGROUND;
            let d = rounded_distance(px, py, cx, cy, cw, ch, cell_radius);
            let inside = coverage(d);
            if inside > 0.0 {
                let t = ((py - cy) / ch).clamp(0.0, 1.0);
                let fill = mix(FILL_TOP, FILL_BOTTOM, t);
                let edge = inside - coverage(d + border);
                color = mix(color, fill, inside);
                color = mix(color, BORDER, edge);
            }
            let a = bg;
            let ch = |v: f32| ((v * a).round() as u32).min(255);
            pixels[(y * size + x) as usize] =
                ((a * 255.0).round() as u32) << 24 | ch(color.0) << 16 | ch(color.1) << 8 | ch(color.2);
        }
    }
    pixels
}

fn mix(a: (f32, f32, f32), b: (f32, f32, f32), t: f32) -> (f32, f32, f32) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, a.2 + (b.2 - a.2) * t)
}

/// How much of a pixel at signed distance `d` from an edge is inside.
fn coverage(d: f32) -> f32 {
    (0.5 - d).clamp(0.0, 1.0)
}

/// Signed distance from (`px`, `py`) to a rounded rectangle; negative inside.
fn rounded_distance(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, radius: f32) -> f32 {
    let r = radius.min(w / 2.0).min(h / 2.0);
    let qx = (px - (x + w / 2.0)).abs() - (w / 2.0 - r);
    let qy = (py - (y + h / 2.0)).abs() - (h / 2.0 - r);
    (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r
}

#[cfg(test)]
mod tests {
    /// Writes the logo at a few sizes, side by side, to `GRID_LOGO_PREVIEW` (a .bmp). Run with
    /// `--ignored`.
    #[test]
    #[ignore]
    fn preview() {
        let Ok(path) = std::env::var("GRID_LOGO_PREVIEW") else { return };
        // Tray sizes (100%–200% scaling), then larger ones.
        let sizes = [16, 20, 24, 32, 64, 256];
        let (w, h) = (sizes.iter().sum::<i32>() + 20 * sizes.len() as i32, 256);
        let mut canvas = vec![0xFFF0_F0F0u32; (w * h) as usize];
        let mut left = 10;
        for size in sizes {
            for (i, p) in super::draw(size).into_iter().enumerate() {
                let (x, y) = (left + i as i32 % size, i as i32 / size);
                let d = &mut canvas[(y * w + x) as usize];
                let inv = 255 - (p >> 24);
                let mix = |s: u32| (((p >> s) & 0xFF) + ((*d >> s) & 0xFF) * inv / 255).min(255) << s;
                *d = 0xFF00_0000 | mix(16) | mix(8) | mix(0);
            }
            left += size + 20;
        }
        let row = (w * 3 + 3) & !3;
        let data = (row * h) as u32;
        let mut bmp = b"BM".to_vec();
        for v in [54 + data, 0, 54, 40] {
            bmp.extend(v.to_le_bytes());
        }
        bmp.extend(w.to_le_bytes());
        bmp.extend(h.to_le_bytes());
        bmp.extend(1u16.to_le_bytes());
        bmp.extend(24u16.to_le_bytes());
        for v in [0u32, data, 2835, 2835, 0, 0] {
            bmp.extend(v.to_le_bytes());
        }
        for y in (0..h).rev() {
            for x in 0..w {
                let c = canvas[(y * w + x) as usize];
                bmp.extend([c as u8, (c >> 8) as u8, (c >> 16) as u8]);
            }
            bmp.resize(bmp.len() + (row - w * 3) as usize, 0);
        }
        std::fs::write(path, bmp).unwrap();
    }
}
