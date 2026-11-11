//! State of one window drag while grid mode is in use. The platform layer feeds it input and
//! reads back what to draw and where the window should be.

use serde::{Deserialize, Serialize};

use crate::geometry::{Point, Rect};
use crate::zones::{anchor_in, area_for, reach, select_span, snap_to_grid, zone_at};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    /// Hold to pick the start zone, release, then move to pick the end zone.
    Resize,
    /// Snap the window to the cells it mostly covers, then move it a whole cell at a time with
    /// the cursor, keeping that size.
    Move,
    /// Put the window back where it was before the drag and leave grid mode.
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Off,
    Resize { start: usize, end: usize, held: bool },
    /// `size` is the window's snapped size. `offset` is where the cursor's cell sits inside it,
    /// so the window always covers the cell under the cursor at the same spot.
    Move { offset: Point, size: (i32, i32) },
}

#[derive(Debug, Clone)]
pub struct Session {
    zones: Vec<Rect>,
    /// Monitor work areas; Move keeps the window inside them.
    areas: Vec<Rect>,
    cursor: Point,
    window: Rect,
    mode: Mode,
}

impl Session {
    pub fn new(zones: Vec<Rect>, areas: Vec<Rect>, cursor: Point, window: Rect) -> Self {
        Self { zones, areas, cursor, window, mode: Mode::Off }
    }

    pub fn zones(&self) -> &[Rect] {
        &self.zones
    }

    pub fn is_active(&self) -> bool {
        self.mode != Mode::Off
    }

    pub fn press(&mut self, command: Command) {
        match command {
            Command::Resize => {
                if let Some(z) = zone_at(&self.zones, self.cursor) {
                    self.mode = Mode::Resize { start: z, end: z, held: true };
                }
            }
            Command::Move => {
                let snapped = snap_to_grid(self.target().unwrap_or(self.window), &self.zones);
                if let Some(z) = zone_at(&self.zones, self.cursor) {
                    let cell = self.zones[z];
                    // Snapping can push an edge past the cursor (grabbing the title bar just
                    // above a grid line); shift the window so it still covers the cursor's cell.
                    let offset = Point::new(
                        (cell.x - snapped.x).clamp(0, (snapped.w - cell.w).max(0)),
                        (cell.y - snapped.y).clamp(0, (snapped.h - cell.h).max(0)),
                    );
                    self.mode = Mode::Move { offset, size: (snapped.w, snapped.h) };
                }
            }
            Command::Cancel => self.cancel(),
        }
    }

    pub fn release(&mut self, command: Command) {
        if let (Command::Resize, Mode::Resize { held, .. }) = (command, &mut self.mode) {
            *held = false;
        }
    }

    /// Leave grid mode.
    pub fn cancel(&mut self) {
        self.mode = Mode::Off;
    }

    /// The window's actual position, while the OS is still moving it (before grid mode).
    pub fn set_window(&mut self, window: Rect) {
        self.window = window;
    }

    pub fn update(&mut self, cursor: Point) {
        self.cursor = cursor;
        let hovered = zone_at(&self.zones, cursor);
        if let (Mode::Resize { start, end, held }, Some(z)) = (&mut self.mode, hovered) {
            *end = z;
            if *held {
                *start = z;
            }
        }
    }

    /// Zone highlighted as the selection's starting point.
    pub fn start_zone(&self) -> Option<Rect> {
        match self.mode {
            Mode::Resize { start, .. } => Some(self.zones[start]),
            _ => None,
        }
    }

