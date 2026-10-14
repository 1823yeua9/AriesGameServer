#include <WinSock2.h>
#include "runtime.h"
#include <iphlpapi.h>
#include <TlHelp32.h>
#include <bcrypt.h>
#include <winhttp.h>
#include <array>
#include <fstream>
#include <iomanip>
#include <sstream>
#include <stdexcept>

#pragma comment(lib, "bcrypt.lib")
#pragma comment(lib, "winhttp.lib")
#pragma comment(lib, "iphlpapi.lib")
#pragma comment(lib, "ws2_32.lib")

namespace loader::init {
    static auto require(bool success, const char* operation) -> void {
        if (!success) {
            throw std::runtime_error(std::string(operation) + " (Windows " + std::to_string(GetLastError()) + ")");
        }
    }

    auto quote_argument(const std::wstring& value) -> std::wstring {
        std::wstring result = L"\"";
        std::size_t backslashes = 0;

        for (const auto character : value) {
            if (character == L'\\') {
                ++backslashes;
                continue;
            }

            result.append(backslashes * (character == L'"' ? 2 : 1), L'\\');

            if (character == L'"') {
                result += L'\\';
            }

            result += character;
            backslashes = 0;
        }

        result.append(backslashes * 2, L'\\');

        return result + L'"';
    }

    Job::Job() : handle(CreateJobObjectW(nullptr, nullptr)) {
        require(handle.get() != nullptr, "create launch job");
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION info{};
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        require(SetInformationJobObject(handle.get(), JobObjectExtendedLimitInformation, &info, sizeof(info)),
                "configure launch job");
    }

    auto Job::start(const std::filesystem::path& executable,
                    const std::vector<std::wstring>& arguments,
                    const std::filesystem::path& working_directory,
                    bool hidden,
                    const std::filesystem::path& log_path) -> Process {
        auto command = quote_argument(executable.wstring());

        for (const auto& argument : arguments) {
            command += L" " + quote_argument(argument);
        }

        if (command.size() >= 32767) {
            throw std::runtime_error("child command line exceeds Windows limit");
        }

        STARTUPINFOEXW startup{};
        startup.StartupInfo.cb = sizeof(startup);

        SECURITY_ATTRIBUTES security{sizeof(security), nullptr, TRUE};
        Handle log(log_path.empty() ? nullptr
                                    : CreateFileW(log_path.c_str(),
                                                  GENERIC_WRITE,
                                                  FILE_SHARE_READ,
                                                  &security,
                                                  CREATE_NEW,
                                                  FILE_ATTRIBUTE_NORMAL,
                                                  nullptr));
        Handle input(
            log_path.empty()
                ? nullptr
                : CreateFileW(
                      L"NUL", GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, &security, OPEN_EXISTING, 0, nullptr));

        std::vector<unsigned char> attributes;

        if (!log_path.empty()) {
            require(log.get() != INVALID_HANDLE_VALUE && input.get() != INVALID_HANDLE_VALUE, "open child log");

            SIZE_T attribute_size = 0;
            InitializeProcThreadAttributeList(nullptr, 1, 0, &attribute_size);
            attributes.resize(attribute_size);
            startup.lpAttributeList = reinterpret_cast<PPROC_THREAD_ATTRIBUTE_LIST>(attributes.data());
            require(InitializeProcThreadAttributeList(startup.lpAttributeList, 1, 0, &attribute_size),
                    "initialize child handle list");

            HANDLE inherited[] = {log.get(), input.get()};

            if (!UpdateProcThreadAttribute(startup.lpAttributeList,
                                           0,
                                           PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                                           inherited,
                                           sizeof(inherited),
                                           nullptr,
                                           nullptr)) {
                DeleteProcThreadAttributeList(startup.lpAttributeList);
                throw std::runtime_error("cannot restrict inherited handles");
            }

            startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            startup.StartupInfo.hStdOutput = log.get();
            startup.StartupInfo.hStdError = log.get();
            startup.StartupInfo.hStdInput = input.get();
        }

        PROCESS_INFORMATION info{};
        const auto flags = CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT | (hidden ? CREATE_NO_WINDOW : 0);
        const auto created = CreateProcessW(executable.c_str(),
                                            command.data(),
                                            nullptr,
                                            nullptr,
                                            !log_path.empty(),
                                            flags,
                                            nullptr,
                                            working_directory.c_str(),
                                            &startup.StartupInfo,
                                            &info);

        if (startup.lpAttributeList) {
            DeleteProcThreadAttributeList(startup.lpAttributeList);
        }

        require(created, "start child");
        Process result{Handle(info.hProcess), Handle(info.hThread), info.dwProcessId};
        // No child executes before it belongs to our kill-on-close job. Failure
        // terminates only this newly created child, never a pre-existing session.
        if (!AssignProcessToJobObject(handle.get(), info.hProcess) ||
            ResumeThread(info.hThread) == static_cast<DWORD>(-1)) {
            TerminateProcess(info.hProcess, 1);
            WaitForSingleObject(info.hProcess, 5000);
            throw std::runtime_error("cannot supervise child process");
        }

        return result;
    }

