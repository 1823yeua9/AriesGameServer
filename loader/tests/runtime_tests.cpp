#include <WinSock2.h>
#include "../src/init/runtime.h"
#include "../src/utils/injector/injector.h"
#include "../src/utils/config/settings.h"
#include <fstream>
#include <iostream>
#include <optional>

namespace loader::tests {
    auto run_command_line_tests() -> void;
    auto run_firewall_tests() -> void;
    namespace fs = std::filesystem;

    static auto check(bool value, const char* message) -> void {
        if (!value) {
            throw std::runtime_error(message);
        }
    }

    static auto test_udp_readiness() -> void {
        WSADATA data{};
        check(WSAStartup(MAKEWORD(2, 2), &data) == 0, "Winsock initialization failed");

        struct Scope {
            ~Scope() {
                WSACleanup();
            }
        } scope;

        for (const auto address :
             {static_cast<unsigned long>(INADDR_LOOPBACK), static_cast<unsigned long>(INADDR_ANY)}) {
            const auto socket = ::socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
            check(socket != INVALID_SOCKET, "test socket creation failed");

            struct SocketScope {
                SOCKET value;

                ~SocketScope() {
                    closesocket(value);
                }
            } socket_scope{socket};

            sockaddr_in endpoint{};
            endpoint.sin_family = AF_INET;
            endpoint.sin_addr.s_addr = htonl(address);
            check(bind(socket, reinterpret_cast<sockaddr*>(&endpoint), sizeof(endpoint)) == 0, "test UDP bind failed");
            int length = sizeof(endpoint);
            check(getsockname(socket, reinterpret_cast<sockaddr*>(&endpoint), &length) == 0,
                  "test UDP address missing");

            const auto port = ntohs(endpoint.sin_port);
            check(init::owns_loopback_udp(GetCurrentProcessId(), port) == (address == INADDR_LOOPBACK),
                  "UDP scope verification failed");
            check(!init::owns_loopback_udp(0, port), "UDP owner verification failed");
        }
    }

