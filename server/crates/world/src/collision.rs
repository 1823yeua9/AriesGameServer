//! Live PhysicsAsset primitives, not root capsules or actor bounding boxes.
//! Wire layout matches internal/collision_feed.h. Coordinates are world cm.

use crate::bots::Region;

pub const MAX_TARGETS: usize = 64;
pub const MAX_SHAPES: usize = 128;
pub const MAX_AGE_MS: u64 = 250;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Shape {
    pub kind: u32,
    pub region: u32,
    pub center: [f64; 3],
    pub axes: [[f64; 3]; 3],
    pub extent: [f64; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Target {
    pub actor: u64,
    pub sampled_at: u64,
    pub count: u32,
    pub unsupported: u32,
    pub shapes: [Shape; MAX_SHAPES],
}

#[repr(C)]
pub struct Snapshot {
    pub version: u32,
    pub count: u32,
    pub world: u64,
    pub targets: [Target; MAX_TARGETS],
}

const _: () = assert!(size_of::<Shape>() == 128);
const _: () = assert!(size_of::<Target>() == 16408);
const _: () = assert!(size_of::<Snapshot>() == 1050128);

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

fn sphere(origin: [f64; 3], direction: [f64; 3], z: f64, radius: f64) -> Option<f64> {
    let offset = [origin[0], origin[1], origin[2] - z];
    let c = dot(offset, offset) - radius * radius;
    if c <= 0.0 { return Some(0.0); }
    let b = dot(offset, direction);
    let discriminant = b * b - c;
    if discriminant < 0.0 { return None; }
    let distance = -b - discriminant.sqrt();
    (distance >= 0.0).then_some(distance)
}

impl Shape {
    pub fn valid(&self) -> bool {
        if self.kind > 2 || self.region > 2 || self.extent.iter().any(|v| *v < 0.0) ||
            self.center.iter().chain(self.axes.iter().flatten()).chain(self.extent.iter()).any(|v| !v.is_finite()) {
            return false;
        }
        for i in 0..3 {
            if (dot(self.axes[i], self.axes[i]) - 1.0).abs() > 0.001 { return false; }
            for j in 0..i {
                if dot(self.axes[i], self.axes[j]).abs() > 0.001 { return false; }
            }
        }
        self.extent[0] > 0.0 && (self.kind != 1 || self.extent.iter().all(|v| *v > 0.0))
    }

    pub fn hit(&self, origin: [f64; 3], direction: [f64; 3]) -> Option<(Region, f64)> {
        if !self.valid() || origin.into_iter().chain(direction).any(|v| !v.is_finite()) ||
            (dot(direction, direction) - 1.0).abs() > 0.001 { return None; }
        let offset = std::array::from_fn(|i| origin[i] - self.center[i]);
        let origin = self.axes.map(|axis| dot(offset, axis));
        let direction = self.axes.map(|axis| dot(direction, axis));
        let distance = match self.kind {
            0 => sphere(origin, direction, 0.0, self.extent[0]),
            1 => {
                let mut near: f64 = 0.0;
                let mut far = f64::INFINITY;
                for axis in 0..3 {
                    if direction[axis].abs() < 1e-12 {
                        if origin[axis].abs() > self.extent[axis] { return None; }
                    } else {
                        let a = (-self.extent[axis] - origin[axis]) / direction[axis];
                        let b = (self.extent[axis] - origin[axis]) / direction[axis];
                        near = near.max(a.min(b));
                        far = far.min(a.max(b));
                        if near > far { return None; }
                    }
                }
                Some(near)
            }
            2 => {
                let radius = self.extent[0];
                let half = self.extent[2];
                let dz = origin[2] - origin[2].clamp(-half, half);
                if origin[0] * origin[0] + origin[1] * origin[1] + dz * dz <= radius * radius {
                    Some(0.0)
                } else {
                    let mut nearest = [sphere(origin, direction, -half, radius), sphere(origin, direction, half, radius)]
                        .into_iter().flatten().min_by(f64::total_cmp);
                    let a = direction[0] * direction[0] + direction[1] * direction[1];
                    let b = origin[0] * direction[0] + origin[1] * direction[1];
                    let c = origin[0] * origin[0] + origin[1] * origin[1] - radius * radius;
                    let discriminant = b * b - a * c;
                    if a > 1e-12 && discriminant >= 0.0 {
                        for distance in [(-b - discriminant.sqrt()) / a, (-b + discriminant.sqrt()) / a] {
                            if distance >= 0.0 && (origin[2] + direction[2] * distance).abs() <= half &&
                                nearest.is_none_or(|previous| distance < previous) { nearest = Some(distance); }
                        }
                    }
                    nearest
                }
            }
            _ => None,
        }?;
        Some((match self.region { 0 => Region::Head, 2 => Region::Legs, _ => Region::Body }, distance))
    }
}

impl Target {
    pub fn usable(&self, now_ms: u64) -> bool {
        self.actor != 0 && self.sampled_at <= now_ms && now_ms - self.sampled_at <= MAX_AGE_MS &&
            self.unsupported == 0 && self.count > 0 && self.count as usize <= MAX_SHAPES &&
            self.shapes[..self.count as usize].iter().all(Shape::valid)
    }

    pub fn hit(&self, origin: [f64; 3], direction: [f64; 3], now_ms: u64) -> Option<(Region, f64)> {
        if !self.usable(now_ms) { return None; }
        self.shapes[..self.count as usize].iter().filter_map(|shape| shape.hit(origin, direction))
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }
}
