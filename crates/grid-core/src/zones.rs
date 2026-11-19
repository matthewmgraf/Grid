//! Zones are the snap targets: grid cells or custom rectangles, in global screen pixels.

use crate::geometry::{Grid, Point, Rect};

/// Cells of `grid` over `area`, row by row. When `transpose_portrait` is set and the area is
/// taller than it is wide, columns and rows are swapped.
pub fn grid_zones(area: Rect, grid: Grid, transpose_portrait: bool) -> Vec<Rect> {
    let grid = if transpose_portrait && area.h > area.w {
        Grid { cols: grid.rows, rows: grid.cols }
    } else {
        grid
    };
    (0..grid.rows)
        .flat_map(|r| (0..grid.cols).map(move |c| grid.cell(area, c, r)))
        .collect()
}

/// Zone under `p`, or the nearest zone when `p` is outside all of them (e.g. over the taskbar).
pub fn zone_at(zones: &[Rect], p: Point) -> Option<usize> {
    zones
        .iter()
        .position(|z| z.contains(p))
        .or_else(|| (0..zones.len()).min_by_key(|&i| zones[i].distance_sq(p)))
}

/// Rectangle covered by a selection from zone `a` to zone `b`.
///
/// Starts with the bounding box of both zones, then grows to take in any zone that is at
/// least half covered by it, until nothing changes. This lets a selection from a tall zone on
/// one monitor pick up all the shorter zones it lines up with on another monitor.
///
/// Each edge then goes to the nearest grid line of the cells lined up with zone `b` (the one
/// under the mouse): its column for the top and bottom, its row for the left and right. So when
/// the start cell is on a monitor whose grid lines fall elsewhere (different size or scaling),
/// the window still fits the cell under the mouse; an edge that's on the start monitor keeps
/// that monitor's line, the nearest one. Within one grid nothing moves.
pub fn select_span(zones: &[Rect], a: usize, b: usize) -> Rect {
    let mut span = zones[a].union(&zones[b]);
    loop {
        let grown = zones
            .iter()
            .filter(|z| z.overlap_area(&span) * 2 >= z.area())
            .fold(span, |acc, z| acc.union(z));
        if grown == span {
            break;
        }
        span = grown;
    }
    let end = zones[b];
    // The zones in `end`'s row (for left and right edges) and column (for top and bottom).
    let row: Vec<&Rect> = zones.iter().filter(|z| z.y < end.bottom() && end.y < z.bottom()).collect();
    let column: Vec<&Rect> = zones.iter().filter(|z| z.x < end.right() && end.x < z.right()).collect();
    let nearest = |lines: &mut dyn Iterator<Item = i32>, v: i32| lines.min_by_key(|l| (l - v).abs()).unwrap_or(v);
    let left = nearest(&mut row.iter().map(|z| z.x), span.x);
    let right = nearest(&mut row.iter().map(|z| z.right()), span.right());
    let top = nearest(&mut column.iter().map(|z| z.y), span.y);
    let bottom = nearest(&mut column.iter().map(|z| z.bottom()), span.bottom());
    // Never smaller than the cell under the mouse.
    Rect::new(left, top, right - left, bottom - top).union(&end)
}

