#include "entity_viewer.h"
#include "geometry.h"
#include "session.h"
#include "overlay/sdk_reflect.hpp"
#include "overlay/vendor/imgui/imgui.h"

#include "../../dumper7/output/5.3.2-5590001+++Ares-Core+release-13.06-ShooterGame/CppSDK/SDK/Engine_classes.hpp"
#include "../../dumper7/output/5.3.2-5590001+++Ares-Core+release-13.06-ShooterGame/CppSDK/SDK/Engine_parameters.hpp"
#include "../../dumper7/valorant_sdk_adapter.hpp"

#include <atomic>
#include <chrono>
#include <map>
#include <memory>
#include <mutex>
#include <set>
#include <sstream>
#include <string>

namespace internal::entity_viewer {
    using geometry::Line;
    using geometry::Transform;
    using geometry::Vector;

    struct Entity {
        std::string name;
        std::vector<Line> bounds;
        std::vector<Line> shapes;
        uint64_t sweep = 0;
        unsigned unsupported = 0;
        bool audit = false;
    };

    struct Snapshot {
        std::map<int, std::shared_ptr<const Entity>> entities;
        geometry::Camera camera;
        bool camera_valid = false;
        unsigned visited = 0;
        unsigned faults = 0;
        double sample_ms = 0;
        std::string world_name;
        ULONGLONG sampled_at = 0;
    };

    static constexpr UINT_PTR timer_id = 0x41524945;
    static std::atomic_bool bounds_enabled = false;
    static std::atomic_bool shapes_enabled = false;
    static std::atomic_uint sample_faults = 0;
    static HHOOK message_hook = nullptr;
    static HWND game_window = nullptr;
    static std::mutex snapshot_mutex;
    static std::shared_ptr<const Snapshot> published_snapshot = std::make_shared<Snapshot>();
    static bool labels_enabled = false;

    static auto describe(const Vector& value) -> std::string {
        std::ostringstream text;
        text << value.x << ',' << value.y << ',' << value.z;

        return text.str();
    }

    static auto read_snapshot() -> std::shared_ptr<const Snapshot> {
        std::lock_guard lock(snapshot_mutex);

        return published_snapshot;
    }

    static auto vector(const SDK::FVector& value) -> Vector {
        return {value.X, value.Y, value.Z};
    }

    static auto transform(const SDK::FTransform& value) -> Transform {
        return {{value.Rotation.X, value.Rotation.Y, value.Rotation.Z, value.Rotation.W},
                vector(value.Translation),
                vector(value.Scale3D)};
    }

    static auto readable(const void* pointer, size_t size) -> bool {
        return valorant_sdk_adapter::is_readable(pointer, size);
    }

    template <typename Element> static auto valid_array(const SDK::TArray<Element>& array, int limit = 65536) -> bool {
        return array.Num() >= 0 && array.Num() <= limit && array.Max() >= array.Num() &&
               (!array.Num() || readable(array.GetDataPtr(), array.Num() * sizeof(Element)));
    }

    static auto is_a(const SDK::UObject* object, const char* name) -> bool {
        if (!readable(object, sizeof(SDK::UObject))) {
            return false;
        }

        auto* type = static_cast<SDK::UStruct*>(object->Class);

        for (int depth = 0; depth < 40 && readable(type, sizeof(SDK::UStruct)); ++depth, type = type->SuperStruct) {
            if (type->Name.GetRawString() == name) {
                return true;
            }
        }

        return false;
    }

    static auto invoke(SDK::UObject* object, const char* owner, const char* name, void* params) -> bool {
        // Functions are rooted class metadata. Validate their identities before reuse.
        static std::map<std::pair<std::string, std::string>, SDK::UFunction*> functions;
        auto*& function = functions[{owner, name}];

        if (!readable(function, sizeof(SDK::UFunction)) || function->Name.GetRawString() != name) {
            function = sdk_reflect::find_function(owner, name);
        }

        if (!function || !readable(object, sizeof(SDK::UObject))) {
            return false;
        }

        object->ProcessEvent(function, params);

        return true;
    }

    static auto
    append_shape(Entity& entity, const std::vector<Line>& lines, const Transform& local, const Transform& world)
        -> void {
        for (const auto& line : lines) {
            const auto a = geometry::transform_point(world, geometry::transform_point(local, line.start));
            const auto b = geometry::transform_point(world, geometry::transform_point(local, line.end));

            if (geometry::finite(a) && geometry::finite(b)) {
                entity.shapes.push_back({a, b});
            }
        }
    }

