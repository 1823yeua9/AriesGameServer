#pragma once

#include <algorithm>
#include <array>
#include <cmath>
#include <vector>

namespace internal::geometry {
    constexpr double pi = 3.14159265358979323846;

    struct Vector {
        double x = 0;
        double y = 0;
        double z = 0;

        auto operator+(Vector other) const -> Vector {
            return {x + other.x, y + other.y, z + other.z};
        }
        auto operator-(Vector other) const -> Vector {
            return {x - other.x, y - other.y, z - other.z};
        }
        auto operator*(double scale) const -> Vector {
            return {x * scale, y * scale, z * scale};
        }
    };

    struct Quaternion {
        double x = 0;
        double y = 0;
        double z = 0;
        double w = 1;
    };

    struct Transform {
        Quaternion rotation;
        Vector translation;
        Vector scale{1, 1, 1};
    };

    struct Line {
        Vector start;
        Vector end;
    };

    inline auto dot(Vector a, Vector b) -> double {
        return a.x * b.x + a.y * b.y + a.z * b.z;
    }

    inline auto cross(Vector a, Vector b) -> Vector {
        return {a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x};
    }

    inline auto finite(Vector value) -> bool {
        return std::isfinite(value.x) && std::isfinite(value.y) && std::isfinite(value.z);
    }

    inline auto rotate(Quaternion rotation, Vector point) -> Vector {
        const Vector axis{rotation.x, rotation.y, rotation.z};
        const auto twice_cross = cross(axis, point) * 2;

        return point + twice_cross * rotation.w + cross(axis, twice_cross);
    }

    inline auto transform_point(const Transform& transform, Vector point) -> Vector {
        return rotate(transform.rotation,
                      {point.x * transform.scale.x, point.y * transform.scale.y, point.z * transform.scale.z}) +
               transform.translation;
    }

    // UE rotators: pitch about -Y, yaw about Z, roll about -X.
    inline auto quaternion(double pitch, double yaw, double roll) -> Quaternion {
        const auto factor = pi / 360;
        const auto sp = std::sin(pitch * factor), cp = std::cos(pitch * factor);
        const auto sy = std::sin(yaw * factor), cy = std::cos(yaw * factor);
        const auto sr = std::sin(roll * factor), cr = std::cos(roll * factor);

        return {cr * sp * sy - sr * cp * cy,
                -cr * sp * cy - sr * cp * sy,
                cr * cp * sy - sr * sp * cy,
                cr * cp * cy + sr * sp * sy};
    }

    inline auto box(Vector center, Vector extent) -> std::vector<Line> {
        if (!finite(center) || !finite(extent) || extent.x < 0 || extent.y < 0 || extent.z < 0) {
            return {};
        }

        std::array<Vector, 8> corners{};

        for (int index = 0; index < 8; ++index) {
            corners[index] = center + Vector{index & 1 ? extent.x : -extent.x,
                                             index & 2 ? extent.y : -extent.y,
                                             index & 4 ? extent.z : -extent.z};
        }

        std::vector<Line> lines;

        for (int index = 0; index < 8; ++index) {
            for (int axis = 1; axis <= 4; axis <<= 1) {
                if (!(index & axis)) {
                    lines.push_back({corners[index], corners[index | axis]});
                }
            }
        }

        return lines;
    }

    // Length is the cylinder length, excluding the two hemispheres (FKSphylElem).
    inline auto capsule(double radius, double length, int segments = 16) -> std::vector<Line> {
        std::vector<Line> lines;

        if (!std::isfinite(radius) || !std::isfinite(length) || radius <= 0 || length < 0 || segments < 4) {
            return lines;
        }

        const auto half = length * 0.5;

        for (int index = 0; index < segments; ++index) {
            const auto a = 2 * pi * index / segments;
            const auto b = 2 * pi * (index + 1) / segments;

            for (const double sign : {-1.0, 1.0}) {
                lines.push_back({{radius * std::cos(a), radius * std::sin(a), sign * half},
                                 {radius * std::cos(b), radius * std::sin(b), sign * half}});
            }
        }

        for (int axis = 0; axis < 4; ++axis) {
            const auto azimuth = pi * axis / 2;
            const auto x = radius * std::cos(azimuth), y = radius * std::sin(azimuth);
            lines.push_back({{x, y, -half}, {x, y, half}});

            for (const double sign : {-1.0, 1.0}) {
                for (int index = 0; index < segments / 4; ++index) {
                    const auto a = 2 * pi * index / segments;
                    const auto b = 2 * pi * (index + 1) / segments;
                    lines.push_back({{x * std::cos(a), y * std::cos(a), sign * (half + radius * std::sin(a))},
                                     {x * std::cos(b), y * std::cos(b), sign * (half + radius * std::sin(b))}});
                }
            }
        }

        return lines;
    }

    struct Camera {
        Vector position;
        Quaternion rotation;
        double horizontal_fov = 90;
    };

    // Clip in camera space before perspective division; never mirror behind-camera edges.
    inline auto project(Line line, const Camera& camera, double width, double height, std::array<double, 4>& screen)
        -> bool {
        if (width <= 0 || height <= 0 || camera.horizontal_fov <= 1 || camera.horizontal_fov >= 179 ||
            !finite(line.start) || !finite(line.end)) {
            return false;
        }

        const Quaternion inverse{-camera.rotation.x, -camera.rotation.y, -camera.rotation.z, camera.rotation.w};
        auto a = rotate(inverse, line.start - camera.position);
        auto b = rotate(inverse, line.end - camera.position);
        constexpr double near_plane = 1;

        if (a.x < near_plane && b.x < near_plane) {
            return false;
        }

        if (a.x < near_plane) {
            a = a + (b - a) * ((near_plane - a.x) / (b.x - a.x));
        }

        if (b.x < near_plane) {
            b = b + (a - b) * ((near_plane - b.x) / (a.x - b.x));
        }

        const auto focal = width / (2 * std::tan(camera.horizontal_fov * pi / 360));
        screen = {width / 2 + a.y * focal / a.x,
                  height / 2 - a.z * focal / a.x,
                  width / 2 + b.y * focal / b.x,
                  height / 2 - b.z * focal / b.x};

        return std::all_of(screen.begin(), screen.end(), [](double value) {
            return std::isfinite(value);
        });
    }
} // namespace internal::geometry
