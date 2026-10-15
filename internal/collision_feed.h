#pragma once

#include "geometry.h"
#include <Windows.h>
#include <cstdint>
#include <memory>
#include <string>

// Versioned, process-local read-only geometry feed for the offline server.
// Keep this layout in sync with world::collision; neither side waits for a lock.
namespace internal::collision_feed {
    constexpr uint32_t protocol = 1;
    constexpr size_t max_targets = 64;
    constexpr size_t max_shapes = 128;

    struct Shape {
        uint32_t kind = 0;   // 0 sphere, 1 box, 2 capsule
        uint32_t region = 1; // 0 head, 1 body, 2 legs
        geometry::Vector center;
        geometry::Vector axes[3];
        geometry::Vector extent; // box half extents, or radius / unused / capsule half length
    };

    struct Target {
        uint64_t actor = 0;
        uint64_t sampled_at = 0;
        uint32_t count = 0;
        uint32_t unsupported = 0;
        Shape shapes[max_shapes]{};
    };

    struct Snapshot {
        uint32_t version = protocol;
        uint32_t count = 0;
        uint64_t world = 0;
        Target targets[max_targets]{};
    };

    static_assert(sizeof(Shape) == 128);
    static_assert(sizeof(Target) == 16408);
    static_assert(sizeof(Snapshot) == 1050128);

    class Publisher {
        HANDLE mapping = nullptr;
        HANDLE mutex = nullptr;
        void* view = nullptr;

    public:
        Publisher() {
            const auto name = L"Local\\AriesCollision_" + std::to_wstring(GetCurrentProcessId());
            mutex = CreateMutexW(nullptr, FALSE, (name + L"_lock").c_str());
            mapping =
                CreateFileMappingW(INVALID_HANDLE_VALUE, nullptr, PAGE_READWRITE, 0, sizeof(Snapshot), name.c_str());
            if (mapping) {
                view = MapViewOfFile(mapping, FILE_MAP_WRITE, 0, 0, sizeof(Snapshot));
            }
        }

        ~Publisher() {
            if (view) {
                UnmapViewOfFile(view);
            }
            if (mapping) {
                CloseHandle(mapping);
            }
            if (mutex) {
                CloseHandle(mutex);
            }
        }

        auto publish(const Snapshot& snapshot) -> bool {
            if (!view || !mutex) {
                return false;
            }
            const auto wait = WaitForSingleObject(mutex, 0);
            if (wait != WAIT_OBJECT_0 && wait != WAIT_ABANDONED) {
                return false;
            }
            memcpy(view, &snapshot, sizeof(snapshot));
            ReleaseMutex(mutex);
            return true;
        }

        Publisher(const Publisher&) = delete;
        auto operator=(const Publisher&) -> Publisher& = delete;
    };

    inline auto shape(uint32_t kind,
                      uint32_t region,
                      const geometry::Transform& local,
                      const geometry::Transform& world,
                      geometry::Vector extent) -> Shape {
        return {kind,
                region,
                geometry::transform_point(world, local.translation),
                {geometry::rotate(world.rotation, geometry::rotate(local.rotation, {1, 0, 0})),
                 geometry::rotate(world.rotation, geometry::rotate(local.rotation, {0, 1, 0})),
                 geometry::rotate(world.rotation, geometry::rotate(local.rotation, {0, 0, 1}))},
                extent * world.scale.x};
    }
} // namespace internal::collision_feed