    static auto collect_physics(Entity& entity, SDK::UObject* mesh) -> void {
        if (!is_a(mesh, "SkinnedMeshComponent")) {
            return;
        }

        auto* asset = sdk_reflect::object_property(mesh, "PhysicsAssetOverride");

        if (!asset) {
            auto* skeletal_mesh = sdk_reflect::object_property(mesh, "SkeletalMeshAsset");

            if (!skeletal_mesh) {
                skeletal_mesh = sdk_reflect::object_property(mesh, "SkeletalMesh");
            }

            asset = sdk_reflect::object_property(skeletal_mesh, "PhysicsAsset");
        }

        if (!is_a(asset, "PhysicsAsset") || !readable(asset, sizeof(SDK::UPhysicsAsset))) {
            return;
        }

        const auto& bodies = static_cast<SDK::UPhysicsAsset*>(asset)->SkeletalBodySetups;

        if (!valid_array(bodies, 256)) {
            return;
        }

        for (auto* body : bodies) {
            if (!readable(body, sizeof(SDK::USkeletalBodySetup))) {
                continue;
            }

            SDK::Params::SceneComponent_DoesSocketExist exists{};
            exists.InSocketName = body->BoneName;

            if (!invoke(mesh, "SceneComponent", "DoesSocketExist", &exists) || !exists.ReturnValue) {
                ++entity.unsupported;
                continue;
            }

            SDK::Params::SceneComponent_GetSocketTransform socket{};
            socket.InSocketName = body->BoneName;
            socket.TransformSpace = SDK::ERelativeTransformSpace::RTS_World;

            if (!invoke(mesh, "SceneComponent", "GetSocketTransform", &socket)) {
                continue;
            }

            const auto world = transform(socket.ReturnValue);
            const auto& geometry = body->AggGeom;

            if (entity.audit) {
                session::log("Target geometry " + entity.name + " bone=" + body->BoneName.GetRawString() +
                             " world=" + describe(world.translation) + " scale=" + describe(world.scale));
            }

            // Affinely stretching capsules/spheres would invent an ellipsoid,
            // not the engine's collision shape. Report unsupported scaling until
            // its solver-specific scaling rule is verified.
            if (!geometry::finite(world.scale) || world.scale.x <= 0 ||
                std::abs(world.scale.x - world.scale.y) > 0.0001 || std::abs(world.scale.x - world.scale.z) > 0.0001) {
                ++entity.unsupported;
                continue;
            }

            if (!valid_array(geometry.SphereElems, 128) || !valid_array(geometry.BoxElems, 128) ||
                !valid_array(geometry.SphylElems, 128)) {
                continue;
            }

            for (const auto& sphere : geometry.SphereElems) {
                if (entity.audit) {
                    session::log(
                        "  sphere center=" + describe(geometry::transform_point(world, vector(sphere.Center))) +
                        " radius=" + std::to_string(sphere.Radius));
                }
                append_shape(entity, geometry::capsule(sphere.Radius, 0), {{}, vector(sphere.Center)}, world);
            }

            for (const auto& box : geometry.BoxElems) {
                const Transform local{geometry::quaternion(box.Rotation.Pitch, box.Rotation.Yaw, box.Rotation.Roll),
                                      vector(box.Center)};
                if (entity.audit) {
                    session::log("  box center=" + describe(geometry::transform_point(world, vector(box.Center))) +
                                 " dimensions=" + describe({box.X, box.Y, box.Z}));
                }
                append_shape(entity, geometry::box({}, {box.X / 2, box.Y / 2, box.Z / 2}), local, world);
            }

            for (const auto& capsule : geometry.SphylElems) {
                const Transform local{
                    geometry::quaternion(capsule.Rotation.Pitch, capsule.Rotation.Yaw, capsule.Rotation.Roll),
                    vector(capsule.Center)};
                if (entity.audit) {
                    const auto start =
                        geometry::transform_point(world, geometry::transform_point(local, {0, 0, -capsule.Length / 2}));
                    const auto end =
                        geometry::transform_point(world, geometry::transform_point(local, {0, 0, capsule.Length / 2}));
                    session::log("  capsule start=" + describe(start) + " end=" + describe(end) +
                                 " radius=" + std::to_string(capsule.Radius * world.scale.x));
                }
                append_shape(entity, geometry::capsule(capsule.Radius, capsule.Length), local, world);
            }

            for (const auto count : {geometry.ConvexElems.Num(),
                                     geometry.TaperedCapsuleElems.Num(),
                                     geometry.LevelSetElems.Num(),
                                     geometry.SkinnedLevelSetElems.Num()}) {
                entity.unsupported += static_cast<unsigned>(std::clamp(count, 0, 256));
            }
        }
    }

