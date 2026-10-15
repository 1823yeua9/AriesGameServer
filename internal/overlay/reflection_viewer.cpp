#include "reflection_viewer.hpp"
#include "sdk_reflect.hpp"

#include <algorithm>
#include <cctype>
#include <cstdio>
#include <cstdint>
#include <string>
#include <utility>
#include <vector>

#include "vendor/imgui/imgui.h"
#include "../../../dumper7/output/5.3.2-5590001+++Ares-Core+release-13.06-ShooterGame/CppSDK/SDK/CoreUObject_classes.hpp"
#include "../../../dumper7/valorant_sdk_adapter.hpp"

namespace reflection_viewer {
    struct SearchResult {
        int object_index;
        std::string object_name;
        std::string class_name;
    };

    struct ParameterSummary {
        std::string name;
        std::string type;
        std::uint64_t flags;
        int offset;
    };

    struct FunctionSummary {
        std::string name;
        std::string declaring_class;
        std::string search_text;
        std::uint32_t flags;
        std::vector<ParameterSummary> parameters;
    };

    static char g_query[128]{};
    static std::vector<SearchResult> g_results;
    static int g_next_index = 0;
    static int g_total_objects = 0;
    static int g_selected_index = -1;
    static std::string g_selected_name;
    static bool g_scanning = false;
    static std::string g_status = "Search a reflected object name or class.";
    static std::string g_world_status = "Press Refresh to inspect the live world.";
    static std::vector<FunctionSummary> g_function_summaries;
    static const SDK::UObject* g_function_cache_object = nullptr;
    static const SDK::UClass* g_function_cache_class = nullptr;
    static bool g_function_cache_dirty = true;
    static bool g_function_cache_truncated = false;
    static char g_function_query[128]{};
    static int g_function_page = 0;

    static bool is_readable(const void* value, std::size_t size) {
        return valorant_sdk_adapter::is_readable(value, size);
    }

    static std::string lower_case(std::string value) {
        std::transform(value.begin(), value.end(), value.begin(), [](unsigned char character) {
            return static_cast<char>(std::tolower(character));
        });
        return value;
    }

    static void select_object(int object_index, const std::string& object_name) {
        g_selected_index = object_index;
        g_selected_name = object_name;
        g_function_cache_dirty = true;
        g_function_page = 0;
    }

    static void select_live_object(SDK::UObject* object) {
        if (!is_readable(object, sizeof(SDK::UObject)) || !SDK::UObject::GObjects || object->Index < 0 ||
            object->Index >= SDK::UObject::GObjects->Num() ||
            SDK::UObject::GObjects->GetByIndex(object->Index) != object) {
            g_status = "Target is not a live reflected object.";
            return;
        }
        select_object(object->Index, object->Name.GetRawString());
        g_status = "Selected " + object->Name.GetRawString();
    }

    static bool is_function_field(const SDK::UField* field) {
        if (!is_readable(field, sizeof(SDK::UFunction))) {
            return false;
        }
        auto* current_class = field->Class;
        for (int depth = 0; depth < 16 && is_readable(current_class, sizeof(SDK::UStruct)); ++depth) {
            if (current_class->Name.GetRawString() == "Function") {
                return true;
            }
            current_class = reinterpret_cast<SDK::UClass*>(current_class->SuperStruct);
        }
        return false;
    }

    static std::string function_flags_text(std::uint32_t flags) {
        char value[16]{};
        std::snprintf(value, sizeof(value), "0x%08X", flags);
        std::string text = value;
        if (flags & static_cast<std::uint32_t>(SDK::EFunctionFlags::Native)) {
            text += " Native";
        }
        if (flags & static_cast<std::uint32_t>(SDK::EFunctionFlags::Event)) {
            text += " Event";
        }
        if (flags & static_cast<std::uint32_t>(SDK::EFunctionFlags::NetClient)) {
            text += " Client";
        }
        if (flags & static_cast<std::uint32_t>(SDK::EFunctionFlags::NetServer)) {
            text += " Server";
        }
        if (flags & static_cast<std::uint32_t>(SDK::EFunctionFlags::NetMulticast)) {
            text += " Multicast";
        }
        if (flags & static_cast<std::uint32_t>(SDK::EFunctionFlags::NetReliable)) {
            text += " Reliable";
        }
        if (flags & static_cast<std::uint32_t>(SDK::EFunctionFlags::BlueprintCallable)) {
            text += " Callable";
        }
        return text;
    }

