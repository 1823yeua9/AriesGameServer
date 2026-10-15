#include "sdk_reflect.hpp"

#include <cstdarg>
#include <cstdio>
#include <cstring>

#include "../../../dumper7/output/5.3.2-5590001+++Ares-Core+release-13.06-ShooterGame/CppSDK/SDK/ShooterGame_classes.hpp"
#include "../../../dumper7/valorant_sdk_adapter.hpp"

namespace sdk_reflect {
    static char g_last_status[192] = "ready";

    static bool is_readable(const void* value, std::size_t size) {
        return valorant_sdk_adapter::is_readable(value, size);
    }

    static bool is_a(const SDK::UObject* object, const char* class_name) {
        if (!is_readable(object, sizeof(SDK::UObject)) || !class_name) {
            return false;
        }
        auto* current_class = object->Class;
        for (int depth = 0; depth < 32 && is_readable(current_class, sizeof(SDK::UStruct)); ++depth) {
            if (current_class->Name.GetRawString() == class_name) {
                return true;
            }
            current_class = reinterpret_cast<SDK::UClass*>(current_class->SuperStruct);
        }
        return false;
    }

    static const SDK::FProperty* find_property(const SDK::UObject* object, const char* property_name) {
        if (!is_readable(object, sizeof(SDK::UObject)) || !property_name) {
            return nullptr;
        }
        auto* current_class = static_cast<SDK::UStruct*>(object->Class);
        for (int depth = 0; depth < 32 && is_readable(current_class, sizeof(SDK::UStruct)); ++depth) {
            auto* field = current_class->ChildProperties;
            for (int count = 0; count < 2048 && is_readable(field, sizeof(SDK::FProperty)); ++count) {
                if (field->Name.GetRawString() == property_name) {
                    return reinterpret_cast<const SDK::FProperty*>(field);
                }
                field = field->Next;
            }
            current_class = current_class->SuperStruct;
        }
        return nullptr;
    }

    static SDK::UObject* get_object_property(const SDK::UObject* object, const char* property_name) {
        const auto* property = find_property(object, property_name);
        if (!property || !is_readable(object->Class, sizeof(SDK::UStruct))) {
            return nullptr;
        }
        if (!is_readable(property->ClassPrivate, sizeof(SDK::FFieldClass))) {
            return nullptr;
        }
        const std::string property_type = property->ClassPrivate->Name.GetRawString();
        if (property_type != "ObjectProperty" && property_type != "ClassProperty") {
            return nullptr;
        }
        const int offset = property->Offset;
        const int object_size = object->Class->Size;
        if (offset < 0 || object_size <= 0 || object_size > 0x200000 ||
            offset > object_size - static_cast<int>(sizeof(SDK::UObject*))) {
            return nullptr;
        }
        const auto* storage = reinterpret_cast<const unsigned char*>(object) + offset;
        if (!is_readable(storage, sizeof(SDK::UObject*))) {
            return nullptr;
        }
        auto* value = *reinterpret_cast<SDK::UObject* const*>(storage);
        return is_readable(value, sizeof(SDK::UObject)) ? value : nullptr;
    }

    bool ensure_ready() {
        return valorant_sdk_adapter::initialize_gobjects();
    }

    SDK::UObject* find_object(const char* name, const char* owner_name) {
        if (!name || !ensure_ready()) {
            return nullptr;
        }
        auto& objects = SDK::UObject::GObjects;
        const int count = objects->Num();
        for (int index = 0; index < count; ++index) {
            auto* object = objects->GetByIndex(index);
            if (!is_readable(object, sizeof(SDK::UObject)) || object->Name.GetRawString() != name) {
                continue;
            }
            if (owner_name && (!is_readable(object->Outer, sizeof(SDK::UObject)) ||
                               object->Outer->Name.GetRawString() != owner_name)) {
                continue;
            }
            return object;
        }
        return nullptr;
    }

    SDK::UClass* find_class(const char* name) {
        if (!name || !ensure_ready()) {
            return nullptr;
        }
        auto& objects = SDK::UObject::GObjects;
        const int count = objects->Num();
        for (int index = 0; index < count; ++index) {
            auto* object = objects->GetByIndex(index);
            if (is_a(object, "Class") && object->Name.GetRawString() == name) {
                return reinterpret_cast<SDK::UClass*>(object);
            }
        }
        return nullptr;
    }

