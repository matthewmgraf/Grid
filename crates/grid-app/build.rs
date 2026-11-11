//! Embeds Grid's logo as the exe's icon (taskbar, Explorer, Start menu, Alt+Tab). The .ico is
//! generated from `grid_core::logo`, so the logo is still defined in one place.

use std::path::Path;

/// Icon sizes Windows picks from for different places and display scalings.
const SIZES: [i32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

fn main() {
    println!("cargo:rerun-if-changed=../grid-core/src/logo.rs");
    // The version lives in the workspace Cargo.toml.
    println!("cargo:rerun-if-changed=../../Cargo.toml");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = std::env::var("OUT_DIR").expect("OUT_DIR is set by Cargo");
    let ico = Path::new(&out).join("grid.ico");
    std::fs::write(&ico, ico_file()).expect("write grid.ico");
    let rc = Path::new(&out).join("grid.rc");
    // Resource ID 1 is what the app loads for window icons (`grid_windows::use_app_icon`).
    let mut rc_text = format!("1 ICON \"{}\"\n", ico.display().to_string().replace('\\', "\\\\"));
    rc_text.push_str(&version_info());
    std::fs::write(&rc, rc_text).expect("write grid.rc");
    embed_resource::compile(&rc, embed_resource::NONE).manifest_required().expect("embed the icon");
}

/// The exe's version information: the name Windows shows for it (Task Manager's processes and
/// startup apps, file properties) and its version.
fn version_info() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let mut parts: Vec<u32> = version.split(['.', '-']).take(3).map(|p| p.parse().unwrap_or(0)).collect();
    parts.resize(4, 0);
    let numbers = parts.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    format!(
        r#"
1 VERSIONINFO
FILEVERSION {numbers}
PRODUCTVERSION {numbers}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "Grid"
      VALUE "ProductName", "Grid"
      VALUE "FileVersion", "{version}"
      VALUE "ProductVersion", "{version}"
      VALUE "OriginalFilename", "grid.exe"
      VALUE "LegalCopyright", "GPL-3.0-or-later"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    )
}

/// An .ico with a 32-bit image per size.
fn ico_file() -> Vec<u8> {
    let images: Vec<Vec<u8>> = SIZES.iter().map(|&s| ico_image(s)).collect();
    let mut file = Vec::new();
    file.extend([0u16, 1, SIZES.len() as u16].iter().flat_map(|v| v.to_le_bytes()));
    let mut offset = 6 + 16 * SIZES.len() as u32;
    for (&size, image) in SIZES.iter().zip(&images) {
        let side = if size >= 256 { 0 } else { size as u8 };
        file.extend([side, side, 0, 0]);
        file.extend(1u16.to_le_bytes());
        file.extend(32u16.to_le_bytes());
        file.extend((image.len() as u32).to_le_bytes());
        file.extend(offset.to_le_bytes());
        offset += image.len() as u32;
    }
    images.iter().for_each(|image| file.extend(image));
    file
}

/// One icon image: a bitmap header (double height: colour + mask), bottom-up BGRA pixels with
/// straight alpha, then an all-zero mask (the alpha channel is used instead).
fn ico_image(size: i32) -> Vec<u8> {
    let pixels = grid_core::logo::draw(size);
    let mask_row = ((size + 31) / 32 * 4) as usize;
    let mut image = Vec::new();
    for v in [40u32, size as u32, (size * 2) as u32] {
        image.extend(v.to_le_bytes());
    }
    image.extend(1u16.to_le_bytes());
    image.extend(32u16.to_le_bytes());
    for v in [0u32, 0, 0, 0, 0, 0] {
        image.extend(v.to_le_bytes());
    }
    for y in (0..size).rev() {
        for x in 0..size {
            let p = pixels[(y * size + x) as usize];
            let a = p >> 24;
            // Undo the premultiplication.
            let ch = |shift: u32| (((p >> shift) & 0xFF) * 255).checked_div(a).map_or(0, |v| v.min(255) as u8);
            image.extend([ch(0), ch(8), ch(16), a as u8]);
        }
    }
    image.resize(image.len() + mask_row * size as usize, 0);
    image
}
