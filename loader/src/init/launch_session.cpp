#include "launch_session.h"
#include "../utils/injector/injector.h"

#include <algorithm>
#include <atomic>
#include <conio.h>
#include <fstream>
#include <iostream>

namespace loader {
    namespace fs = std::filesystem;

    static std::atomic_bool stop_requested = false;

    static auto WINAPI console_control(DWORD event) -> BOOL {
        if (event == CTRL_C_EVENT || event == CTRL_BREAK_EVENT || event == CTRL_CLOSE_EVENT) {
            stop_requested = true;

            return TRUE;
        }

        return FALSE;
    }

    template <class Predicate>
    static auto wait_for(Predicate predicate, const init::Process& process, DWORD timeout, const char* purpose)
        -> void {
        const auto deadline = GetTickCount64() + timeout;

        while (!predicate()) {
            if (stop_requested || !process.running()) {
                throw std::runtime_error(std::string(purpose) + ": child exited or launch canceled");
            }

            if (GetTickCount64() >= deadline) {
                throw std::runtime_error(std::string(purpose) + ": timed out");
            }

            Sleep(50);
        }
    }

    static auto file_contains(const fs::path& path, const std::string& marker) -> bool {
        std::ifstream input(path, std::ios::binary);

        if (!input) {
            return false;
        }

        input.seekg(0, std::ios::end);

        const auto length = input.tellg();

        if (length <= 0) {
            return false;
        }

        // Bounded fresh-log tail, never stale global ShooterGame.log.
        input.seekg((std::max)(std::streamoff(0), std::streamoff(length) - 256 * 1024));

        const std::string text((std::istreambuf_iterator<char>(input)), {});

        return text.find(marker) != std::string::npos;
    }

    static auto utf8_path(const fs::path& path) -> std::string {
        const auto text = path.u8string();

        return std::string(text.begin(), text.end());
    }

    LaunchSession::LaunchSession(const config::Settings& settings, ServicePorts requested_ports, bool server_only)
        : settings(settings), requested_ports(requested_ports), server_only(server_only) {}

    auto LaunchSession::run() -> void {
        SetConsoleCtrlHandler(console_control, TRUE);

        prepare_platform_config();

        start_platform();

        if (server_only) {
            supervise_platform();

            return;
        }

        start_game();

        initialize_internal();

        wait_for(
            [&] {
                return file_contains(game_log, "MainMenuV2");
            },
            *game,
            90000,
            "main-menu initialization");

        start_gameplay();

        supervise_game();
    }

    auto LaunchSession::prepare_platform_config() -> void {
        run_directory = settings.path_for("runtime_path") / init::random_token();
        fs::create_directories(run_directory);

        gate = run_directory / L"startup.armed";
        waiting = gate;
        waiting.replace_extension(L"waiting");

        state_path = run_directory / L"backend_state.json";
        server_config = run_directory / L"platform.json";
        game_log = run_directory / L"game.log";

        auth_token = init::random_token();
        admin_token = init::random_token();

        nlohmann::json platform_config = {{"port", requested_ports.platform},
                                          {"admin_port", requested_ports.admin},

                                          {"cert_path", utf8_path(settings.path_for("cert_path"))},
                                          {"key_path", utf8_path(settings.path_for("key_path"))},

                                          {"skin_catalog", utf8_path(settings.path_for("skin_catalog"))},
                                          {"skin_state", utf8_path(settings.path_for("skin_state"))},
                                          {"persistence_path", utf8_path(settings.path_for("game_settings_path"))},

                                          {"state_path", utf8_path(state_path)},
                                          {"auth_token", auth_token},
                                          {"admin_token", admin_token}};

        if (!server_only) {
            std::ofstream armed(gate);
            armed << "armed";
            armed.close();

            if (!armed) {
                throw std::runtime_error("cannot arm startup gate");
            }

            platform_config["startup_gate"] = utf8_path(gate);
        }

        init::write_json(server_config, platform_config);
    }

    auto LaunchSession::start_platform() -> void {
        const auto path = settings.path_for("server_path");

        server.emplace(job.start(
            path, {L"--config", server_config.wstring()}, path.parent_path(), true, run_directory / L"platform.log"));

        wait_for(
            [&] {
                return fs::is_regular_file(state_path);
            },
            *server,
            15000,
            "platform readiness");

        verify_platform_state();

        wait_for(
            [&] {
                try {
                    const auto health = init::admin_request(ports.admin, admin_token, L"GET", L"/health");

                    return health.value("pid", 0u) == server->pid && health.value("ready", false);
                }
                catch (...) {
                    return false;
                }
            },
            *server,
            10000,
            "platform health");

        std::cout << "Platform ready. Run logs: " << run_directory.string() << '\n';
    }

    auto LaunchSession::verify_platform_state() -> void {
        const auto state = init::read_json(state_path);

        const auto port_number = state.value("port", 0);
        const auto admin_port_number = state.value("admin_port", 0);

        const auto identity_matches = state.value("pid", 0u) == server->pid &&
                                      state.value("auth_token", "") == auth_token &&
                                      state.value("admin_token", "") == admin_token;

        const auto hosts_match = state.value("host", "") == "127.0.0.1" && state.value("admin_host", "") == "127.0.0.1";

        const auto ports_valid =
            port_number >= 1 && port_number <= 65535 && admin_port_number >= 1 && admin_port_number <= 65535;

        const auto ports_match = (!requested_ports.platform || port_number == requested_ports.platform) &&
                                 (!requested_ports.admin || admin_port_number == requested_ports.admin);

        if (!identity_matches || !hosts_match || !ports_valid || !ports_match) {
            throw std::runtime_error("platform state does not belong to this launch");
        }

        ports.platform = static_cast<unsigned short>(port_number);
        ports.admin = static_cast<unsigned short>(admin_port_number);
    }