    SDK::UFunction* find_function(const char* owner_class, const char* function_name) {
        if (!owner_class || !function_name || !ensure_ready()) {
            return nullptr;
        }
        auto& objects = SDK::UObject::GObjects;
        const int count = objects->Num();
        for (int index = 0; index < count; ++index) {
            auto* object = objects->GetByIndex(index);
            if (!is_a(object, "Function") || object->Name.GetRawString() != function_name) {
                continue;
            }
            if (is_readable(object->Outer, sizeof(SDK::UObject)) && object->Outer->Name.GetRawString() == owner_class) {
                return reinterpret_cast<SDK::UFunction*>(object);
            }
        }
        return nullptr;
    }

    SDK::UObject* find_by_path(const char* package_path) {
        if (!package_path || !ensure_ready()) {
            return nullptr;
        }
        auto& objects = SDK::UObject::GObjects;
        const int count = objects->Num();
        for (int index = 0; index < count; ++index) {
            auto* object = objects->GetByIndex(index);
            if (!is_readable(object, sizeof(SDK::UObject))) {
                continue;
            }
            std::string path = object->Name.GetRawString();
            auto* outer = object->Outer;
            for (int depth = 0; depth < 16 && is_readable(outer, sizeof(SDK::UObject)); ++depth) {
                path = outer->Name.GetRawString() + "." + path;
                outer = outer->Outer;
            }
            if (path == package_path) {
                return object;
            }
        }
        return nullptr;
    }

    SDK::UObject* object_property(const SDK::UObject* object, const char* property_name) {
        return get_object_property(object, property_name);
    }

    SDK::UObject* load_object(const char* package_path) {
        auto* object = find_by_path(package_path);
        set_last_status(object ? "found loaded asset" : "asset not loaded; async loading is not implemented");
        return object;
    }

    SDK::UEngine* engine() {
        if (!ensure_ready()) {
            return nullptr;
        }
        auto& objects = SDK::UObject::GObjects;
        const int count = objects->Num();
        for (int index = 0; index < count; ++index) {
            auto* object = objects->GetByIndex(index);
            if (!is_a(object, "AresGameEngine") || object->Name.GetRawString().starts_with("Default__")) {
                continue;
            }
            if (get_object_property(object, "GameViewport")) {
                return reinterpret_cast<SDK::UEngine*>(object);
            }
        }
        return nullptr;
    }

    SDK::UWorld* world() {
        auto* game_engine = engine();
        auto* viewport = get_object_property(reinterpret_cast<SDK::UObject*>(game_engine), "GameViewport");
        auto* current_world = get_object_property(viewport, "World");
        return is_a(current_world, "World") ? reinterpret_cast<SDK::UWorld*>(current_world) : nullptr;
    }

    SDK::UObject* local_player_controller() {
        if (!ensure_ready()) {
            return nullptr;
        }
        auto& objects = SDK::UObject::GObjects;
        const int count = objects->Num();
        for (int index = 0; index < count; ++index) {
            auto* object = objects->GetByIndex(index);
            if (!is_a(object, "LocalPlayer") || object->Name.GetRawString().starts_with("Default__")) {
                continue;
            }
            auto* controller = get_object_property(object, "PlayerController");
            if (is_a(controller, "PlayerController")) {
                return controller;
            }
        }
        return nullptr;
    }

    SDK::UObject* local_player_pawn() {
        return get_object_property(local_player_controller(), "Pawn");
    }

    bool invoke(SDK::UObject* target, const char* owner_class, const char* function_name, void* params) {
        if (!is_readable(target, sizeof(SDK::UObject))) {
            return false;
        }
        auto* function = find_function(owner_class, function_name);
        if (!function) {
            return false;
        }
        target->ProcessEvent(function, params);
        return true;
    }

    bool invoke_static(const char* owner_class, const char* function_name, void* params) {
        if (!owner_class) {
            return false;
        }
        const std::string default_name = std::string("Default__") + owner_class;
        return invoke(find_object(default_name.c_str()), owner_class, function_name, params);
    }

    const char* last_status() {
        return g_last_status;
    }

    void set_last_status(const char* format, ...) {
        if (!format) {
            return;
        }
        va_list arguments;
        va_start(arguments, format);
        std::vsnprintf(g_last_status, sizeof(g_last_status), format, arguments);
        va_end(arguments);
    }
} // namespace sdk_reflect