    static std::string parameter_flags_text(std::uint64_t flags) {
        char value[24]{};
        std::snprintf(value, sizeof(value), "0x%llX", static_cast<unsigned long long>(flags));
        std::string text = value;
        if (flags & static_cast<std::uint64_t>(SDK::EPropertyFlags::OutParm)) {
            text += " Out";
        }
        if (flags & static_cast<std::uint64_t>(SDK::EPropertyFlags::ReturnParm)) {
            text += " Return";
        }
        if (flags & static_cast<std::uint64_t>(SDK::EPropertyFlags::ConstParm)) {
            text += " Const";
        }
        if (flags & static_cast<std::uint64_t>(SDK::EPropertyFlags::ReferenceParm)) {
            text += " Ref";
        }
        return text;
    }

    static void rebuild_function_cache(const SDK::UObject* object) {
        g_function_summaries.clear();
        g_function_cache_object = object;
        g_function_cache_class = object->Class;
        g_function_cache_dirty = false;
        g_function_cache_truncated = false;
        constexpr int max_functions = 2048;
        constexpr int max_children_per_class = 4096;
        constexpr int max_parameters_per_function = 128;
        auto* current_class = static_cast<SDK::UStruct*>(object->Class);
        for (int depth = 0; depth < 16 && is_readable(current_class, sizeof(SDK::UStruct));
             current_class = current_class->SuperStruct, ++depth) {
            const std::string declaring_class = current_class->Name.GetRawString();
            auto* field = current_class->Children;
            for (int count = 0; field && count < max_children_per_class; ++count) {
                if (!is_readable(field, sizeof(SDK::UField))) {
                    g_function_cache_truncated = true;
                    break;
                }
                auto* next_field = field->Next;
                if (is_function_field(field)) {
                    if (g_function_summaries.size() >= max_functions) {
                        g_function_cache_truncated = true;
                        return;
                    }
                    auto* function = static_cast<SDK::UFunction*>(field);
                    FunctionSummary summary{};
                    summary.name = function->Name.GetRawString();
                    summary.declaring_class = declaring_class;
                    summary.search_text = lower_case(summary.declaring_class + "." + summary.name);
                    summary.flags = static_cast<std::uint32_t>(function->FunctionFlags);
                    auto* parameter_field = function->ChildProperties;
                    for (int parameter_count = 0; parameter_field && parameter_count < max_parameters_per_function;
                         ++parameter_count) {
                        if (!is_readable(parameter_field, sizeof(SDK::FProperty))) {
                            g_function_cache_truncated = true;
                            break;
                        }
                        auto* property = static_cast<SDK::FProperty*>(parameter_field);
                        if (property->PropertyFlags & static_cast<std::uint64_t>(SDK::EPropertyFlags::Parm)) {
                            ParameterSummary parameter{};
                            parameter.name = property->Name.GetRawString();
                            parameter.type = is_readable(property->ClassPrivate, sizeof(SDK::FFieldClass))
                                                 ? property->ClassPrivate->Name.GetRawString()
                                                 : "unknown";
                            parameter.flags = property->PropertyFlags;
                            parameter.offset = property->Offset;
                            summary.parameters.push_back(std::move(parameter));
                        }
                        parameter_field = parameter_field->Next;
                    }
                    if (parameter_field) {
                        g_function_cache_truncated = true;
                    }
                    g_function_summaries.push_back(std::move(summary));
                }
                field = next_field;
                if (field && count + 1 == max_children_per_class) {
                    g_function_cache_truncated = true;
                }
            }
        }
    }