    auto LaunchSession::supervise_platform() -> void {
        while (!stop_requested && server->running()) {
            if (_kbhit() && _getch() == 'q') {
                stop_requested = true;

                break;
            }

            Sleep(100);
        }

        if (!stop_requested && !server->running()) {
            throw std::runtime_error("platform exited unexpectedly");
        }
    }

    auto LaunchSession::game_arguments() const -> std::vector<std::wstring> {
        const auto endpoint = L"https://127.0.0.1:" + std::to_wstring(ports.platform) + L"/.a.pvp.net";
        const auto wide_token = std::wstring(auth_token.begin(), auth_token.end());

        return {
            L"-remoting-app-port=" + std::to_wstring(ports.platform),
            L"-remoting-auth-token=" + wide_token,
            L"-patchline=live",

            L"-subject=7c9f41a4-65e2-4d81-a8d5-f879c3e5ab61",
            L"-ares-deployment=na",

            std::wstring(L"-config-endpoint=") + endpoint,
            std::wstring(L"-riot-config-endpoint=") + endpoint,
            std::wstring(
                L"-clientconfigoverrides=valorant.client.should_use_affinity=false|valorant.client.config_endpoint=") +
                endpoint + L"|valorant.client.platform_environment=na|valorant.client.game_shard_zone=na",

            L"-savetouserdir",
            L"-culture=en_US",
            L"-MULTIHOME=127.0.0.1",
            L"--persistence-path=" + settings.path_for("game_settings_path").wstring(),

            L"-ABSLOG=" + game_log.wstring(),
            L"-aries-runtime=" + run_directory.wstring()};
    }

    auto LaunchSession::start_game() -> void {
        if (init::process_exists(L"VALORANT-Win64-Shipping.exe") || init::process_exists(L"VALORANT.exe")) {
            throw std::runtime_error("a game appeared during startup; launch canceled");
        }

        const auto path = settings.path_for("game_path");

        game.emplace(job.start(path, game_arguments(), path.parent_path(), false));

        wait_for(
            [&] {
                return fs::is_regular_file(waiting);
            },
            *game,
            20000,
            "held game configuration request");
    }

    auto LaunchSession::initialize_internal() -> void {
        const auto ready_name = L"Local\\Aries.Internal.Ready." + std::to_wstring(game->pid);

        internal_ready.emplace(CreateEventW(nullptr, TRUE, FALSE, ready_name.c_str()));

        if (!internal_ready->get() || GetLastError() == ERROR_ALREADY_EXISTS) {
            throw std::runtime_error("internal readiness event is unavailable or stale");
        }

        injector::Injector injection(game->pid);

        if (injection.inject_module(settings.path_for("internal_path")) != injector::InjectionResult::Success) {
            throw std::runtime_error("internal DLL failed to load; owned launch will close");
        }

        // internal signals only after its startup work is prepared. DLL attachment
        // alone is not readiness. The bounded gate remains held on any failure.
        wait_for(
            [&] {
                return WaitForSingleObject(internal_ready->get(), 0) == WAIT_OBJECT_0;
            },
            *game,
            15000,
            "internal startup readiness");

        if (!fs::remove(gate)) {
            throw std::runtime_error("startup gate was unexpectedly absent");
        }
    }

    auto LaunchSession::start_gameplay() -> void {
        const auto path = settings.path_for("gameplay_path");

        if (path.empty()) {
            return;
        }

        // aries-server resolves the current image base from this owned PID.
        // It must start after the client, not race an absent process at boot.
        const std::vector<std::wstring> arguments = {L"--config",
                                                     settings.path_for("gameplay_config").wstring(),
                                                     L"--pid",
                                                     std::to_wstring(game->pid),
                                                     L"--port",
                                                     L"58100"};

        gameplay.emplace(job.start(path, arguments, path.parent_path(), true, run_directory / L"gameplay.log"));

        wait_for(
            [&] {
                if (!game->running() || !server->running()) {
                    throw std::runtime_error("game/platform exited while starting gameplay");
                }

                return init::owns_loopback_udp(gameplay->pid, 58100);
            },
            *gameplay,
            15000,
            "gameplay loopback listener readiness");
    }

    auto LaunchSession::request_range() -> void {
        if (!gameplay) {
            std::cout << "No gameplay executable configured; Range request not sent.\n";

            return;
        }

        init::admin_request(ports.admin, admin_token, L"POST", L"/dev/range/enter");

        std::cout << "Range transition requested (not a gameplay-completion claim).\n";
    }

    auto LaunchSession::supervise_game() -> void {
        std::cout << "MainMenuV2 observed. R requests Range; Q/Ctrl+C closes this launch.\n";

        while (!stop_requested && game->running()) {
            if (!server->running() || (gameplay && !gameplay->running())) {
                throw std::runtime_error("owned server exited; closing this game session");
            }

            if (_kbhit()) {
                const auto key = _getch();

                if (key == 'q' || key == 'Q') {
                    break;
                }

                if (key == 'r' || key == 'R') {
                    request_range();
                }
            }

            Sleep(100);
        }
    }
} // namespace loader
