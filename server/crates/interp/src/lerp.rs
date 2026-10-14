//! value types the histories can blend.

/// linear blend plus the hermite combination. lerp with t outside [0, 1]
/// extrapolates along the same line.
pub trait Lerp: Sized {
    fn lerp(from: &Self, to: &Self, t: f64) -> Self;

    /// basis = [p1, p2, tangent at p1, tangent at p2]; tangents are p1 - p0 and p2 - p1.
    fn hermite(p0: &Self, p1: &Self, p2: &Self, basis: [f64; 4]) -> Self;
}

impl Lerp for f64 {
    fn lerp(from: &Self, to: &Self, t: f64) -> Self {
        from + (to - from) * t
    }

    fn hermite(p0: &Self, p1: &Self, p2: &Self, basis: [f64; 4]) -> Self {
        basis[0] * p1 + basis[1] * p2 + basis[2] * (p1 - p0) + basis[3] * (p2 - p1)
    }
}

impl Lerp for [f64; 3] {
    fn lerp(from: &Self, to: &Self, t: f64) -> Self {
        std::array::from_fn(|i| f64::lerp(&from[i], &to[i], t))
    }

    fn hermite(p0: &Self, p1: &Self, p2: &Self, basis: [f64; 4]) -> Self {
        std::array::from_fn(|i| f64::hermite(&p0[i], &p1[i], &p2[i], basis))
    }
}

/// shortest signed difference between two angles in degrees, in (-180, 180].
pub fn angle_delta(from: f64, to: f64) -> f64 {
    let delta = (to - from).rem_euclid(360.0);
    if delta > 180.0 { delta - 360.0 } else { delta }
}

/// pitch/yaw/roll in degrees; blends along the shortest arc so 359 -> 1 goes through 0.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rotator(pub [f64; 3]);

impl Lerp for Rotator {
    fn lerp(from: &Self, to: &Self, t: f64) -> Self {
        Rotator(std::array::from_fn(|i| from.0[i] + angle_delta(from.0[i], to.0[i]) * t))
    }

    fn hermite(p0: &Self, p1: &Self, p2: &Self, basis: [f64; 4]) -> Self {
        // unwrap p0 and p2 around p1 so the spline never crosses the seam
        Rotator(std::array::from_fn(|i| {
            let a = p1.0[i] - angle_delta(p0.0[i], p1.0[i]);
            let c = p1.0[i] + angle_delta(p1.0[i], p2.0[i]);
            f64::hermite(&a, &p1.0[i], &c, basis)
        }))
    }
}

/// a view: eye location plus rotation (the camera pov the oracle reads).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct View {
    pub location: [f64; 3],
    pub rotation: Rotator,
}

impl Lerp for View {
    fn lerp(from: &Self, to: &Self, t: f64) -> Self {
        View { location: <[f64; 3]>::lerp(&from.location, &to.location, t), rotation: Rotator::lerp(&from.rotation, &to.rotation, t) }
    }

    fn hermite(p0: &Self, p1: &Self, p2: &Self, basis: [f64; 4]) -> Self {
        View {
            location: <[f64; 3]>::hermite(&p0.location, &p1.location, &p2.location, basis),
            rotation: Rotator::hermite(&p0.rotation, &p1.rotation, &p2.rotation, basis),
        }
    }
}
