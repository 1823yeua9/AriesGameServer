//! interpolatable value types. every formula keeps the exact operation order
//! from docs/port/10_interp_source_sdk.md 4.3 so results are bit-reproducible
//! (no mul_add, no reassociation).

pub trait Interp: Copy {
    /// a + (b - a) * f, per component, in that order.
    fn lerp(a: Self, b: Self, f: f64) -> Self;
    /// whether three-point hermite applies (rotations and steps never use it).
    const HERMITE: bool = false;
    /// hermite over [p1, p2]; p0 is already time-fixed. default: lerp(p1, p2, t).
    fn hermite(_p0: Self, p1: Self, p2: Self, t: f64) -> Self {
        Self::lerp(p1, p2, t)
    }
    /// continue old -> new to factor f (> 1). step types hold `new`.
    fn extrapolate(old: Self, new: Self, f: f64) -> Self {
        Self::lerp(old, new, f)
    }
}

/// values with a meaningful rate of change.
pub trait Velocity: Interp {
    /// (b - a) * scale, per component.
    fn rate(a: Self, b: Self, scale: f64) -> Self;
    /// hermite derivative over [p1, p2] in units per unit t (divide by the span).
    fn hermite_rate(p0: Self, p1: Self, p2: Self, t: f64, span: f64) -> Self;
    fn scaled(self, f: f64) -> Self;
}

fn hermite_basis(t: f64) -> [f64; 4] {
    let t2 = t * t;
    let t3 = t * t2;
    [2.0 * t3 - 3.0 * t2 + 1.0, -2.0 * t3 + 3.0 * t2, t3 - 2.0 * t2 + t, t3 - t2]
}

fn hermite_rate_basis(t: f64) -> [f64; 4] {
    let t2 = t * t;
    [6.0 * t2 - 6.0 * t, -6.0 * t2 + 6.0 * t, 3.0 * t2 - 4.0 * t + 1.0, 3.0 * t2 - 2.0 * t]
}

fn hermite1(p0: f64, p1: f64, p2: f64, h: [f64; 4]) -> f64 {
    let d1 = p1 - p0;
    let d2 = p2 - p1;
    p1 * h[0] + p2 * h[1] + d1 * h[2] + d2 * h[3]
}

impl Interp for f64 {
    const HERMITE: bool = true;

    fn lerp(a: Self, b: Self, f: f64) -> Self {
        a + (b - a) * f
    }

    fn hermite(p0: Self, p1: Self, p2: Self, t: f64) -> Self {
        hermite1(p0, p1, p2, hermite_basis(t))
    }
}

impl Velocity for f64 {
    fn rate(a: Self, b: Self, scale: f64) -> Self {
        (b - a) * scale
    }

    fn hermite_rate(p0: Self, p1: Self, p2: Self, t: f64, span: f64) -> Self {
        hermite1(p0, p1, p2, hermite_rate_basis(t)) / span
    }

    fn scaled(self, f: f64) -> Self {
        self * f
    }
}

/// a location (ue5 locations are doubles).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn to_array(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }

    fn map3(a: Self, b: Self, c: Self, f: impl Fn(f64, f64, f64) -> f64) -> Self {
        Self { x: f(a.x, b.x, c.x), y: f(a.y, b.y, c.y), z: f(a.z, b.z, c.z) }
    }
}

impl From<[f64; 3]> for Vec3 {
    fn from(v: [f64; 3]) -> Self {
        Self { x: v[0], y: v[1], z: v[2] }
    }
}

impl Interp for Vec3 {
    const HERMITE: bool = true;

    fn lerp(a: Self, b: Self, f: f64) -> Self {
        Self { x: f64::lerp(a.x, b.x, f), y: f64::lerp(a.y, b.y, f), z: f64::lerp(a.z, b.z, f) }
    }

    fn hermite(p0: Self, p1: Self, p2: Self, t: f64) -> Self {
        let h = hermite_basis(t);
        Self::map3(p0, p1, p2, |a, b, c| hermite1(a, b, c, h))
    }
}

impl Velocity for Vec3 {
    fn rate(a: Self, b: Self, scale: f64) -> Self {
        Self { x: (b.x - a.x) * scale, y: (b.y - a.y) * scale, z: (b.z - a.z) * scale }
    }

    fn hermite_rate(p0: Self, p1: Self, p2: Self, t: f64, span: f64) -> Self {
        let h = hermite_rate_basis(t);
        Self::map3(p0, p1, p2, |a, b, c| hermite1(a, b, c, h) / span)
    }

    fn scaled(self, f: f64) -> Self {
        Self { x: self.x * f, y: self.y * f, z: self.z * f }
    }
}

/// x wrapped into [-180, 180).
pub fn wrap180(x: f64) -> f64 {
    x - 360.0 * ((x + 180.0) / 360.0).floor()
}

/// pitch/yaw/roll in degrees. lerped per axis along the shortest arc and
/// always returned wrapped; never hermite (source slerps angles).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rot {
    pub pitch: f64,
    pub yaw: f64,
    pub roll: f64,
}

impl Rot {
    pub const fn new(pitch: f64, yaw: f64, roll: f64) -> Self {
        Self { pitch, yaw, roll }
    }
}

fn lerp_angle(a: f64, b: f64, f: f64) -> f64 {
    wrap180(a + wrap180(b - a) * f)
}

impl Interp for Rot {
    fn lerp(a: Self, b: Self, f: f64) -> Self {
        Self { pitch: lerp_angle(a.pitch, b.pitch, f), yaw: lerp_angle(a.yaw, b.yaw, f), roll: lerp_angle(a.roll, b.roll, f) }
    }
}

/// discrete values (alive flags, enums): the older value until f >= 1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Step<T: Copy>(pub T);

impl<T: Copy> Interp for Step<T> {
    fn lerp(a: Self, b: Self, f: f64) -> Self {
        if f < 1.0 { a } else { b }
    }

    fn extrapolate(_old: Self, new: Self, _f: f64) -> Self {
        new
    }
}