    static auto collect_root_shape(Entity& entity, SDK::UObject* root) -> void {
        if (!is_a(root, "ShapeComponent")) {
            return;
        }

        SDK::Params::SceneComponent_K2_GetComponentToWorld placement{};

        if (!invoke(root, "SceneComponent", "K2_GetComponentToWorld", &placement)) {
            return;
        }

        auto world = transform(placement.ReturnValue);

        if (entity.audit) {
            session::log("Target component " + root->Name.GetRawString() + " world=" + describe(world.translation));
        }

        if (is_a(root, "CapsuleComponent") && readable(root, sizeof(SDK::UCapsuleComponent))) {
            // Use the engine's scaled dimensions: nonuniform scaling is NOT a stretched ellipsoid.
            SDK::Params::CapsuleComponent_GetScaledCapsuleSize size{};

            if (invoke(root, "CapsuleComponent", "GetScaledCapsuleSize", &size)) {
                world.scale = {1, 1, 1};
                append_shape(entity,
                             geometry::capsule(size.OutRadius, 2 * std::max(0.0f, size.OutHalfHeight - size.OutRadius)),
                             {},
                             world);
            }
        }
        else if (is_a(root, "BoxComponent") && readable(root, sizeof(SDK::UBoxComponent))) {
            append_shape(
                entity, geometry::box({}, vector(static_cast<SDK::UBoxComponent*>(root)->BoxExtent)), {}, world);
        }
        else if (is_a(root, "SphereComponent") && readable(root, sizeof(SDK::USphereComponent))) {
            SDK::Params::SphereComponent_GetScaledSphereRadius size{};

            if (invoke(root, "SphereComponent", "GetScaledSphereRadius", &size)) {
                world.scale = {1, 1, 1};
                append_shape(entity, geometry::capsule(size.ReturnValue, 0), {}, world);
            }
        }
    }

    static auto collect_component_shapes(Entity& entity, SDK::UObject* root, SDK::UObject* mesh) -> void {
        std::vector<SDK::UObject*> components;
        components.reserve(64);

        const auto add = [&](SDK::UObject* component) {
            if (component && std::find(components.begin(), components.end(), component) == components.end()) {
                if (components.size() < 64) {
                    components.push_back(component);
                }
                else {
                    ++entity.unsupported;
                }
            }
        };

        add(root);
        add(mesh);

        for (size_t index = 0; index < components.size(); ++index) {
            auto* component = components[index];

            if (!is_a(component, "SceneComponent") || !readable(component, sizeof(SDK::USceneComponent))) {
                continue;
            }

            collect_root_shape(entity, component);
            collect_physics(entity, component);
            const auto& children = static_cast<SDK::USceneComponent*>(component)->AttachChildren;

            if (valid_array(children, 256)) {
                for (auto* child : children) {
                    add(child);
                }
            }
        }
    }