    static void draw_function_inspector(const SDK::UObject* object) {
        if (g_function_cache_dirty || g_function_cache_object != object || g_function_cache_class != object->Class) {
            rebuild_function_cache(object);
        }
        if (!ImGui::CollapsingHeader("Functions (read-only)")) {
            return;
        }
        ImGui::InputText("Function filter", g_function_query, sizeof(g_function_query));
        const std::string query = lower_case(g_function_query);
        std::vector<std::size_t> matches;
        matches.reserve(g_function_summaries.size());
        for (std::size_t index = 0; index < g_function_summaries.size(); ++index) {
            if (g_function_summaries[index].search_text.find(query) != std::string::npos) {
                matches.push_back(index);
            }
        }
        constexpr int page_size = 12;
        const int page_count = static_cast<int>((matches.size() + page_size - 1) / page_size);
        if (g_function_page >= page_count) {
            g_function_page = (std::max)(0, page_count - 1);
        }
        if (ImGui::Button("Previous function page") && g_function_page > 0) {
            --g_function_page;
        }
        ImGui::SameLine();
        if (ImGui::Button("Next function page") && g_function_page + 1 < page_count) {
            ++g_function_page;
        }
        ImGui::Text("%zu matching functions, page %d/%d%s",
                    matches.size(),
                    g_function_page + 1,
                    (std::max)(1, page_count),
                    g_function_cache_truncated ? " (bounded scan)" : "");
        const int start = g_function_page * page_size;
        const int end = (std::min)(start + page_size, static_cast<int>(matches.size()));
        if (ImGui::BeginChild("function_results", ImVec2(0, 200), true)) {
            for (int match_index = start; match_index < end; ++match_index) {
                const auto& function = g_function_summaries[matches[match_index]];
                ImGui::PushID(static_cast<int>(matches[match_index]));
                if (ImGui::TreeNode("function",
                                    "%s.%s (%zu params)",
                                    function.declaring_class.c_str(),
                                    function.name.c_str(),
                                    function.parameters.size())) {
                    ImGui::Text("Flags: %s", function_flags_text(function.flags).c_str());
                    for (const auto& parameter : function.parameters) {
                        ImGui::Text("%s : %s +0x%X  [%s]",
                                    parameter.name.c_str(),
                                    parameter.type.c_str(),
                                    parameter.offset,
                                    parameter_flags_text(parameter.flags).c_str());
                    }
                    ImGui::TreePop();
                }
                ImGui::PopID();
            }
        }
        ImGui::EndChild();
    }

    static void begin_search() {
        g_results.clear();
        g_selected_index = -1;
        g_selected_name.clear();
        g_next_index = 0;
        g_total_objects = 0;
        if (g_query[0] == '\0') {
            g_scanning = false;
            g_status = "Enter a name or class fragment first.";
            return;
        }
        if (!valorant_sdk_adapter::initialize_gobjects() || !SDK::UObject::GObjects) {
            g_scanning = false;
            g_status = "GObjects is unavailable.";
            return;
        }
        g_total_objects = SDK::UObject::GObjects->Num();
        g_scanning = true;
        g_status = "Scanning reflected objects...";
    }

