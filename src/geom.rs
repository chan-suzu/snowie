use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V2 {
    pub x: f32,
    pub y: f32,
}

#[allow(non_snake_case)]
pub const fn V2(x: f32, y: f32) -> V2 {
    V2 { x, y }
}

impl V2 {
    pub const ZERO: V2 = V2 { x: 0.0, y: 0.0 };
    pub fn len(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
    pub fn norm(self) -> V2 {
        let l = self.len();
        if l > 1e-6 {
            self / l
        } else {
            V2::ZERO
        }
    }
    pub fn dot(self, o: V2) -> f32 {
        self.x * o.x + self.y * o.y
    }
    pub fn cross(self, o: V2) -> f32 {
        self.x * o.y - self.y * o.x
    }
    pub fn from_angle(a: f32) -> V2 {
        V2(a.cos(), a.sin())
    }
}

impl Add for V2 {
    type Output = V2;
    fn add(self, o: V2) -> V2 {
        V2(self.x + o.x, self.y + o.y)
    }
}
impl AddAssign for V2 {
    fn add_assign(&mut self, o: V2) {
        self.x += o.x;
        self.y += o.y;
    }
}
impl Sub for V2 {
    type Output = V2;
    fn sub(self, o: V2) -> V2 {
        V2(self.x - o.x, self.y - o.y)
    }
}
impl Mul<f32> for V2 {
    type Output = V2;
    fn mul(self, k: f32) -> V2 {
        V2(self.x * k, self.y * k)
    }
}
impl Div<f32> for V2 {
    type Output = V2;
    fn div(self, k: f32) -> V2 {
        V2(self.x / k, self.y / k)
    }
}
impl Neg for V2 {
    type Output = V2;
    fn neg(self) -> V2 {
        V2(-self.x, -self.y)
    }
}

/// Integer screen rectangle, right/bottom exclusive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RectI {
    pub l: i32,
    pub t: i32,
    pub r: i32,
    pub b: i32,
}

impl RectI {
    pub fn w(&self) -> i32 {
        self.r - self.l
    }
    pub fn h(&self) -> i32 {
        self.b - self.t
    }
    pub fn contains(&self, p: V2) -> bool {
        p.x >= self.l as f32 && p.x < self.r as f32 && p.y >= self.t as f32 && p.y < self.b as f32
    }
    pub fn contains_inflated(&self, p: V2, d: f32) -> bool {
        p.x >= self.l as f32 - d && p.x < self.r as f32 + d && p.y >= self.t as f32 - d && p.y < self.b as f32 + d
    }
    pub fn tl(&self) -> V2 {
        V2(self.l as f32, self.t as f32)
    }
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