    static auto sample() -> void {
        if (!bounds_enabled && !shapes_enabled) {
            return;
        }

        const auto started = std::chrono::steady_clock::now();
        static SDK::UObject* engine = nullptr;
        static SDK::UWorld* previous_world = nullptr;
        static SDK::UObject* controller = nullptr;
        static int level_index = 0;
        static int actor_index = 0;
        static uint64_t sweep = 1;
        // Only the window thread mutates this state. Renderers hold immutable
        // snapshots, so engine calls never hold a lock needed by Present.
        static Snapshot snapshot;
        static std::set<std::string> audited_classes;

        if (!is_a(engine, "Engine")) {
            engine = reinterpret_cast<SDK::UObject*>(sdk_reflect::engine());
        }

        auto* viewport = sdk_reflect::object_property(engine, "GameViewport");
        auto* world = reinterpret_cast<SDK::UWorld*>(sdk_reflect::object_property(viewport, "World"));

        if (!readable(world, sizeof(SDK::UWorld)) || !valid_array(world->Levels, 1024)) {
            return;
        }

        if (world != previous_world) {
            snapshot = {};
            snapshot.world_name = world->Name.GetRawString();
            previous_world = world;
            controller = nullptr;
            level_index = actor_index = 0;
            sweep = 1;
            audited_classes.clear();
        }

        if (!is_a(controller, "PlayerController") || sdk_reflect::object_property(controller, "Player") == nullptr) {
            controller = sdk_reflect::local_player_controller();
        }

        auto* camera = sdk_reflect::object_property(controller, "PlayerCameraManager");
        SDK::Params::PlayerCameraManager_GetCameraLocation location{};
        SDK::Params::PlayerCameraManager_GetCameraRotation rotation{};
        SDK::Params::PlayerCameraManager_GetFOVAngle fov{};
        snapshot.camera_valid = invoke(camera, "PlayerCameraManager", "GetCameraLocation", &location) &&
                                invoke(camera, "PlayerCameraManager", "GetCameraRotation", &rotation) &&
                                invoke(camera, "PlayerCameraManager", "GetFOVAngle", &fov);
        snapshot.camera = {
            vector(location.ReturnValue),
            geometry::quaternion(rotation.ReturnValue.Pitch, rotation.ReturnValue.Yaw, rotation.ReturnValue.Roll),
            fov.ReturnValue};

        const auto now = GetTickCount64();
        unsigned processed = 0;

        while (level_index < world->Levels.Num() && processed < 64) {
            auto* level = world->Levels[level_index];

            if (!readable(level, sizeof(SDK::ULevel)) || level->OwningWorld != world || !valid_array(level->Actors) ||
                actor_index >= level->Actors.Num()) {
                ++level_index;
                actor_index = 0;
                continue;
            }

            auto* actor = level->Actors[actor_index++];
            ++processed;

            if (!is_a(actor, "Actor")) {
                continue;
            }

            auto* root = sdk_reflect::object_property(actor, "RootComponent");

            if (!root) {
                continue;
            }

            SDK::Params::Actor_GetActorBounds bounds{};

            if (!invoke(actor, "Actor", "GetActorBounds", &bounds)) {
                continue;
            }

            const auto extent = vector(bounds.BoxExtent);

            if (!geometry::finite(extent) || geometry::dot(extent, extent) < 0.001) {
                continue;
            }

            Entity entity;
            entity.name = actor->Name.GetRawString();
            entity.sweep = sweep;

            if (bounds_enabled) {
                entity.bounds = geometry::box(vector(bounds.Origin), extent);
            }

            if (shapes_enabled) {
                const auto class_name = actor->Class->Name.GetRawString();
                entity.audit =
                    class_name.find("TrainingBot") != std::string::npos && audited_classes.insert(class_name).second;
                if (entity.audit) {
                    session::log("Target actor " + entity.name + " bounds_origin=" + describe(vector(bounds.Origin)) +
                                 " half_extents=" + describe(extent));
                }
                auto* mesh = sdk_reflect::object_property(actor, "Mesh");
                collect_component_shapes(entity, root, mesh);
            }

            snapshot.entities[actor->Index] = std::make_shared<Entity>(std::move(entity));

            if (std::chrono::steady_clock::now() - started > std::chrono::milliseconds(2)) {
                break;
            }
        }

        if (level_index >= world->Levels.Num()) {
            level_index = actor_index = 0;
            // Expire by completed traversal, not an arbitrary two-second TTL:
            // large worlds can take longer than that to revisit an actor.
            std::erase_if(snapshot.entities, [completed_sweep = sweep](const auto& item) {
                return item.second->sweep != completed_sweep;
            });
            ++sweep;
        }

        snapshot.sampled_at = now;
        snapshot.visited = static_cast<unsigned>(snapshot.entities.size());
        snapshot.sample_ms =
            std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - started).count();