    static std::string
    read_property_value(const SDK::UObject* object, const SDK::FProperty* property, const std::string& type) {
        const int object_size = object->Class->Size;
        const int offset = property->Offset;
        if (offset < 0 || object_size <= 0 || object_size > 0x200000 || offset > object_size ||
            property->ElementSize <= 0 || property->ElementSize > object_size - offset) {
            return {};
        }
        const auto* storage = reinterpret_cast<const unsigned char*>(object) + offset;
        if (type == "ObjectProperty" || type == "ClassProperty") {
            if (property->ArrayDim > 1 && property->ArrayDim <= 1024 &&
                property->ElementSize == sizeof(SDK::UObject*)) {
                const auto array_size = static_cast<std::size_t>(property->ArrayDim) * sizeof(SDK::UObject*);
                if (array_size > static_cast<std::size_t>(object_size - offset) || !is_readable(storage, array_size)) {
                    return {};
                }
                const auto* targets = reinterpret_cast<SDK::UObject* const*>(storage);
                int populated = 0;
                for (int index = 0; index < property->ArrayDim; ++index) {
                    if (is_readable(targets[index], sizeof(SDK::UObject))) {
                        ++populated;
                    }
                }
                return std::to_string(populated) + "/" + std::to_string(property->ArrayDim) + " objects";
            }
            if (!is_readable(storage, sizeof(SDK::UObject*))) {
                return {};
            }
            auto* target = *reinterpret_cast<SDK::UObject* const*>(storage);
            if (!target) {
                return "null";
            }
            if (!is_readable(target, sizeof(SDK::UObject))) {
                return "unreadable object";
            }
            return target->Name.GetRawString();
        }
        if (type == "IntProperty") {
            if (!is_readable(storage, sizeof(std::int32_t))) {
                return {};
            }
            return std::to_string(*reinterpret_cast<const std::int32_t*>(storage));
        }
        if ((type == "ByteProperty" || type == "EnumProperty") && property->ElementSize == 1) {
            if (!is_readable(storage, 1)) {
                return {};
            }
            return std::to_string(*storage);
        }
        if (type == "Int64Property") {
            if (!is_readable(storage, sizeof(std::int64_t))) {
                return {};
            }
            return std::to_string(*reinterpret_cast<const std::int64_t*>(storage));
        }
        if (type == "FloatProperty") {
            if (!is_readable(storage, sizeof(float))) {
                return {};
            }
            char text[64]{};
            std::snprintf(text, sizeof(text), "%.4f", *reinterpret_cast<const float*>(storage));
            return text;
        }
        if (type == "BoolProperty") {
            if (!is_readable(property, sizeof(SDK::FBoolProperty))) {
                return {};
            }
            const auto* boolean_property = static_cast<const SDK::FBoolProperty*>(property);
            if (boolean_property->ByteOffset >= property->ElementSize ||
                !is_readable(storage + boolean_property->ByteOffset, 1)) {
                return {};
            }
            return (storage[boolean_property->ByteOffset] & boolean_property->ByteMask) ? "true" : "false";
        }
        if (type == "NameProperty") {
            if (!is_readable(storage, sizeof(SDK::FName))) {
                return {};
            }
            return reinterpret_cast<const SDK::FName*>(storage)->GetRawString();
        }
        if (type == "ArrayProperty") {
            if (property->ElementSize < 16 || !is_readable(storage, 16)) {
                return {};
            }
            const int count = *reinterpret_cast<const std::int32_t*>(storage + 8);
            const int capacity = *reinterpret_cast<const std::int32_t*>(storage + 12);
            if (count < 0 || capacity < count || capacity > 1000000) {
                return "invalid array header";
            }
            return std::to_string(count) + " elements";
        }
        if (type == "StructProperty" && property->ElementSize >= 16 &&
            is_readable(property, sizeof(SDK::FStructProperty))) {
            const auto* struct_property = static_cast<const SDK::FStructProperty*>(property);
            if (is_readable(struct_property->Struct, sizeof(SDK::UStruct)) &&
                struct_property->Struct->Name.GetRawString() == "Guid" && is_readable(storage, 16)) {
                const auto* parts = reinterpret_cast<const std::uint32_t*>(storage);
                char text[64]{};
                std::snprintf(text, sizeof(text), "%08X-%08X-%08X-%08X", parts[0], parts[1], parts[2], parts[3]);
                return text;
            }
        }
        return {};
    }

    static SDK::UObject*
    read_object_target(const SDK::UObject* object, const SDK::FProperty* property, const std::string& type) {
        if (type != "ObjectProperty" && type != "ClassProperty") {
            return nullptr;
        }
        if (property->ArrayDim != 1) {
            return nullptr;
        }
        const int object_size = object->Class->Size;
        const int offset = property->Offset;
        if (offset < 0 || object_size <= 0 || object_size > 0x200000 ||
            offset > object_size - static_cast<int>(sizeof(SDK::UObject*))) {
            return nullptr;
        }
        const auto* storage = reinterpret_cast<const unsigned char*>(object) + offset;
        if (!is_readable(storage, sizeof(SDK::UObject*))) {
            return nullptr;
        }
        auto* target = *reinterpret_cast<SDK::UObject* const*>(storage);
        return is_readable(target, sizeof(SDK::UObject)) ? target : nullptr;
    }

    static void scan_batch() {
        if (!g_scanning) {
            return;
        }
        const std::string query = lower_case(g_query);
        auto& objects = SDK::UObject::GObjects;
        const int end_index = (std::min)(g_next_index + 512, g_total_objects);
        for (; g_next_index < end_index; ++g_next_index) {
            auto* object = objects->GetByIndex(g_next_index);
            if (!is_readable(object, sizeof(SDK::UObject)) || !is_readable(object->Class, sizeof(SDK::UClass))) {
                continue;
            }
            const std::string object_name = object->Name.GetRawString();
            const std::string class_name = object->Class->Name.GetRawString();
            if (lower_case(object_name).find(query) == std::string::npos &&
                lower_case(class_name).find(query) == std::string::npos) {
                continue;
            }
            g_results.push_back({g_next_index, object_name, class_name});
            if (g_results.size() >= 256) {
                g_scanning = false;
                g_status = "First 256 matches shown; refine search for more.";
                return;
            }
        }
        if (g_next_index >= g_total_objects) {
            g_scanning = false;
            g_status = "Search complete.";
        }
    }