    auto process_exists(const std::wstring& name) -> bool {
        Handle snapshot(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0));
        require(snapshot.get() != INVALID_HANDLE_VALUE, "enumerate processes");

        PROCESSENTRY32W entry{};
        entry.dwSize = sizeof(entry);
        require(Process32FirstW(snapshot.get(), &entry), "read process list");
        do {
            if (_wcsicmp(entry.szExeFile, name.c_str()) == 0) {
                return true;
            }
        } while (Process32NextW(snapshot.get(), &entry));

        return false;
    }

    auto owns_loopback_udp(DWORD pid, unsigned short port) -> bool {
        ULONG size = 0;
        auto status = GetExtendedUdpTable(nullptr, &size, FALSE, AF_INET, UDP_TABLE_OWNER_PID, 0);

        if (status != ERROR_INSUFFICIENT_BUFFER) {
            return false;
        }

        std::vector<unsigned char> storage(size);
        status = GetExtendedUdpTable(storage.data(), &size, FALSE, AF_INET, UDP_TABLE_OWNER_PID, 0);

        if (status != NO_ERROR) {
            return false;
        }

        const auto* table = reinterpret_cast<const MIB_UDPTABLE_OWNER_PID*>(storage.data());
        bool found = false;

        for (DWORD index = 0; index < table->dwNumEntries; ++index) {
            const auto& row = table->table[index];

            if (row.dwOwningPid != pid || ntohs(static_cast<unsigned short>(row.dwLocalPort)) != port) {
                continue;
            }

            if (row.dwLocalAddr != htonl(INADDR_LOOPBACK)) {
                return false;
            }

            found = true;
        }

        return found;
    }

    auto sha256(HANDLE file) -> std::string {
        BCRYPT_ALG_HANDLE algorithm = nullptr;

        if (BCryptOpenAlgorithmProvider(&algorithm, BCRYPT_SHA256_ALGORITHM, nullptr, 0) < 0) {
            throw std::runtime_error("cannot open SHA256 provider");
        }

        struct AlgorithmScope {
            BCRYPT_ALG_HANDLE value;

            ~AlgorithmScope() {
                BCryptCloseAlgorithmProvider(value, 0);
            }
        } algorithm_scope{algorithm};

        BCRYPT_HASH_HANDLE hash = nullptr;

        if (BCryptCreateHash(algorithm, &hash, nullptr, 0, nullptr, 0, 0) < 0) {
            throw std::runtime_error("cannot create SHA256 hash");
        }

        struct HashScope {
            BCRYPT_HASH_HANDLE value;

            ~HashScope() {
                BCryptDestroyHash(value);
            }
        } hash_scope{hash};

        LARGE_INTEGER zero{};
        require(SetFilePointerEx(file, zero, nullptr, FILE_BEGIN), "seek pinned binary");

        std::array<unsigned char, 65536> bytes{};
        DWORD count = 0;

        for (;;) {
            require(ReadFile(file, bytes.data(), static_cast<DWORD>(bytes.size()), &count, nullptr),
                    "read pinned binary");

            if (!count) {
                break;
            }

            if (BCryptHashData(hash, bytes.data(), count, 0) < 0) {
                throw std::runtime_error("SHA256 update failed");
            }
        }

        std::array<unsigned char, 32> digest{};

        if (BCryptFinishHash(hash, digest.data(), static_cast<ULONG>(digest.size()), 0) < 0) {
            throw std::runtime_error("SHA256 finish failed");
        }

        std::ostringstream output;

        for (const auto value : digest) {
            output << std::hex << std::setw(2) << std::setfill('0') << static_cast<unsigned>(value);
        }

        return output.str();
    }

    auto random_token() -> std::string {
        std::array<unsigned char, 32> bytes{};

        if (BCryptGenRandom(nullptr, bytes.data(), static_cast<ULONG>(bytes.size()), BCRYPT_USE_SYSTEM_PREFERRED_RNG) <
            0) {
            throw std::runtime_error("secure random generation failed");
        }

        std::ostringstream output;

        for (const auto value : bytes) {
            output << std::hex << std::setw(2) << std::setfill('0') << static_cast<unsigned>(value);
        }

        return output.str();
    }

    auto read_json(const std::filesystem::path& path) -> nlohmann::json {
        if (std::filesystem::file_size(path) > 4 * 1024 * 1024) {
            throw std::runtime_error("JSON file exceeds size limit");
        }

        std::ifstream input(path);
        nlohmann::json value;
        input >> value;

        return value;
    }

    auto write_json(const std::filesystem::path& path, const nlohmann::json& value) -> void {
        auto temporary = path;
        temporary += L".tmp";
        {
            std::ofstream output(temporary);
            output << value.dump(2);
            output.flush();

            if (!output) {
                throw std::runtime_error("cannot write JSON file");
            }
        }

        require(MoveFileExW(temporary.c_str(), path.c_str(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH),
                "publish JSON file");
    }

    class HttpHandle {
    public:
        HINTERNET value;

        explicit HttpHandle(HINTERNET handle) : value(handle) {
            require(value != nullptr, "create HTTP handle");
        }

        ~HttpHandle() {
            WinHttpCloseHandle(value);
        }

        HttpHandle(const HttpHandle&) = delete;
    };

    auto admin_request(unsigned short port, const std::string& token, const wchar_t* method, const wchar_t* path)
        -> nlohmann::json {
        HttpHandle session(WinHttpOpen(L"AriesLoader/1", WINHTTP_ACCESS_TYPE_NO_PROXY, nullptr, nullptr, 0));
        require(WinHttpSetTimeouts(session.value, 2000, 2000, 2000, 2000), "set HTTP timeouts");
        HttpHandle connection(WinHttpConnect(session.value, L"127.0.0.1", port, 0));
        HttpHandle request(WinHttpOpenRequest(
            connection.value, method, path, nullptr, WINHTTP_NO_REFERER, WINHTTP_DEFAULT_ACCEPT_TYPES, 0));

        DWORD redirects = WINHTTP_OPTION_REDIRECT_POLICY_NEVER;
        require(WinHttpSetOption(request.value, WINHTTP_OPTION_REDIRECT_POLICY, &redirects, sizeof(redirects)),
                "disable HTTP redirects");

        const auto header = L"Authorization: Bearer " + std::wstring(token.begin(), token.end());
        require(WinHttpSendRequest(request.value, header.c_str(), static_cast<DWORD>(header.size()), nullptr, 0, 0, 0),
                "send local admin request");
        require(WinHttpReceiveResponse(request.value, nullptr), "receive local admin response");

        DWORD status = 0, size = sizeof(status);
        require(
            WinHttpQueryHeaders(
                request.value, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, nullptr, &status, &size, nullptr),
            "read HTTP status");

        if (status != 200) {
            throw std::runtime_error("local admin HTTP " + std::to_string(status));
        }

        std::string payload;
        std::array<char, 4096> chunk{};
        DWORD count = 0;

        for (;;) {
            require(WinHttpReadData(request.value, chunk.data(), static_cast<DWORD>(chunk.size()), &count),
                    "read local admin body");

            if (!count) {
                break;
            }

            payload.append(chunk.data(), count);

            if (payload.size() > 1024 * 1024) {
                throw std::runtime_error("local admin body exceeds limit");
            }
        }

        return nlohmann::json::parse(payload);
    }
} // namespace loader::init