        auto published = std::make_shared<Snapshot>(snapshot);
        {
            std::lock_guard lock(snapshot_mutex);
            published_snapshot = std::move(published);
        }
    }

    static auto guarded_sample() -> void {
        __try {
            sample();
        }
        __except (EXCEPTION_EXECUTE_HANDLER) {
            // A travel/GC race must disable sampling, not crash or repeatedly fault each frame.
            bounds_enabled = false;
            shapes_enabled = false;
            ++sample_faults;
        }
    }

    static auto CALLBACK on_message(int code, WPARAM word, LPARAM parameter) -> LRESULT {
        if (code >= 0 && word == PM_REMOVE) {
            const auto* message = reinterpret_cast<MSG*>(parameter);

            if (message && message->hwnd == game_window && message->message == WM_TIMER &&
                message->wParam == timer_id) {
                guarded_sample();
            }
        }

        return CallNextHookEx(message_hook, code, word, parameter);
    }

    auto attach(HWND window) -> bool {
        if (message_hook) {
            return game_window == window;
        }

        DWORD pid = 0;
        const auto thread = GetWindowThreadProcessId(window, &pid);

        if (pid != GetCurrentProcessId()) {
            return false;
        }

        game_window = window;
        message_hook = SetWindowsHookExW(WH_GETMESSAGE, on_message, nullptr, thread);

        if (!message_hook || !SetTimer(window, timer_id, 33, nullptr)) {
            if (message_hook) {
                UnhookWindowsHookEx(message_hook);
                message_hook = nullptr;
            }

            return false;
        }

        session::log("Entity sampler attached to game window thread; read-only, toggles off");

        if (std::filesystem::exists(session::runtime_directory() / "capture_target_geometry")) {
            shapes_enabled = true;
            session::log("Requested target geometry capture enabled for this run");
        }

        return true;
    }

    auto draw_options() -> void {
        bool bounds = bounds_enabled.load(), shapes = shapes_enabled.load();

        if (ImGui::Checkbox("3D entity boxes (engine world bounds)", &bounds)) {
            bounds_enabled = bounds;
        }

        if (ImGui::Checkbox("Hitbox viewer (client collision geometry)", &shapes)) {
            shapes_enabled = shapes;
        }

        ImGui::Checkbox("Entity names", &labels_enabled);
        ImGui::TextWrapped(
            "Cyan: actor bounds. Gold: root collision and skeletal PhysicsAsset shapes. These are client shapes, not a claim of matching the external server's hit tests.");
        ImGui::TextDisabled(
            "Read-only. No collision resizing. Convex/level-set/tapered shapes are reported, not approximated.");
        ImGui::TextDisabled(
            "All streamed levels; round-robin sampling, 2 ms target / 64 actors per tick; entries retire after a full sweep.");
        const auto current = read_snapshot();
        const auto& snapshot = *current;
        unsigned unsupported = 0;

        for (const auto& [index, entity] : snapshot.entities) {
            unsupported += entity->unsupported;
        }

        ImGui::Text("World: %s | Entities: %u | Last sample: %.2f ms | Unsupported shapes: %u",
                    snapshot.world_name.c_str(),
                    snapshot.visited,
                    snapshot.sample_ms,
                    unsupported);
        ImGui::Text("Sampler: %s | Camera: %s",
                    message_hook ? "attached" : "unavailable",
                    snapshot.camera_valid ? "valid" : "unavailable");
        ImGui::TextDisabled("Read faults: %u (sampling disables on a fault)", sample_faults.load());
    }

    auto draw_world() -> void {
        if (!bounds_enabled && !shapes_enabled) {
            return;
        }

        const auto current = read_snapshot();
        const auto& snapshot = *current;

        if (!snapshot.camera_valid || GetTickCount64() - snapshot.sampled_at > 2000) {
            return;
        }

        auto* draw = ImGui::GetBackgroundDrawList();
        const auto size = ImGui::GetIO().DisplaySize;
        size_t line_count = 0;

        for (const auto& [index, entry] : snapshot.entities) {
            const auto& entity = *entry;

            const auto render = [&](const std::vector<Line>& lines, ImU32 color) {
                bool labelled = false;

                for (const auto& line : lines) {
                    if (++line_count > 100000) {
                        return;
                    }

                    std::array<double, 4> screen{};

                    if (geometry::project(line, snapshot.camera, size.x, size.y, screen)) {
                        const ImVec2 a(static_cast<float>(screen[0]), static_cast<float>(screen[1]));
                        const ImVec2 b(static_cast<float>(screen[2]), static_cast<float>(screen[3]));
                        draw->AddLine(a, b, color, 1.0f);

                        if (labels_enabled && !labelled && a.x >= 0 && a.y >= 0 && a.x < size.x && a.y < size.y) {
                            draw->AddText(a, color, entity.name.c_str());
                            labelled = true;
                        }
                    }
                }
            };

            if (bounds_enabled) {
                render(entity.bounds, IM_COL32(80, 225, 235, 170));
            }

            if (shapes_enabled) {
                render(entity.shapes, IM_COL32(250, 194, 95, 210));
            }
        }
    }
} // namespace internal::entity_viewer