    static void draw_selected_object() {
        if (g_selected_index < 0 || !SDK::UObject::GObjects) {
            return;
        }
        auto* object = SDK::UObject::GObjects->GetByIndex(g_selected_index);
        if (!is_readable(object, sizeof(SDK::UObject)) || !is_readable(object->Class, sizeof(SDK::UClass))) {
            ImGui::TextDisabled("Selected object is no longer live.");
            return;
        }
        if (object->Name.GetRawString() != g_selected_name) {
            ImGui::TextDisabled("Selected object index was reused; search again.");
            return;
        }

        ImGui::Separator();
        ImGui::Text("Object: %s", object->Name.GetRawString().c_str());
        ImGui::Text("Class: %s", object->Class->Name.GetRawString().c_str());
        ImGui::Text("Index: %d", object->Index);
        if (is_readable(object->Outer, sizeof(SDK::UObject))) {
            ImGui::Text("Outer: %s", object->Outer->Name.GetRawString().c_str());
        }
        ImGui::TextDisabled("Read-only reflected properties; no values are changed.");
        draw_function_inspector(object);

        auto* current_class = static_cast<SDK::UStruct*>(object->Class);
        for (int depth = 0; is_readable(current_class, sizeof(SDK::UStruct)) && depth < 16;
             current_class = current_class->SuperStruct, ++depth) {
            const std::string class_name = current_class->Name.GetRawString();
            if (!ImGui::TreeNode(reinterpret_cast<void*>(current_class), "%s", class_name.c_str())) {
                continue;
            }
            auto* field = current_class->ChildProperties;
            for (int count = 0; field && count < 512; ++count) {
                if (!is_readable(field, sizeof(SDK::FProperty))) {
                    ImGui::TextDisabled("Unreadable property link");
                    break;
                }
                auto* property = reinterpret_cast<SDK::FProperty*>(field);
                const std::string name = field->Name.GetRawString();
                const std::string type = is_readable(field->ClassPrivate, sizeof(SDK::FFieldClass))
                                             ? field->ClassPrivate->Name.GetRawString()
                                             : "unknown";
                const std::string value = read_property_value(object, property, type);
                ImGui::Text("%s : %s  +0x%X (%d)%s%s",
                            name.c_str(),
                            type.c_str(),
                            property->Offset,
                            property->ElementSize,
                            value.empty() ? "" : " = ",
                            value.c_str());
                auto* target = read_object_target(object, property, type);
                if (target && target->Index >= 0 && target->Index < SDK::UObject::GObjects->Num() &&
                    SDK::UObject::GObjects->GetByIndex(target->Index) == target) {
                    ImGui::SameLine();
                    ImGui::PushID(field);
                    if (ImGui::SmallButton("Inspect")) {
                        select_object(target->Index, target->Name.GetRawString());
                    }
                    ImGui::PopID();
                }
                if ((type == "ObjectProperty" || type == "ClassProperty") && property->ArrayDim > 1 &&
                    property->ArrayDim <= 64 && property->ElementSize == sizeof(SDK::UObject*) &&
                    property->Offset >= 0 &&
                    property->Offset <=
                        object->Class->Size - property->ArrayDim * static_cast<int>(sizeof(SDK::UObject*))) {
                    const auto* array = reinterpret_cast<SDK::UObject* const*>(
                        reinterpret_cast<const unsigned char*>(object) + property->Offset);
                    if (is_readable(array, property->ArrayDim * sizeof(SDK::UObject*)) &&
                        ImGui::TreeNode(field, "Elements")) {
                        for (int index = 0; index < property->ArrayDim; ++index) {
                            auto* element = array[index];
                            if (!is_readable(element, sizeof(SDK::UObject))) {
                                continue;
                            }
                            ImGui::PushID(index);
                            ImGui::Text("[%d] %s", index, element->Name.GetRawString().c_str());
                            if (element->Index >= 0 && element->Index < SDK::UObject::GObjects->Num() &&
                                SDK::UObject::GObjects->GetByIndex(element->Index) == element) {
                                ImGui::SameLine();
                                if (ImGui::SmallButton("Inspect")) {
                                    select_object(element->Index, element->Name.GetRawString());
                                }
                            }
                            ImGui::PopID();
                        }
                        ImGui::TreePop();
                    }
                }
                field = field->Next;
            }
            ImGui::TreePop();
        }
    }