    /// Where the window should be right now.
    pub fn target(&self) -> Option<Rect> {
        match self.mode {
            Mode::Off => None,
            Mode::Resize { start, end, .. } => Some(select_span(&self.zones, start, end)),
            Mode::Move { offset, size: (w, h) } => {
                let cell = self.zones[zone_at(&self.zones, self.cursor)?];
                let placed = Rect::new(cell.x - offset.x, cell.y - offset.y, w, h);
                // Keep it on screen: within the monitors it overlaps, so it can straddle two of
                // them. Then fit to the grid lines there (a no-op on the grid it was sized on).
                let bounds = reach(&self.areas, placed).unwrap_or_else(|| area_for(&self.areas, cell));
                let on_screen = anchor_in(placed, w, h, bounds);
                Some(snap_to_grid(on_screen, &self.zones))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Grid;
    use crate::zones::grid_zones;

    fn session() -> Session {
        let zones = grid_zones(Rect::new(0, 0, 400, 400), Grid { cols: 4, rows: 4 }, false);
        Session::new(zones, vec![Rect::new(0, 0, 400, 400)], Point::new(50, 50), Rect::new(10, 10, 150, 150))
    }

    #[test]
    fn resize_start_follows_cursor_until_released() {
        let mut s = session();
        s.press(Command::Resize);
        s.update(Point::new(150, 150));
        assert_eq!(s.start_zone(), Some(Rect::new(100, 100, 100, 100)));
        s.release(Command::Resize);
        s.update(Point::new(350, 250));
        assert_eq!(s.start_zone(), Some(Rect::new(100, 100, 100, 100)));
        assert_eq!(s.target(), Some(Rect::new(100, 100, 300, 200)));
    }

    #[test]
    fn move_snaps_at_once_and_keeps_size() {
        let mut s = session();
        s.press(Command::Move);
        // Window (10,10,150,150) snaps to 2×2 cells right away.
        assert_eq!(s.target(), Some(Rect::new(0, 0, 200, 200)));
        // Anywhere inside the next cell gives the same place and size.
        s.update(Point::new(160, 60));
        assert_eq!(s.target(), Some(Rect::new(100, 0, 200, 200)));
        s.update(Point::new(195, 95));
        assert_eq!(s.target(), Some(Rect::new(100, 0, 200, 200)));
    }

    #[test]
    fn move_keeps_cursor_cell_and_stays_on_screen() {
        let zones = grid_zones(Rect::new(0, 0, 400, 400), Grid { cols: 4, rows: 4 }, false);
        let area = Rect::new(0, 0, 400, 400);
        // Grabbed in the bottom-right cell of a 2×2 window.
        let mut s = Session::new(zones, vec![area], Point::new(150, 150), Rect::new(0, 0, 200, 200));
        s.press(Command::Move);
        s.update(Point::new(250, 250));
        assert_eq!(s.target(), Some(Rect::new(100, 100, 200, 200)));
        // Near the top edge the window can't keep that spot without leaving the screen.
        s.update(Point::new(350, 50));
        assert_eq!(s.target(), Some(Rect::new(200, 0, 200, 200)));
    }

    #[test]
    fn move_covers_cursor_when_snap_pushes_edge_past_it() {
        let zones = grid_zones(Rect::new(0, 0, 400, 400), Grid { cols: 4, rows: 4 }, false);
        // Title bar grabbed at y=165; the window's top (160) snaps down to 200, past the cursor.
        let mut s = Session::new(zones, vec![Rect::new(0, 0, 400, 400)], Point::new(50, 165), Rect::new(10, 160, 150, 150));
        s.press(Command::Move);
        assert_eq!(s.target(), Some(Rect::new(0, 100, 200, 100)));
        s.update(Point::new(150, 265));
        assert_eq!(s.target(), Some(Rect::new(100, 200, 200, 100)));
    }

    #[test]
    fn move_can_straddle_two_monitors() {
        let (left, right) = (Rect::new(0, 0, 400, 400), Rect::new(400, 0, 400, 400));
        let mut zones = grid_zones(left, Grid { cols: 4, rows: 4 }, false);
        zones.extend(grid_zones(right, Grid { cols: 4, rows: 4 }, false));
        // A 2×2 window grabbed in its right column.
        let mut s = Session::new(zones, vec![left, right], Point::new(150, 50), Rect::new(0, 0, 200, 200));
        s.press(Command::Move);
        // Cursor onto the right monitor's first cell: half the window stays on the left one.
        s.update(Point::new(450, 50));
        assert_eq!(s.target(), Some(Rect::new(300, 0, 200, 200)));
    }

    #[test]
    fn move_after_resize_starts_from_resized_window() {
        let mut s = session();
        s.press(Command::Resize);
        s.release(Command::Resize);
        s.press(Command::Move);
        s.update(Point::new(150, 150));
        assert_eq!(s.target(), Some(Rect::new(100, 100, 100, 100)));
    }

    #[test]
    fn cancel_clears_target() {
        let mut s = session();
        s.press(Command::Resize);
        s.cancel();
        assert!(!s.is_active());
        assert_eq!(s.target(), None);
    }
}