/// `window` fitted to the grid: each edge moves to the nearest zone edge, so the window fills
/// the cells it mostly covers, and it always covers at least one cell. Only zones the window
/// overlaps (or the nearest one) are used, so lines on other monitors don't interfere.
pub fn snap_to_grid(window: Rect, zones: &[Rect]) -> Rect {
    let mut near: Vec<&Rect> = zones.iter().filter(|z| z.overlap_area(&window) > 0).collect();
    if near.is_empty() {
        let center = Point::new(window.x + window.w / 2, window.y + window.h / 2);
        near.extend(zone_at(zones, center).map(|i| &zones[i]));
    }
    if near.is_empty() {
        return window;
    }
    let lines = |edges: fn(&Rect) -> [i32; 2]| {
        let mut v: Vec<i32> = near.iter().flat_map(|z| edges(z)).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let (x0, x1) = snap_span(window.x, window.right(), &lines(|z| [z.x, z.right()]));
    let (y0, y1) = snap_span(window.y, window.bottom(), &lines(|z| [z.y, z.bottom()]));
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

/// Where a `w`×`h` window goes when its app won't shrink it to `target` (e.g. it has a minimum
/// size): top-left on the target's, shifted back inside `area` (its monitor's work area) where
/// it would stick out.
pub fn anchor_in(target: Rect, w: i32, h: i32, area: Rect) -> Rect {
    let x = target.x.min(area.right() - w).max(area.x);
    let y = target.y.min(area.bottom() - h).max(area.y);
    Rect::new(x, y, w, h)
}

/// The area from `areas` that overlaps `r` the most, or `r` itself if none does.
pub fn area_for(areas: &[Rect], r: Rect) -> Rect {
    areas
        .iter()
        .copied()
        .filter(|a| a.overlap_area(&r) > 0)
        .max_by_key(|a| a.overlap_area(&r))
        .unwrap_or(r)
}

/// The bounding box of every area in `areas` that `r` overlaps, so a window can straddle
/// neighboring monitors; `None` if it overlaps none.
pub fn reach(areas: &[Rect], r: Rect) -> Option<Rect> {
    areas.iter().filter(|a| a.overlap_area(&r) > 0).copied().reduce(|acc, a| acc.union(&a))
}

/// `target` shrunk to leave gaps: `edges` pixels where it touches the edge of a monitor's work
/// area, and half of `between` elsewhere, so two neighboring windows end up `between` apart.
/// Never shrinks below 1×1.
pub fn apply_gaps(target: Rect, areas: &[Rect], between: i32, edges: i32) -> Rect {
    if between <= 0 && edges <= 0 {
        return target;
    }
    let (near, far) = (between / 2, between - between / 2);
    let on_edge = |edge: fn(&Rect) -> i32| areas.iter().any(|a| edge(a) == edge(&target));
    let pick = |is_edge: bool, inner: i32| if is_edge { edges.max(0) } else { inner.max(0) };
    let left = pick(on_edge(|r| r.x), near);
    let top = pick(on_edge(|r| r.y), near);
    let right = pick(on_edge(Rect::right), far);
    let bottom = pick(on_edge(Rect::bottom), far);
    let (w, h) = ((target.w - left - right).max(1), (target.h - top - bottom).max(1));
    Rect::new(target.x + left.min(target.w - 1), target.y + top.min(target.h - 1), w, h)
}

/// Moves `lo` and `hi` to their nearest lines, keeping at least one gap between them.
/// `lines` must be sorted and non-empty.
fn snap_span(lo: i32, hi: i32, lines: &[i32]) -> (i32, i32) {
    let nearest = |v: i32| *lines.iter().min_by_key(|&&l| (l - v).abs()).expect("lines is non-empty");
    let (mut a, mut b) = (nearest(lo), nearest(hi));
    if b <= a {
        if let Some(&next) = lines.iter().find(|&&l| l > a) {
            b = next;
        } else if let Some(&prev) = lines.iter().rev().find(|&&l| l < a) {
            (a, b) = (prev, a);
        }
    }
    (a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portrait_grid_is_transposed() {
        let zones = grid_zones(Rect::new(0, 0, 1080, 1920), Grid { cols: 4, rows: 2 }, true);
        assert_eq!(zones.len(), 8);
        assert_eq!(zones[0], Rect::new(0, 0, 540, 480));
    }

    #[test]
    fn zone_at_falls_back_to_nearest() {
        let zones = grid_zones(Rect::new(0, 0, 100, 100), Grid { cols: 2, rows: 2 }, false);
        assert_eq!(zone_at(&zones, Point::new(75, 25)), Some(1));
        assert_eq!(zone_at(&zones, Point::new(75, 130)), Some(3));
    }

    // Owner's example: zone 1 is the left monitor, zones 2 and 3 split the right monitor
    // top/bottom. Dragging from 1 into 2 covers 2 and 3 as well.
    #[test]
    fn span_across_monitors_takes_matching_zones() {
        let zones = [
            Rect::new(0, 0, 1920, 1080),
            Rect::new(1920, 0, 1920, 540),
            Rect::new(1920, 540, 1920, 540),
        ];
        assert_eq!(select_span(&zones, 0, 1), Rect::new(0, 0, 3840, 1080));
    }

    #[test]
    fn span_across_monitors_of_different_height() {
        let zones = [
            Rect::new(0, 0, 1920, 1080),
            Rect::new(1920, 0, 2560, 720),
            Rect::new(1920, 720, 2560, 720),
        ];
        assert_eq!(select_span(&zones, 0, 1), Rect::new(0, 0, 4480, 1440));
    }

    #[test]
    fn misaligned_grids_do_not_cascade() {
        let mut zones = grid_zones(Rect::new(0, 0, 1920, 1080), Grid { cols: 8, rows: 8 }, false);
        let b = zones.len();
        zones.extend(grid_zones(Rect::new(1920, 0, 2560, 1440), Grid { cols: 8, rows: 8 }, false));
        // Top-right cell of the left monitor to top-left cell of the right monitor.
        let span = select_span(&zones, 7, b);
        assert_eq!(span, Rect::new(1680, 0, 560, 180));
    }

    #[test]
    fn span_ends_on_the_grid_under_the_mouse() {
        // Side by side, with rows that don't line up: 1920×1080 at 6 rows (180 px) and, to its
        // right, 1920×1080 at 7 rows (about 154 px). From the left monitor's 4th row (540–720) to
        // the right monitor's 5th row (617–771): the window's top and bottom follow the right
        // monitor's lines (the cell under the mouse), not the left monitor's 720.
        let mut zones = grid_zones(Rect::new(0, 0, 1920, 1080), Grid { cols: 4, rows: 6 }, false);
        let b = zones.len();
        zones.extend(grid_zones(Rect::new(1920, 0, 1920, 1080), Grid { cols: 4, rows: 7 }, false));
        let end = b + 4 * 4; // right monitor, row 5 (index 4), first column
        let span = select_span(&zones, 3 * 4 + 3, end); // left monitor, row 4 (index 3), last column
        assert_eq!((span.bottom(), span.right()), (zones[end].bottom(), zones[end].right()));
        assert_eq!(span.x, zones[3 * 4 + 3].x);
        assert!(span.y == zones[end].y || zones.iter().any(|z| z.y == span.y && z.x >= 1920));
    }

    #[test]
    fn span_within_grid_is_exact() {
        let zones = grid_zones(Rect::new(0, 0, 800, 800), Grid { cols: 8, rows: 8 }, false);
        assert_eq!(select_span(&zones, 9, 18), Rect::new(100, 100, 200, 200));
    }

    fn grid_800() -> Vec<Rect> {
        grid_zones(Rect::new(0, 0, 800, 800), Grid { cols: 8, rows: 8 }, false)
    }

    #[test]
    fn oversized_window_anchors_to_target_and_stays_on_screen() {
        let area = Rect::new(0, 0, 1920, 1080);
        // Fits from the target's corner.
        assert_eq!(anchor_in(Rect::new(240, 135, 240, 135), 960, 800, area), Rect::new(240, 135, 960, 800));
        // Would run off the bottom-right: shifted back inside.
        assert_eq!(anchor_in(Rect::new(1680, 945, 240, 135), 960, 800, area), Rect::new(960, 280, 960, 800));
    }

    #[test]
    fn area_for_picks_most_overlapping_monitor() {
        let areas = [Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 1920, 1080)];
        assert_eq!(area_for(&areas, Rect::new(1800, 0, 400, 100)), areas[1]);
    }

    #[test]
    fn gaps_use_edge_gap_at_screen_edges_and_half_gap_between() {
        let areas = [Rect::new(0, 0, 800, 800)];
        // Left half: screen edges on the left, top and bottom; a neighbor on the right.
        assert_eq!(apply_gaps(Rect::new(0, 0, 400, 800), &areas, 10, 4), Rect::new(4, 4, 391, 792));
        // Right half: neighbor on the left.
        assert_eq!(apply_gaps(Rect::new(400, 0, 400, 800), &areas, 10, 4), Rect::new(405, 4, 391, 792));
        // No gaps: unchanged.
        assert_eq!(apply_gaps(Rect::new(100, 100, 200, 200), &areas, 0, 0), Rect::new(100, 100, 200, 200));
    }

    #[test]
    fn small_window_fills_its_cell() {
        assert_eq!(snap_to_grid(Rect::new(140, 230, 30, 40), &grid_800()), Rect::new(100, 200, 100, 100));
    }

    #[test]
    fn window_snaps_to_cells_it_mostly_covers() {
        // Spans 2.2 columns and 1.6 rows.
        assert_eq!(snap_to_grid(Rect::new(108, 191, 220, 160), &grid_800()), Rect::new(100, 200, 200, 200));
    }

    #[test]
    fn tiny_window_at_far_edge_still_covers_a_cell() {
        assert_eq!(snap_to_grid(Rect::new(790, 795, 10, 5), &grid_800()), Rect::new(700, 700, 100, 100));
    }

    #[test]
    fn snap_ignores_lines_on_other_monitors() {
        let mut zones = grid_800();
        zones.extend(grid_zones(Rect::new(800, 0, 900, 900), Grid { cols: 3, rows: 3 }, false));
        assert_eq!(snap_to_grid(Rect::new(110, 110, 80, 80), &zones), Rect::new(100, 100, 100, 100));
    }
}