    static std::string object_label(const SDK::UObject* object) {
        if (!is_readable(object, sizeof(SDK::UObject)) || !is_readable(object->Class, sizeof(SDK::UClass))) {
            return "null";
        }
        return object->Name.GetRawString() + " [" + object->Class->Name.GetRawString() + "]";
    }

    static std::string object_path(const SDK::UObject* object) {
        if (!is_readable(object, sizeof(SDK::UObject))) {
            return "unavailable";
        }
        std::string path = object->Name.GetRawString();
        auto* outer = object->Outer;
        for (int depth = 0; depth < 16 && is_readable(outer, sizeof(SDK::UObject)); ++depth) {
            path = outer->Name.GetRawString() + "." + path;
            outer = outer->Outer;
        }
        return path;
    }

    static std::string property_value(const SDK::UObject* object, const char* property_name) {
        if (!is_readable(object, sizeof(SDK::UObject)) || !is_readable(object->Class, sizeof(SDK::UStruct))) {
            return "unavailable";
        }
        auto* current_class = static_cast<SDK::UStruct*>(object->Class);
        for (int depth = 0; depth < 32 && is_readable(current_class, sizeof(SDK::UStruct));
             current_class = current_class->SuperStruct, ++depth) {
            auto* field = current_class->ChildProperties;
            for (int count = 0; count < 2048 && is_readable(field, sizeof(SDK::FProperty));
                 field = field->Next, ++count) {
                if (field->Name.GetRawString() != property_name) {
                    continue;
                }
                if (!is_readable(field->ClassPrivate, sizeof(SDK::FFieldClass))) {
                    return "unavailable";
                }
                const std::string type = field->ClassPrivate->Name.GetRawString();
                const std::string value =
                    read_property_value(object, reinterpret_cast<const SDK::FProperty*>(field), type);
                return value.empty() ? "not displayed" : value;
            }
        }
        return "not reflected";
    }

    static void append_inventory_slot_status(const SDK::UObject* inventory) {
        if (!is_readable(inventory, sizeof(SDK::UObject)) || !is_readable(inventory->Class, sizeof(SDK::UStruct))) {
            return;
        }
        auto* current_class = static_cast<SDK::UStruct*>(inventory->Class);
        for (int depth = 0; depth < 32 && is_readable(current_class, sizeof(SDK::UStruct));
             current_class = current_class->SuperStruct, ++depth) {
            auto* field = current_class->ChildProperties;
            for (int count = 0; count < 2048 && is_readable(field, sizeof(SDK::FProperty));
                 field = field->Next, ++count) {
                const auto* property = reinterpret_cast<const SDK::FProperty*>(field);
                if (field->Name.GetRawString() != "ItemSlots" || property->ArrayDim < 1 || property->ArrayDim > 64 ||
                    property->ElementSize != sizeof(SDK::UObject*)) {
                    continue;
                }
                const int object_size = inventory->Class->Size;
                const int bytes = property->ArrayDim * static_cast<int>(sizeof(SDK::UObject*));
                if (property->Offset < 0 || bytes > object_size || property->Offset > object_size - bytes) {
                    return;
                }
                const auto* slots = reinterpret_cast<SDK::UObject* const*>(
                    reinterpret_cast<const unsigned char*>(inventory) + property->Offset);
                if (!is_readable(slots, bytes)) {
                    return;
                }
                for (int index = 0; index < property->ArrayDim; ++index) {
                    const auto* slot = slots[index];
                    g_world_status += "\nSlot[" + std::to_string(index) + "]: " + object_label(slot);
                    if (!is_readable(slot, sizeof(SDK::UObject))) {
                        continue;
                    }
                    g_world_status += " type=" + property_value(slot, "SlotType");
                    g_world_status +=
                        " parent=" +
                        std::string(sdk_reflect::object_property(slot, "Inventory") == inventory ? "yes" : "no");
                    g_world_status += " contents=" + property_value(slot, "Contents");
                }
                return;
            }
        }
    }

