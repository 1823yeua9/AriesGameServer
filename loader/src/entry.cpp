#include <iostream>
#include <fstream>
#include <atomic>
#include <optional>
#include <conio.h>
#include "utils/config/settings.h"
#include "utils/injector/injector.h"
#include "init/runtime.h"
#include "init/networking/firewall.h"

namespace loader {
namespace fs = std::filesystem;
using init::Handle;
using init::Process;
static std::atomic_bool stop_requested = false;
static constexpr auto pinned_hash = "ec6e08c7c3866c7100be4322fa70fe1198185c43e2a89f9d417b4f64b5bab0a2";

static auto WINAPI console_control(DWORD event) -> BOOL {
    if (event == CTRL_C_EVENT || event == CTRL_BREAK_EVENT || event == CTRL_CLOSE_EVENT) {
        stop_requested = true;
        return TRUE;
    }
    return FALSE;
}
static auto require_file(const fs::path& path, const char* purpose) -> void {
    if (path.empty() || !fs::is_regular_file(path)) throw std::runtime_error(std::string("missing ") + purpose + ": " + path.string());
}
template<class Predicate>
static auto wait_for(Predicate predicate, const Process& process, DWORD timeout, const char* purpose) -> void {
    const auto deadline = GetTickCount64() + timeout;
    while (!predicate()) {
        if (stop_requested || !process.running()) throw std::runtime_error(std::string(purpose) + ": child exited or launch canceled");
        if (GetTickCount64() >= deadline) throw std::runtime_error(std::string(purpose) + ": timed out");
        Sleep(50);
    }
}
static auto file_contains(const fs::path& path, const std::string& marker) -> bool {
    std::ifstream input(path, std::ios::binary);
    if (!input) return false;
    input.seekg(0, std::ios::end);
    const auto length = input.tellg();
    if (length <= 0) return false;
    // Bounded fresh-log tail, never stale global ShooterGame.log.
    input.seekg((std::max)(std::streamoff(0), std::streamoff(length) - 256 * 1024));
    std::string text((std::istreambuf_iterator<char>(input)), {});
    return text.find(marker) != std::string::npos;
}

auto run(int argc, wchar_t** argv) -> int {
    fs::path config_path;
    enum class Mode { Launch, Check, InstallFirewall, Template, ServerOnly };
    auto mode = Mode::Launch;
    for (int index = 1; index < argc; ++index) {
        const std::wstring argument = argv[index];
        if (argument == L"--help") {
            std::cout << "Aries loader --config <settings.val> [--check | --install-firewall | --server-only]\n"
                "Aries loader --write-config <settings.val>\n"
                "Launch owns only its children. R enters Range, Q/Ctrl+C closes the launch.\n";
            return 0;
        }
        if ((argument == L"--config" || argument == L"--write-config") && index + 1 < argc) {
            config_path = fs::absolute(argv[++index]);
            if (argument == L"--write-config") mode = Mode::Template;
        } else if (argument == L"--check") mode = Mode::Check;
        else if (argument == L"--install-firewall") mode = Mode::InstallFirewall;
        else if (argument == L"--server-only") mode = Mode::ServerOnly;
        else throw std::runtime_error("invalid or incomplete command line; use --help");
    }
    config::Settings settings(config_path);
    if (mode == Mode::Template) {
        if (fs::exists(config_path)) throw std::runtime_error("refusing to overwrite existing settings");
        settings.set_setting("game_path", "../game req/VALORANT_GAME/VALORANT/live/ShooterGame/Binaries/Win64/VALORANT-Win64-Shipping.exe");
        settings.set_setting("game_settings_path", "runtime/account/persistence");
        if (!settings.save_settings()) throw std::runtime_error("cannot write settings template");
        std::cout << "Created settings template. Relative paths are based on its directory.\n";
        return 0;
    }
    if (!settings.load_settings()) throw std::runtime_error("missing or invalid settings; use --write-config first");
    const auto parse_port = [&](const std::string& key) -> unsigned short {
        const auto& text = settings.get_setting(key);
        if (text.empty() || text.find_first_not_of("0123456789") != std::string::npos) throw std::runtime_error("invalid port setting");
        const auto number = std::stoul(text);
        if (number > 65535 || (!number && mode != Mode::ServerOnly)) throw std::runtime_error("port must be 1..65535 (0 allowed for server-only)");
        return static_cast<unsigned short>(number);
    };
    const auto requested_port = parse_port("port"), requested_admin_port = parse_port("admin_port");
    if (settings.get_setting("host_ip") != "localhost" && settings.get_setting("host_ip") != "127.0.0.1") throw std::runtime_error("this profile supports loopback only");
    const auto game_path = settings.path_for("game_path");
    const auto server_path = settings.path_for("server_path");
    const auto internal_path = settings.path_for("internal_path");
    const auto runtime_path = settings.path_for("runtime_path");
    if (runtime_path.empty()) throw std::runtime_error("runtime_path is required");
    if (settings.path_for("skin_state").empty() || settings.path_for("game_settings_path").empty()) throw std::runtime_error("skin_state and game_settings_path are required");
    const auto reporter_path = game_path.parent_path().parent_path().parent_path().parent_path() / L"Engine/Binaries/Win64/CrashReportClient.exe";
    // Refuse rather than terminate any unrelated/pre-existing game.
    Handle launch_mutex(CreateMutexW(nullptr, TRUE, L"Local\\Aries.Offline.Loader"));
    if (!launch_mutex.get() || GetLastError() == ERROR_ALREADY_EXISTS) throw std::runtime_error("another Aries loader is active");
    if (mode != Mode::ServerOnly && (init::process_exists(L"VALORANT-Win64-Shipping.exe") || init::process_exists(L"VALORANT.exe"))) throw std::runtime_error("a VALORANT instance is already running; it was left untouched");
    if (mode == Mode::InstallFirewall) {
        for (const auto& path : {game_path, reporter_path}) {
            require_file(path, "contained executable");
            if (init::networking::contain_loopback(path.wstring(), true) != init::networking::FirewallResult::Success) throw std::runtime_error("firewall installation/verification failed; use an elevated terminal, with all firewall profiles enabled");
        }
        std::cout << "Verified game and crash-reporter non-loopback block rules. No other programs or services changed.\n";
        return 0;
    }
    require_file(server_path, "platform executable");
    require_file(settings.path_for("cert_path"), "TLS certificate");
    require_file(settings.path_for("key_path"), "TLS private key");
    require_file(settings.path_for("skin_catalog"), "skin catalog");
    if (!settings.path_for("gameplay_path").empty()) {
        require_file(settings.path_for("gameplay_path"), "gameplay executable");
        require_file(settings.path_for("gameplay_config"), "gameplay config");
    }
    // Keep the original file open without write/delete sharing until teardown.
    Handle pinned_file(mode == Mode::ServerOnly ? nullptr : CreateFileW(game_path.c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr));
    if (mode != Mode::ServerOnly) {
        require_file(game_path, "game executable");
        require_file(internal_path, "internal DLL (oracle/startup module)");
        require_file(reporter_path, "crash reporter");
        if (pinned_file.get() == INVALID_HANDLE_VALUE || init::sha256(pinned_file.get()) != pinned_hash) throw std::runtime_error("game file is not the pinned original 13.06 build");
        for (const auto& path : {game_path, reporter_path}) {
            if (init::networking::contain_loopback(path.wstring(), false) != init::networking::FirewallResult::Success) throw std::runtime_error("containment missing or different; run --install-firewall explicitly");
        }
    }
    if (mode == Mode::Check) { std::cout << "Preflight passed; no process launched.\n"; return 0; }
    SetConsoleCtrlHandler(console_control, TRUE);
    const auto run_directory = runtime_path / init::random_token();
    fs::create_directories(run_directory);
    const auto gate = run_directory / L"startup.armed";
    auto waiting = gate; waiting.replace_extension(L"waiting");
    const auto state_path = run_directory / L"backend_state.json";
    const auto server_config = run_directory / L"platform.json";
    const auto game_log = run_directory / L"game.log";
    const auto auth_token = init::random_token(), admin_token = init::random_token();
    auto utf8_path = [](const fs::path& path) { const auto text = path.u8string(); return std::string(text.begin(), text.end()); };
    nlohmann::json platform_config = {
        {"port", requested_port}, {"admin_port", requested_admin_port}, {"cert_path", utf8_path(settings.path_for("cert_path"))},
        {"key_path", utf8_path(settings.path_for("key_path"))}, {"skin_catalog", utf8_path(settings.path_for("skin_catalog"))},
        {"skin_state", utf8_path(settings.path_for("skin_state"))}, {"state_path", utf8_path(state_path)},
        {"persistence_path", utf8_path(settings.path_for("game_settings_path"))}, {"auth_token", auth_token}, {"admin_token", admin_token}
    };
    if (mode != Mode::ServerOnly) {
        std::ofstream armed(gate); armed << "armed"; armed.close();
        if (!armed) throw std::runtime_error("cannot arm startup gate");
        platform_config["startup_gate"] = utf8_path(gate);
    }
    init::write_json(server_config, platform_config);
    init::Job job;
    auto server = job.start(server_path, {L"--config", server_config.wstring()}, server_path.parent_path(), true, run_directory / L"platform.log");
    wait_for([&] { return fs::is_regular_file(state_path); }, server, 15000, "platform readiness");
    const auto state = init::read_json(state_path);
    const auto port_number = state.value("port", 0), admin_port_number = state.value("admin_port", 0);
    if (state.value("pid", 0u) != server.pid || state.value("host", "") != "127.0.0.1" || state.value("admin_host", "") != "127.0.0.1" || port_number < 1 || port_number > 65535 || admin_port_number < 1 || admin_port_number > 65535 || (requested_port && port_number != requested_port) || (requested_admin_port && admin_port_number != requested_admin_port) || state.value("auth_token", "") != auth_token || state.value("admin_token", "") != admin_token) throw std::runtime_error("platform state does not belong to this launch");
    const auto port = static_cast<unsigned short>(port_number), admin_port = static_cast<unsigned short>(admin_port_number);
    wait_for([&] {
        try { const auto health = init::admin_request(admin_port, admin_token, L"GET", L"/health"); return health.value("pid", 0u) == server.pid && health.value("ready", false); }
        catch (...) { return false; }
    }, server, 10000, "platform health");
    std::cout << "Platform ready. Run logs: " << run_directory.string() << '\n';
    if (mode == Mode::ServerOnly) {
        while (!stop_requested && server.running()) { if (_kbhit() && _getch() == 'q') { stop_requested = true; break; } Sleep(100); }
        if (!stop_requested && !server.running()) throw std::runtime_error("platform exited unexpectedly");
        return 0;
    }
    std::optional<Process> gameplay;
    if (init::process_exists(L"VALORANT-Win64-Shipping.exe") || init::process_exists(L"VALORANT.exe")) throw std::runtime_error("a game appeared during startup; launch canceled");
    const auto endpoint = L"https://127.0.0.1:" + std::to_wstring(port) + L"/.a.pvp.net";
    std::vector<std::wstring> arguments = {L"-remoting-app-port=" + std::to_wstring(port), L"-remoting-auth-token=" + std::wstring(auth_token.begin(), auth_token.end()), L"-patchline=live",
        L"-subject=7c9f41a4-65e2-4d81-a8d5-f879c3e5ab61", L"-ares-deployment=na",
        std::wstring(L"-config-endpoint=") + endpoint, std::wstring(L"-riot-config-endpoint=") + endpoint,
        std::wstring(L"-clientconfigoverrides=valorant.client.should_use_affinity=false|valorant.client.config_endpoint=") + endpoint + L"|valorant.client.platform_environment=na|valorant.client.game_shard_zone=na",
        L"-savetouserdir", L"-culture=en_US", L"-MULTIHOME=127.0.0.1", L"--persistence-path=" + settings.path_for("game_settings_path").wstring(),
        L"-ABSLOG=" + game_log.wstring(), L"-aries-runtime=" + run_directory.wstring()};
    auto game = job.start(game_path, arguments, game_path.parent_path(), false);
    wait_for([&] { return fs::is_regular_file(waiting); }, game, 20000, "held game configuration request");
    const auto ready_name = L"Local\\Aries.Internal.Ready." + std::to_wstring(game.pid);
    Handle internal_ready(CreateEventW(nullptr, TRUE, FALSE, ready_name.c_str()));
    if (!internal_ready.get() || GetLastError() == ERROR_ALREADY_EXISTS) throw std::runtime_error("internal readiness event is unavailable or stale");
    injector::Injector injection(game.pid);
    if (injection.inject_module(internal_path) != injector::InjectionResult::Success) throw std::runtime_error("internal DLL failed to load; owned launch will close");
    // internal signals only after its startup work is prepared. DLL attachment
    // alone is not readiness. The bounded gate remains held on any failure.
    wait_for([&] { return WaitForSingleObject(internal_ready.get(), 0) == WAIT_OBJECT_0; }, game, 15000, "internal startup readiness");
    if (!fs::remove(gate)) throw std::runtime_error("startup gate was unexpectedly absent");
    wait_for([&] { return file_contains(game_log, "MainMenuV2"); }, game, 90000, "main-menu initialization");
    if (!settings.path_for("gameplay_path").empty()) {
        const auto path = settings.path_for("gameplay_path");
        // aries-server resolves the current image base from this owned PID.
        // It must start after the client, not race an absent process at boot.
        gameplay.emplace(job.start(path, {L"--config", settings.path_for("gameplay_config").wstring(),
            L"--pid", std::to_wstring(game.pid), L"--port", L"58100"}, path.parent_path(), true, run_directory / L"gameplay.log"));
        wait_for([&] {
            if (!game.running() || !server.running()) throw std::runtime_error("game/platform exited while starting gameplay");
            return init::owns_loopback_udp(gameplay->pid, 58100);
        }, *gameplay, 15000, "gameplay loopback listener readiness");
    }
    std::cout << "MainMenuV2 observed. R requests Range; Q/Ctrl+C closes this launch.\n";
    while (!stop_requested && game.running()) {
        if (!server.running() || (gameplay && !gameplay->running())) throw std::runtime_error("owned server exited; closing this game session");
        if (_kbhit()) {
            const auto key = _getch();
            if (key == 'q' || key == 'Q') break;
            if (key == 'r' || key == 'R') {
                if (!gameplay) std::cout << "No gameplay executable configured; Range request not sent.\n";
                else { init::admin_request(admin_port, admin_token, L"POST", L"/dev/range/enter"); std::cout << "Range transition requested (not a gameplay-completion claim).\n"; }
            }
        }
        Sleep(100);
    }
    return 0;
}
}

int wmain(int argc, wchar_t** argv) {
    try { return loader::run(argc, argv); }
    catch (const std::exception& error) { std::cerr << "Aries launch failed: " << error.what() << '\n'; return 1; }
}
