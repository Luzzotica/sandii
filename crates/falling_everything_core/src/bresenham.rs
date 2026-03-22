//! Integer [Bresenham line algorithm](https://en.wikipedia.org/wiki/Bresenham%27s_line_algorithm).
//! No floating-point math — suitable for large-scale particle / grid simulations.

use crate::world::Vec2i;

/// Integer square root (floor), for `n >= 0`. Used e.g. for circle chord half-widths without floats.
#[inline]
pub fn isqrt_i32(n: i32) -> i32 {
    if n <= 0 {
        return 0;
    }
    let n = n as u32;
    let mut x = n;
    let mut y = (x + 1) >> 1;
    while y < x {
        x = y;
        y = (x + n / x) >> 1;
    }
    x as i32
}

/// All grid cells on the line from `a` to `b` inclusive, in order from `a` toward `b`.
/// Allocation-free: use in `for` loops or with `for_each`.
///
/// # Example
/// ```
/// use falling_everything_core::bresenham::bresenham_line;
/// use falling_everything_core::world::Vec2i;
/// let cells: Vec<_> = bresenham_line(Vec2i::new(0, 0), Vec2i::new(3, 1)).collect();
/// assert!(cells.len() >= 4);
/// ```
pub fn bresenham_line(a: Vec2i, b: Vec2i) -> BresenhamLine {
    BresenhamLine::new(a.x, a.y, b.x, b.y)
}

/// Visit every cell on the line from `a` to `b` inclusive.
pub fn for_each_bresenham_line<F>(a: Vec2i, b: Vec2i, mut f: F)
where
    F: FnMut(Vec2i),
{
    for p in bresenham_line(a, b) {
        f(p);
    }
}

/// Iterator over line points. Uses the integer generalization (works for all octants).
pub struct BresenhamLine {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    dx: i32,
    dy: i32,
    sx: i32,
    sy: i32,
    err: i32,
    finished: bool,
}

impl BresenhamLine {
    fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let err = dx + dy;
        Self {
            x0,
            y0,
            x1,
            y1,
            dx,
            dy,
            sx,
            sy,
            err,
            finished: false,
        }
    }
}

impl Iterator for BresenhamLine {
    type Item = Vec2i;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        let out = Vec2i::new(self.x0, self.y0);
        if self.x0 == self.x1 && self.y0 == self.y1 {
            self.finished = true;
            return Some(out);
        }
        let e2 = self.err.saturating_mul(2);
        if e2 >= self.dy {
            self.err += self.dy;
            self.x0 += self.sx;
        }
        if e2 <= self.dx {
            self.err += self.dx;
            self.y0 += self.sy;
        }
        Some(out)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.finished {
            return (0, Some(0));
        }
        let manhattan = (self.x1 - self.x0).abs() as usize + (self.y1 - self.y0).abs() as usize;
        (1, Some(manhattan + 1))
    }
}

impl std::iter::FusedIterator for BresenhamLine {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_and_vertical() {
        let h: Vec<_> = bresenham_line(Vec2i::new(0, 0), Vec2i::new(3, 0)).collect();
        assert_eq!(
            h,
            vec![
                Vec2i::new(0, 0),
                Vec2i::new(1, 0),
                Vec2i::new(2, 0),
                Vec2i::new(3, 0),
            ]
        );
        let v: Vec<_> = bresenham_line(Vec2i::new(2, 1), Vec2i::new(2, 4)).collect();
        assert_eq!(
            v,
            vec![
                Vec2i::new(2, 1),
                Vec2i::new(2, 2),
                Vec2i::new(2, 3),
                Vec2i::new(2, 4),
            ]
        );
    }

    #[test]
    fn diagonal() {
        let d: Vec<_> = bresenham_line(Vec2i::new(0, 0), Vec2i::new(3, 3)).collect();
        assert_eq!(d.len(), 4);
        for (i, p) in d.iter().enumerate() {
            assert_eq!(p.x, i as i32);
            assert_eq!(p.y, i as i32);
        }
    }

    #[test]
    fn single_point() {
        let p = Vec2i::new(5, -3);
        let pts: Vec<_> = bresenham_line(p, p).collect();
        assert_eq!(pts, vec![p]);
    }

    #[test]
    fn steep_negative() {
        let pts: Vec<_> = bresenham_line(Vec2i::new(0, 0), Vec2i::new(2, 5)).collect();
        assert_eq!(pts.first(), Some(&Vec2i::new(0, 0)));
        assert_eq!(pts.last(), Some(&Vec2i::new(2, 5)));
        assert!(pts.len() >= 6);
    }

    #[test]
    fn isqrt_matches_floor_sqrt() {
        assert_eq!(isqrt_i32(0), 0);
        assert_eq!(isqrt_i32(1), 1);
        assert_eq!(isqrt_i32(2), 1);
        assert_eq!(isqrt_i32(3), 1);
        assert_eq!(isqrt_i32(4), 2);
        assert_eq!(isqrt_i32(15), 3);
        assert_eq!(isqrt_i32(16), 4);
        assert_eq!(isqrt_i32(100), 10);
    }
}