    static void refresh_world_status() {
        auto* current_world = sdk_reflect::world();
        auto* controller = sdk_reflect::local_player_controller();
        auto* pawn = sdk_reflect::local_player_pawn();
        g_world_status = "World: " + object_label(reinterpret_cast<SDK::UObject*>(current_world));
        g_world_status += "\nGameMode: " + object_label(sdk_reflect::object_property(
                                               reinterpret_cast<SDK::UObject*>(current_world), "AuthorityGameMode"));
        g_world_status +=
            "\nGameState: " +
            object_label(sdk_reflect::object_property(reinterpret_cast<SDK::UObject*>(current_world), "GameState"));
        g_world_status +=
            "\nNetDriver: " +
            object_label(sdk_reflect::object_property(reinterpret_cast<SDK::UObject*>(current_world), "NetDriver"));
        g_world_status += "\nController: " + object_label(controller);
        g_world_status += "\nPawn: " + object_label(pawn);
        auto* player_state = sdk_reflect::object_property(controller, "PlayerState");
        auto* inventory = sdk_reflect::object_property(pawn, "Inventory");
        g_world_status += "\nPlayerState: " + object_label(player_state);
        g_world_status += "\nInventory: " + object_label(inventory);
        g_world_status += "\nItemSlot UClass path: " +
                          object_path(reinterpret_cast<SDK::UObject*>(sdk_reflect::find_class("ItemSlot")));
        g_world_status += "\nWaiting: " + property_value(controller, "bPlayerIsWaiting");
        g_world_status += "\nDesiredClass: " + property_value(player_state, "DesiredClass");
        g_world_status += "\nSpawnedCharacter: " + property_value(player_state, "SpawnedCharacter");
        g_world_status += "\nPossessedCharacter: " + property_value(player_state, "PossessedCharacter");
        g_world_status += "\nItemSlots: " + property_value(inventory, "ItemSlots");
        append_inventory_slot_status(inventory);
        g_world_status += "\nCurrentEquippable: " + property_value(inventory, "CurrentEquippable");
        g_world_status +=
            "\nInitialServerCorrectionProcessed: " + property_value(inventory, "bInitialServerCorrectionProcessed");
    }

    void draw() {
        if (ImGui::Button("Refresh world snapshot")) {
            refresh_world_status();
        }
        ImGui::SameLine();
        if (ImGui::Button("Copy snapshot")) {
            ImGui::SetClipboardText(g_world_status.c_str());
        }
        ImGui::TextUnformatted(g_world_status.c_str());
        if (ImGui::SmallButton("Inspect world")) {
            select_live_object(reinterpret_cast<SDK::UObject*>(sdk_reflect::world()));
        }
        ImGui::SameLine();
        if (ImGui::SmallButton("Controller")) {
            select_live_object(sdk_reflect::local_player_controller());
        }
        ImGui::SameLine();
        if (ImGui::SmallButton("Pawn")) {
            select_live_object(sdk_reflect::local_player_pawn());
        }
        ImGui::SameLine();
        if (ImGui::SmallButton("PlayerState")) {
            select_live_object(sdk_reflect::object_property(sdk_reflect::local_player_controller(), "PlayerState"));
        }
        ImGui::SameLine();
        if (ImGui::SmallButton("Inventory")) {
            select_live_object(sdk_reflect::object_property(sdk_reflect::local_player_pawn(), "Inventory"));
        }
        ImGui::Separator();
        ImGui::InputText("Object / class", g_query, sizeof(g_query));
        ImGui::SameLine();
        if (ImGui::Button("Search")) {
            begin_search();
        }
        scan_batch();
        ImGui::TextDisabled("%s  %d/%d; %d matches",
                            g_status.c_str(),
                            g_next_index,
                            g_total_objects,
                            static_cast<int>(g_results.size()));

        if (ImGui::BeginChild("object_results", ImVec2(0, 150), true)) {
            for (const auto& result : g_results) {
                ImGui::PushID(result.object_index);
                const std::string label = result.object_name + " [" + result.class_name + "]";
                if (ImGui::Selectable(label.c_str(), g_selected_index == result.object_index)) {
                    select_object(result.object_index, result.object_name);
                }
                ImGui::PopID();
            }
        }
        ImGui::EndChild();
        draw_selected_object();
    }
} // namespace reflection_viewer
