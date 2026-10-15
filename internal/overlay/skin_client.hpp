#pragma once

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <cstdint>
#include <string>
#include <vector>

namespace skin_client {
    struct SkinRecord {
        std::string weapon_id;
        std::string skin_id;
        std::string name;
        bool owned = false;
    };

    struct EquippedRecord {
        std::string weapon_id;
        std::string skin_id;
    };

    struct SkinState {
        bool connected = false;
        bool request_pending = false;
        std::uint64_t version = 0;
        std::string status;
        std::string last_action_error;
        std::vector<SkinRecord> available;
        std::vector<EquippedRecord> equipped;
    };

    void initialize(HMODULE module);
    void shutdown();
    SkinState snapshot();
    void request_refresh();
    bool request_grant(const std::string& skin_id);
    bool request_grant_all();
    bool request_equip(const std::string& weapon_id, const std::string& skin_id);
} // namespace skin_client