    auto run(const fs::path& self) -> int {
        run_command_line_tests();
        run_firewall_tests();
        test_udp_readiness();

        const auto suffix = init::random_token();
        const auto directory =
            fs::temp_directory_path() / (L"aries test 测试 " + std::wstring(suffix.begin(), suffix.end()));
        check(!fs::exists(directory), "test directory already exists");
        fs::create_directory(directory);

        // Only this newly created directory is removed; no project/account data.
        struct DirectoryScope {
            fs::path path;

            ~DirectoryScope() {
                std::error_code error;
                fs::remove_all(path, error);
            }
        } scope{directory};

        const auto echo_path = directory / L"args.json";
        const std::vector<std::wstring> arguments = {
            L"", L"space space", L"ends\\", L"quote\"inside", L"\\\\\"", L"unicode 测试"};
        {
            init::Job job;
            std::vector<std::wstring> command = {L"--echo", echo_path.wstring()};
            command.insert(command.end(), arguments.begin(), arguments.end());

            auto child = job.start(self, command, directory, true, directory / L"child.log");
            check(WaitForSingleObject(child.process.get(), 10000) == WAIT_OBJECT_0, "argument child did not exit");

            const auto result = init::read_json(echo_path);

            for (std::size_t index = 0; index < arguments.size(); ++index) {
                const auto text = result[index].get<std::vector<unsigned short>>();
                check(std::wstring(text.begin(), text.end()) == arguments[index], "argument quoting changed a value");
            }
        }

        const auto hash_path = directory / L"hash.txt";
        {
            std::ofstream output(hash_path);
            output << "abc";
        }

        init::Handle hash_file(
            CreateFileW(hash_path.c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING, 0, nullptr));
        check(init::sha256(hash_file.get()) == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
              "SHA256 mismatch");

        const auto settings_path = directory / L"settings.val";
        config::Settings settings(settings_path);
        settings.set_setting("game_path", "relative/game.exe");
        check(settings.save_settings(), "settings save failed");
        config::Settings loaded(settings_path);
        check(loaded.load_settings(), "settings load failed");
        check(loaded.path_for("game_path") == directory / L"relative/game.exe",
              "settings path was not relative to config");
        init::write_json(settings_path, {{"game_path", "wrong"}, {"host_ip", 123}});
        check(!loaded.load_settings(), "invalid config accepted");
        check(loaded.get_setting("game_path") == "relative/game.exe", "invalid config partially mutated settings");

        const auto dll_path = directory / L"module 测试.dll";
        fs::copy_file(self.parent_path() / L"fixture.dll", dll_path);

        std::optional<init::Process> held_child;
        {
            init::Job job;
            held_child.emplace(job.start(self, {L"--child"}, directory, true));

            const auto ready_name = L"Local\\Aries.Test.ChildReady." + std::to_wstring(held_child->pid);
            const auto deadline = GetTickCount64() + 10000;
            bool ready = false;

            while (GetTickCount64() < deadline) {
                init::Handle event(OpenEventW(SYNCHRONIZE, FALSE, ready_name.c_str()));

                if (event.get() && WaitForSingleObject(event.get(), 0) == WAIT_OBJECT_0) {
                    ready = true;
                    break;
                }

                Sleep(20);
            }

            check(ready, "fixture child not ready");

            const auto event_name = L"Local\\Aries.Test.DllLoaded." + std::to_wstring(held_child->pid);
            init::Handle loaded_event(CreateEventW(nullptr, TRUE, FALSE, event_name.c_str()));
            injector::Injector injection(held_child->pid);
            check(injection.inject_module(directory / L"missing.dll") == injector::InjectionResult::FailedDll,
                  "missing DLL accepted");

            const auto result = injection.inject_module(dll_path);

            if (result != injector::InjectionResult::Success) {
                loader::init::Handle snapshot(CreateToolhelp32Snapshot(TH32CS_SNAPMODULE, held_child->pid));
                MODULEENTRY32W entry{};
                entry.dwSize = sizeof(entry);

                auto print_path = [](const fs::path& path) {
                    const auto text = path.u8string();
                    std::cerr << nlohmann::json(std::string(text.begin(), text.end())).dump(-1, ' ', true) << '\n';
                };
                print_path(fs::canonical(dll_path));

                if (Module32FirstW(snapshot.get(), &entry)) {
                    do {
                        print_path(entry.szExePath);
                    } while (Module32NextW(snapshot.get(), &entry));
                }
                else {
                    std::cerr << "Module enumeration error: " << GetLastError() << '\n';
                }

                throw std::runtime_error(
                    "Unicode DLL load failed: result=" + std::to_string(static_cast<unsigned>(result)) +
                    " thread=" + std::to_string(injection.last_thread_exit_code()));
            }

            check(WaitForSingleObject(loaded_event.get(), 1000) == WAIT_OBJECT_0, "DLL attach event not signaled");
            check(injection.inject_module(dll_path) == injector::InjectionResult::Success,
                  "duplicate injection not idempotent");
            check(held_child->running(), "child unexpectedly exited");
        }

        check(WaitForSingleObject(held_child->process.get(), 5000) == WAIT_OBJECT_0, "job did not close its child");

        std::cout << "PASS: command-line modes/paths, UDP PID/scope readiness, firewall scope normalization (no policy "
                     "writes), Unicode/quoted arguments, SHA256, transactional settings, Unicode DLL load, "
                     "duplicate/missing DLL, job cleanup\n";

        return 0;
    }
} // namespace loader::tests

int wmain(int argc, wchar_t** argv) {
    try {
        if (argc >= 2 && std::wstring(argv[1]) == L"--child") {
            const auto name = L"Local\\Aries.Test.ChildReady." + std::to_wstring(GetCurrentProcessId());
            loader::init::Handle ready(CreateEventW(nullptr, TRUE, TRUE, name.c_str()));
            Sleep(30000);

            return 0;
        }

        if (argc >= 3 && std::wstring(argv[1]) == L"--echo") {
            auto value = nlohmann::json::array();

            for (int index = 3; index < argc; ++index) {
                const std::wstring text = argv[index];
                value.push_back(std::vector<unsigned short>(text.begin(), text.end()));
            }

            loader::init::write_json(argv[2], value);

            return 0;
        }

        wchar_t self[32768]{};
        GetModuleFileNameW(nullptr, self, 32768);

        return loader::tests::run(self);
    }
    catch (const std::exception& error) {
        std::cerr << error.what() << '\n';

        return 1;
    }
}
