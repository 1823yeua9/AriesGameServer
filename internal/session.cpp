#include "session.h"

#include <Windows.h>
#include <shellapi.h>
#include <fstream>
#include <mutex>
#include <stdexcept>
#include "../loader/3p/json/json.hpp"

namespace internal::session {
    static std::filesystem::path runtime_path;
    static std::ofstream log_file;
    static std::mutex log_mutex;

    auto initialize() -> void {
        int count = 0;
        auto** arguments = CommandLineToArgvW(GetCommandLineW(), &count);

        if (!arguments) {
            throw std::runtime_error("Cannot parse process arguments");
        }

        for (int index = 1; index < count; ++index) {
            const std::wstring argument(arguments[index]);
            const std::wstring prefix = L"-aries-runtime=";

            if (argument.starts_with(prefix)) {
                if (!runtime_path.empty()) {
                    LocalFree(arguments);
                    throw std::runtime_error("Duplicate runtime argument");
                }

                runtime_path = argument.substr(prefix.size());
            }
        }

        LocalFree(arguments);

        if (!runtime_path.is_absolute() || !std::filesystem::is_regular_file(runtime_path / "startup.armed")) {
            throw std::runtime_error("Internal requires an armed loader-owned session");
        }

        std::ifstream input(runtime_path / "internal.json");
        const auto config = nlohmann::json::parse(input);
        wchar_t image_path[32768]{};
        const auto length = GetModuleFileNameW(nullptr, image_path, 32768);
        const auto expected_path = config.at("game_path").get<std::string>();
        const std::u8string expected_utf8(expected_path.begin(), expected_path.end());

        if (!length || length >= 32768 || config.value("protocol", 0) != 1 ||
            config.value("pid", 0u) != GetCurrentProcessId() || config.value("host", "") != "127.0.0.1" ||
            !std::filesystem::equivalent(std::filesystem::path(expected_utf8), image_path)) {
            throw std::runtime_error("Internal session identity mismatch");
        }

        log_file.open(runtime_path / "internal.log", std::ios::app);
        log_file << std::unitbuf;

        if (!log_file) {
            throw std::runtime_error("Cannot open internal audit log");
        }

        log("Validated loader session; entity viewers disabled by default");
    }

    auto runtime_directory() -> const std::filesystem::path& {
        return runtime_path;
    }

    auto signal_ready() -> void {
        const auto name = L"Local\\Aries.Internal.Ready." + std::to_wstring(GetCurrentProcessId());
        const auto event = OpenEventW(EVENT_MODIFY_STATE, FALSE, name.c_str());

        if (!event) {
            throw std::runtime_error("Loader readiness event missing");
        }

        const auto signaled = SetEvent(event);
        CloseHandle(event);

        if (!signaled) {
            throw std::runtime_error("Cannot signal internal readiness");
        }

        log("Internal startup prepared; loader may release configuration gate");
    }

    auto log(const std::string& message) -> void {
        std::lock_guard lock(log_mutex);
        log_file << GetTickCount64() << " " << message << '\n';

        if (message.starts_with("FAILED:") && !runtime_path.empty()) {
            std::ofstream failed(runtime_path / "internal.failed");
            failed << message;
        }
    }
} // namespace internal::session

extern "C" void overlay_log(const char* message) {
    internal::session::log(message ? message : "(null)");
}
