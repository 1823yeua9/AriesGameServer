#pragma once

#include "graphics.h"
#include "../loader/src/init/runtime.h"
#include <optional>

namespace launcher {
    class Panel {
    public:
        Panel(HWND window, ID3D11Device* device, ID3D11DeviceContext* context);

        ~Panel();

        auto draw(ID3D11RenderTargetView* target) -> void;

        auto running() const -> bool;

        auto stop() -> void;

    private:
        auto home() -> void;

        auto settings() -> void;

        auto logs() -> void;

        auto login() -> void;

        auto title_bar() -> void;

        auto launch_bar() -> void;

        auto phase() const -> float;

        auto start(const std::wstring& mode) -> void;

        auto poll() -> void;

        auto browse_config() -> void;

        auto install_containment() -> void;

        auto load_username() -> void;

        auto save_username() -> bool;

        HWND window;
        ComPtr<ID3D11DeviceContext> context;
        std::filesystem::path root;
        std::filesystem::path config_path;
        std::filesystem::path loader_path;
        std::filesystem::path log_path;
        std::optional<loader::init::Job> job;
        std::optional<loader::init::Process> process;
        std::string status = "Choose a local profile, then launch.";
        std::string log_text;
        std::wstring current_mode;
        ULONGLONG last_poll = 0;
        // 0 login, 1 profile, 2 settings, 3 logs (spt: login view, profile view, settings)
        int page = 0;
        int back_page = 0;
        bool preflight_ok = false;
        // refreshed with the log poll, not stat()ed every frame
        bool profile_found = false;
        bool loader_found = false;
        char username[129] = "OfflinePlayer";
    };
} // namespace launcher
