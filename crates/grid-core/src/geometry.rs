use serde::{Deserialize, Serialize};

/// A point in physical (device) pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// A rectangle in physical (device) pixels. `x`/`y` is the top-left corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub const fn right(&self) -> i32 {
        self.x + self.w
    }

    pub const fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub const fn area(&self) -> i64 {
        self.w as i64 * self.h as i64
    }

    pub const fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    /// Overlapping area (0 when the rectangles only touch or are apart).
    pub fn overlap_area(&self, other: &Rect) -> i64 {
        let w = self.right().min(other.right()) - self.x.max(other.x);
        let h = self.bottom().min(other.bottom()) - self.y.max(other.y);
        if w > 0 && h > 0 { w as i64 * h as i64 } else { 0 }
    }

    /// Squared distance from `p` to the nearest point of the rectangle (0 if inside).
    pub fn distance_sq(&self, p: Point) -> i64 {
        let dx = (self.x - p.x).max(p.x - (self.right() - 1)).max(0) as i64;
        let dy = (self.y - p.y).max(p.y - (self.bottom() - 1)).max(0) as i64;
        dx * dx + dy * dy
    }

    /// Smallest rectangle containing both `self` and `other`.
    pub fn union(&self, other: &Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect::new(x, y, self.right().max(other.right()) - x, self.bottom().max(other.bottom()) - y)
    }
}

/// A uniform grid of `cols` x `rows` cells laid over an area (usually a monitor's work area).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grid {
    pub cols: u32,
    pub rows: u32,
}

impl Grid {
    /// Rectangle of the cell at (`col`, `row`). Cell edges are computed from the area's
    /// edges, so cells tile the area exactly with no rounding gaps.
    pub fn cell(&self, area: Rect, col: u32, row: u32) -> Rect {
        let edge = |start: i32, len: i32, i: u32, n: u32| start + (len as i64 * i as i64 / n as i64) as i32;
        let x0 = edge(area.x, area.w, col, self.cols);
        let x1 = edge(area.x, area.w, col + 1, self.cols);
        let y0 = edge(area.y, area.h, row, self.rows);
        let y1 = edge(area.y, area.h, row + 1, self.rows);
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_tile_area_without_gaps() {
        let area = Rect::new(0, 0, 1921, 1079);
        let grid = Grid { cols: 8, rows: 8 };
        for c in 0..grid.cols - 1 {
            assert_eq!(grid.cell(area, c, 0).right(), grid.cell(area, c + 1, 0).x);
        }
        assert_eq!(grid.cell(area, 7, 7).right(), area.right());
        assert_eq!(grid.cell(area, 7, 7).bottom(), area.bottom());
    }

    #[test]
    fn overlap_and_distance() {
        let a = Rect::new(0, 0, 10, 10);
        assert_eq!(a.overlap_area(&Rect::new(5, 5, 10, 10)), 25);
        assert_eq!(a.overlap_area(&Rect::new(10, 0, 10, 10)), 0);
        assert_eq!(a.distance_sq(Point::new(5, 5)), 0);
        assert_eq!(a.distance_sq(Point::new(12, 9)), 9);
    }
}
