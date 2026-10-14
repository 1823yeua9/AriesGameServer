#include "settings.h"
#include <filesystem>
#include <fstream>
#include <system_error>
#include <Windows.h>
#include <ShlObj.h>
#include <json/json.hpp>

namespace loader::config {
    namespace fs = std::filesystem;

    // No cwd fallback: a failed profile lookup must not silently use another account.
    static auto resolve_settings_path() -> fs::path {
        PWSTR appdata = nullptr;

        if (FAILED(SHGetKnownFolderPath(FOLDERID_RoamingAppData, 0, nullptr, &appdata))) {
            throw std::runtime_error("Cannot resolve roaming AppData");
        }

        fs::path path = appdata;
        CoTaskMemFree(appdata);
        path /= L"Aries";

        return path / settings_file_name;
    }

    auto Settings::load_settings() -> bool {
        auto path = file_path.empty() ? resolve_settings_path() : fs::absolute(file_path);

        if (!fs::exists(path)) {
            return false;
        }

        std::ifstream in(path);

        if (!in) {
            return false;
        }

        try {
            nlohmann::json j;
            in >> j;

            if (!j.is_object()) {
                return false;
            }

            auto candidate = *this;
            candidate.file_path = path;

            for (const auto& [key, value] : j.items()) {
                candidate.set_setting(key, value.get<std::string>());
            }

            *this = std::move(candidate);
        }
        catch (...) {
            return false; // bad json / cast; let caller re-prompt
        }

        return true;
    }

    auto Settings::save_settings() -> bool {
        auto path = file_path.empty() ? resolve_settings_path() : fs::absolute(file_path);

        std::error_code ec;
        fs::create_directories(path.parent_path(), ec);

        if (ec) {
            return false;
        }

        auto temporary = path;
        temporary += L".tmp." + std::to_wstring(GetCurrentProcessId());

        std::ofstream out(temporary);

        if (!out) {
            return false;
        }

        nlohmann::json j = {
            {"game_path", game_path},
            {"game_settings_path", game_settings_path},
            {"host_ip", host_ip},
        };

        for (const auto& [key, value] : launch_settings) {
            j[key] = value;
        }

        out << j.dump(2);
        out.close();

        if (!out || !MoveFileExW(temporary.c_str(), path.c_str(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) {
            fs::remove(temporary, ec);

            return false;
        }

        file_path = path;

        return true;
    }

    auto Settings::path_for(const std::string& key) const -> fs::path {
        const auto& text = get_setting(key);
        auto path = fs::path(std::u8string(text.begin(), text.end()));

        if (path.empty()) {
            return {};
        }

        if (path.is_relative()) {
            path = (file_path.empty() ? resolve_settings_path() : fs::absolute(file_path)).parent_path() / path;
        }

        return fs::absolute(path).lexically_normal();
    }
} // namespace loader::config
