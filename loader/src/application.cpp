#include "application.h"
#include "command_line.h"
#include "init/launch_session.h"
#include "init/networking/firewall.h"

#include <iostream>

namespace loader {
    namespace fs = std::filesystem;

    static constexpr auto pinned_hash = "ec6e08c7c3866c7100be4322fa70fe1198185c43e2a89f9d417b4f64b5bab0a2";

    static auto require_file(const fs::path& path, const char* purpose) -> void {
        if (path.empty() || !fs::is_regular_file(path)) {
            throw std::runtime_error(std::string("missing ") + purpose + ": " + path.string());
        }
    }

    static auto write_template(config::Settings& settings, const fs::path& path) -> void {
        if (fs::exists(path)) {
            throw std::runtime_error("refusing to overwrite existing settings");
        }

        settings.set_setting(
            "game_path",
            "../game req/VALORANT_GAME/VALORANT/live/ShooterGame/Binaries/Win64/VALORANT-Win64-Shipping.exe");

        settings.set_setting("game_settings_path", "runtime/account/persistence");

        if (!settings.save_settings()) {
            throw std::runtime_error("cannot write settings template");
        }

        std::cout << "Created settings template. Relative paths are based on its directory.\n";
    }

    static auto parse_port(const std::string& text, bool allow_zero) -> unsigned short {
        if (text.empty() || text.find_first_not_of("0123456789") != std::string::npos) {
            throw std::runtime_error("invalid port setting");
        }

        const auto number = std::stoul(text);

        if (number > 65535 || (!number && !allow_zero)) {
            throw std::runtime_error("port must be 1..65535 (0 allowed for server-only)");
        }

        return static_cast<unsigned short>(number);
    }

    static auto validate_settings(const config::Settings& settings, bool server_only) -> ServicePorts {
        const ServicePorts ports{parse_port(settings.get_setting("port"), server_only),
                                 parse_port(settings.get_setting("admin_port"), server_only)};

        if (ports.platform && ports.platform == ports.admin) {
            throw std::runtime_error("platform and admin ports must differ");
        }

        const auto& host = settings.get_setting("host_ip");

        if (host != "localhost" && host != "127.0.0.1") {
            throw std::runtime_error("this profile supports loopback only");
        }

        if (settings.path_for("runtime_path").empty()) {
            throw std::runtime_error("runtime_path is required");
        }

        if (settings.path_for("skin_state").empty() || settings.path_for("game_settings_path").empty()) {
            throw std::runtime_error("skin_state and game_settings_path are required");
        }

        return ports;
    }

    static auto reporter_path_for(const config::Settings& settings) -> fs::path {
        const auto game_path = settings.path_for("game_path");
        const auto install_path = game_path.parent_path().parent_path().parent_path().parent_path();

        return install_path / L"Engine/Binaries/Win64/CrashReportClient.exe";
    }

    static auto install_firewall(const config::Settings& settings) -> void {
        for (const auto& path : {settings.path_for("game_path"), reporter_path_for(settings)}) {
            require_file(path, "contained executable");

            const auto result = init::networking::contain_loopback(path.wstring(), true);

            if (result != init::networking::FirewallResult::Success) {
                throw std::runtime_error("firewall installation/verification failed; use an elevated terminal, with "
                                         "all firewall profiles enabled");
            }
        }

        std::cout
            << "Verified game and crash-reporter non-loopback block rules. No other programs or services changed.\n";
    }

    static auto verify_service_files(const config::Settings& settings, bool server_only) -> void {
        require_file(settings.path_for("server_path"), "platform executable");

        require_file(settings.path_for("cert_path"), "TLS certificate");
        require_file(settings.path_for("key_path"), "TLS private key");

        require_file(settings.path_for("skin_catalog"), "skin catalog");

        if (!server_only) {
            require_file(settings.path_for("gameplay_path"), "gameplay executable");
            require_file(settings.path_for("gameplay_config"), "gameplay config");
        }
    }

    static auto pin_game_file(const config::Settings& settings) -> init::Handle {
        const auto game_path = settings.path_for("game_path");
        const auto reporter_path = reporter_path_for(settings);

        // Keep the original file open without write/delete sharing until teardown.
        init::Handle file(CreateFileW(
            game_path.c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr));

        require_file(game_path, "game executable");
        require_file(settings.path_for("internal_path"), "internal DLL (oracle/startup module)");
        require_file(reporter_path, "crash reporter");

        if (file.get() == INVALID_HANDLE_VALUE || init::sha256(file.get()) != pinned_hash) {
            throw std::runtime_error("game file is not the pinned original 13.06 build");
        }

        for (const auto& path : {game_path, reporter_path}) {
            const auto result = init::networking::contain_loopback(path.wstring(), false);

            if (result != init::networking::FirewallResult::Success) {
                throw std::runtime_error("containment missing or different; run --install-firewall explicitly");
            }
        }

        return file;
    }

    auto run(int argc, wchar_t** argv) -> int {
        std::cout << std::unitbuf;

        const auto options = parse_command_line(argc, argv);

        if (options.mode == LaunchMode::Help) {
            print_help();

            return 0;
        }

        config::Settings settings(options.config_path);

        if (options.mode == LaunchMode::Template) {
            write_template(settings, options.config_path);

            return 0;
        }

        if (!settings.load_settings()) {
            throw std::runtime_error("missing or invalid settings; use --write-config first");
        }

        const auto server_only = options.mode == LaunchMode::ServerOnly;
        const auto ports = validate_settings(settings, server_only);

        init::Handle launch_mutex(CreateMutexW(nullptr, TRUE, L"Local\\Aries.Offline.Loader"));

        if (!launch_mutex.get() || GetLastError() == ERROR_ALREADY_EXISTS) {
            throw std::runtime_error("another Aries loader is active");
        }

        // Refuse rather than terminate any unrelated/pre-existing game.
        if (!server_only &&
            (init::process_exists(L"VALORANT-Win64-Shipping.exe") || init::process_exists(L"VALORANT.exe"))) {
            throw std::runtime_error("a VALORANT instance is already running; it was left untouched");
        }

        if (options.mode == LaunchMode::InstallFirewall) {
            install_firewall(settings);

            return 0;
        }

        verify_service_files(settings, server_only);

        const auto pinned_file = server_only ? init::Handle{} : pin_game_file(settings);

        if (options.mode == LaunchMode::Check) {
            std::cout << "Preflight passed; no process launched.\n";

            return 0;
        }

        LaunchSession session(settings, ports, server_only);
        session.run();

        return 0;
    }
} // namespace loader
