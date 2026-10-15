//! Standing Range target head calibration for build 13.06.
//!
//! Source: live PhysicsAsset Head bodies captured on 2026-10-14 by the internal
//! viewer. Coordinates are root-relative, inverse-rotated by actor yaw. Training
//! root (4463.2,-14407.7,1351.8), yaw 65.87055; dummy root (6190,-11640,1270),
//! yaw 180. Both have unit scale. This is a measured standing-pose model, NOT
//! animated skeletal authority. Torso/legs retain the existing approximation.
//! The old radius-22 sphere at root+75 included upper-chest space that none of
//! the actual Head bodies occupy. Weapon damage values must not compensate for it.

#![allow(non_upper_case_globals)]

type Point = [f64; 3];

#[derive(Clone, Copy)]
struct Capsule {
    start: Point,
    end: Point,
    radius: f64,
}

const training_shapes: [Capsule; 3] = [
    Capsule { start: [3.06513, -0.97190, 87.76], end: [3.06513, -0.97190, 87.76], radius: 12.0 },
    Capsule { start: [4.82375, -0.98403, 86.52], end: [3.65778, -1.56110, 80.65], radius: 11.47 },
    Capsule { start: [0.66900, -1.00426, 88.80], end: [-0.22728, -1.44957, 83.91], radius: 11.22 },
];

const dummy_shapes: [Capsule; 3] = [
    Capsule { start: [3.19, -1.10, 92.20], end: [3.19, -1.10, 92.20], radius: 12.0 },
    Capsule { start: [4.93, -1.10, 90.96], end: [3.84, -1.70, 85.09], radius: 11.47 },
    Capsule { start: [0.85, -1.10, 93.24], end: [-0.06, -1.60, 88.35], radius: 11.22 },
];

fn subtract(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn inverse_yaw(value: Point, yaw: f64) -> Point {
    let (sine, cosine) = yaw.to_radians().sin_cos();
    [cosine * value[0] + sine * value[1], -sine * value[0] + cosine * value[1], value[2]]
}

fn sphere(origin: Point, direction: Point, center: Point, radius: f64) -> Option<f64> {
    let offset = subtract(origin, center);
    let a = dot(direction, direction);
    let b = dot(offset, direction);
    let discriminant = b * b - a * (dot(offset, offset) - radius * radius);

    if a <= 1e-12 || discriminant < 0.0 {
        return None;
    }

    [(-b - discriminant.sqrt()) / a, (-b + discriminant.sqrt()) / a]
        .into_iter().find(|distance| *distance > 0.0)
}

fn capsule(origin: Point, direction: Point, shape: &Capsule) -> Option<f64> {
    let axis = subtract(shape.end, shape.start);
    let offset = subtract(origin, shape.start);
    let length_squared = dot(axis, axis);
    let mut nearest = [sphere(origin, direction, shape.start, shape.radius),
                       sphere(origin, direction, shape.end, shape.radius)]
        .into_iter().flatten().min_by(f64::total_cmp);

    if length_squared <= 1e-12 {
        return nearest;
    }

    let axis_direction = dot(axis, direction);
    let axis_offset = dot(axis, offset);
    let a = length_squared * dot(direction, direction) - axis_direction * axis_direction;
    let b = length_squared * dot(offset, direction) - axis_offset * axis_direction;
    let c = length_squared * (dot(offset, offset) - shape.radius * shape.radius) - axis_offset * axis_offset;
    let discriminant = b * b - a * c;

    if a > 1e-12 && discriminant >= 0.0 {
        for distance in [(-b - discriminant.sqrt()) / a, (-b + discriminant.sqrt()) / a] {
            let along_axis = axis_offset + distance * axis_direction;
            if distance > 0.0 && (0.0..=length_squared).contains(&along_axis)
                && nearest.is_none_or(|current| distance < current)
            {
                nearest = Some(distance);
            }
        }
    }

    nearest
}

pub fn head_hit(origin: Point, direction: Point, root: Point, yaw: f64, dummy: bool) -> Option<f64> {
    if origin.into_iter().chain(direction).chain(root).chain([yaw]).any(|value| !value.is_finite()) {
        return None;
    }

    let local_origin = inverse_yaw(subtract(origin, root), yaw);
    let local_direction = inverse_yaw(direction, yaw);
    let shapes = if dummy { &dummy_shapes } else { &training_shapes };

    shapes.iter().filter_map(|shape| capsule(local_origin, local_direction, shape)).min_by(f64::total_cmp)
}

/// The torso approximation meets the lowest measured Head body, instead of
/// leaving the old artificial head/chest band classified as a headshot.
pub fn head_floor(dummy: bool) -> f64 {
    let shapes = if dummy { &dummy_shapes } else { &training_shapes };
    shapes.iter().map(|shape| shape.start[2].min(shape.end[2]) - shape.radius)
        .min_by(f64::total_cmp).expect("nonempty head calibration")
}
