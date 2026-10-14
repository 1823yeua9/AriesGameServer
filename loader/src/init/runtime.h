#pragma once
#include <Windows.h>
#include <filesystem>
#include <string>
#include <vector>
#include <json/json.hpp>

namespace loader::init {
class Handle {
    HANDLE value = nullptr;
public:
    explicit Handle(HANDLE handle = nullptr) : value(handle) {}
    ~Handle() { if (value && value != INVALID_HANDLE_VALUE) CloseHandle(value); }
    Handle(const Handle&) = delete;
    auto operator=(const Handle&) -> Handle& = delete;
    Handle(Handle&& other) noexcept : value(other.value) { other.value = nullptr; }
    auto get() const -> HANDLE { return value; }
};
struct Process {
    Handle process;
    Handle thread;
    DWORD pid;
    auto running() const -> bool { return WaitForSingleObject(process.get(), 0) == WAIT_TIMEOUT; }
};
class Job {
    Handle handle;
public:
    Job();
    auto start(const std::filesystem::path& executable, const std::vector<std::wstring>& arguments,
        const std::filesystem::path& working_directory, bool hidden, const std::filesystem::path& log_path = {}) -> Process;
};
auto quote_argument(const std::wstring& value) -> std::wstring;
auto process_exists(const std::wstring& name) -> bool;
auto owns_loopback_udp(DWORD pid, unsigned short port) -> bool;
auto sha256(HANDLE file) -> std::string;
auto random_token() -> std::string;
auto read_json(const std::filesystem::path& path) -> nlohmann::json;
auto write_json(const std::filesystem::path& path, const nlohmann::json& value) -> void;
auto admin_request(unsigned short port, const std::string& token, const wchar_t* method, const wchar_t* path) -> nlohmann::json;
}
