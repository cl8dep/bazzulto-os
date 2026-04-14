//! Geometric primitives — Point, Size, Rect.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Point {
        Point { x, y }
    }

    pub const ZERO: Point = Point { x: 0, y: 0 };
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Size {
    pub width:  u32,
    pub height: u32,
}

impl Size {
    pub const fn new(width: u32, height: u32) -> Size {
        Size { width, height }
    }

    pub fn area(self) -> u64 {
        self.width as u64 * self.height as u64
    }

    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// Axis-aligned rectangle.  Origin is the top-left corner.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rect {
    pub origin: Point,
    pub size:   Size,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Rect {
        Rect {
            origin: Point { x, y },
            size:   Size { width, height },
        }
    }

    pub fn x(self)      -> i32 { self.origin.x }
    pub fn y(self)      -> i32 { self.origin.y }
    pub fn width(self)  -> u32 { self.size.width }
    pub fn height(self) -> u32 { self.size.height }

    pub fn right(self)  -> i32 { self.origin.x + self.size.width  as i32 }
    pub fn bottom(self) -> i32 { self.origin.y + self.size.height as i32 }

    pub fn contains(self, point: Point) -> bool {
        point.x >= self.x()
            && point.x < self.right()
            && point.y >= self.y()
            && point.y < self.bottom()
    }

    pub fn intersection(self, other: Rect) -> Option<Rect> {
        let left   = self.x().max(other.x());
        let top    = self.y().max(other.y());
        let right  = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());

        if right > left && bottom > top {
            Some(Rect::new(left, top, (right - left) as u32, (bottom - top) as u32))
        } else {
            None
        }
    }

    pub fn is_empty(self) -> bool {
        self.size.is_empty()
    }
}
