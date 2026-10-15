#include "skin_client.hpp"
#include "../session.h"

#include <winhttp.h>
#include <algorithm>
#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cctype>
#include <limits>
#include <mutex>
#include <thread>
#include <utility>

namespace skin_client {
    namespace detail {
        bool is_hex_digit(char value) {
            return (value >= '0' && value <= '9') || (value >= 'a' && value <= 'f') || (value >= 'A' && value <= 'F');
        }

        enum class JsonKind { invalid, null_value, boolean, number, string, array, object };

        struct JsonValue {
            JsonKind kind = JsonKind::invalid;
            bool boolean_value = false;
            std::uint64_t number_value = 0;
            std::string string_value;
            std::vector<JsonValue> array_values;
            std::vector<std::pair<std::string, JsonValue>> object_values;

            const JsonValue* field(const char* name) const {
                if (kind != JsonKind::object) {
                    return nullptr;
                }
                for (const auto& entry : object_values) {
                    if (entry.first == name) {
                        return &entry.second;
                    }
                }
                return nullptr;
            }
        };

        class JsonParser {
        public:
            explicit JsonParser(const std::string& input) : input_(input) {}

            bool parse(JsonValue& output) {
                if (!parse_value(output, 0)) {
                    return false;
                }
                skip_space();
                return position_ == input_.size();
            }

        private:
            const std::string& input_;
            std::size_t position_ = 0;

            void skip_space() {
                while (position_ < input_.size() && (input_[position_] == ' ' || input_[position_] == '\n' ||
                                                     input_[position_] == '\r' || input_[position_] == '\t')) {
                    ++position_;
                }
            }

            bool take(char expected) {
                skip_space();
                if (position_ >= input_.size() || input_[position_] != expected) {
                    return false;
                }
                ++position_;
                return true;
            }

            static int hex_digit(char value) {
                if (value >= '0' && value <= '9') {
                    return value - '0';
                }
                if (value >= 'a' && value <= 'f') {
                    return value - 'a' + 10;
                }
                if (value >= 'A' && value <= 'F') {
                    return value - 'A' + 10;
                }
                return -1;
            }

            bool unicode_escape(std::uint32_t& codepoint) {
                if (position_ + 4 > input_.size()) {
                    return false;
                }
                codepoint = 0;
                for (int index = 0; index < 4; ++index) {
                    const int digit = hex_digit(input_[position_++]);
                    if (digit < 0) {
                        return false;
                    }
                    codepoint = (codepoint << 4) | static_cast<std::uint32_t>(digit);
                }
                return true;
            }

            static void append_utf8(std::string& result, std::uint32_t codepoint) {
                if (codepoint <= 0x7f) {
                    result.push_back(static_cast<char>(codepoint));
                }
                else if (codepoint <= 0x7ff) {
                    result.push_back(static_cast<char>(0xc0 | (codepoint >> 6)));
                    result.push_back(static_cast<char>(0x80 | (codepoint & 0x3f)));
                }
                else if (codepoint <= 0xffff) {
                    result.push_back(static_cast<char>(0xe0 | (codepoint >> 12)));
                    result.push_back(static_cast<char>(0x80 | ((codepoint >> 6) & 0x3f)));
                    result.push_back(static_cast<char>(0x80 | (codepoint & 0x3f)));
                }
                else {
                    result.push_back(static_cast<char>(0xf0 | (codepoint >> 18)));
                    result.push_back(static_cast<char>(0x80 | ((codepoint >> 12) & 0x3f)));
                    result.push_back(static_cast<char>(0x80 | ((codepoint >> 6) & 0x3f)));
                    result.push_back(static_cast<char>(0x80 | (codepoint & 0x3f)));
                }
            }

            bool parse_string(std::string& output) {
                if (!take('"')) {
                    return false;
                }
                while (position_ < input_.size()) {
                    const unsigned char value = static_cast<unsigned char>(input_[position_++]);
                    if (value == '"') {
                        return true;
                    }
                    if (value < 0x20) {
                        return false;
                    }
                    if (value != '\\') {
                        output.push_back(static_cast<char>(value));
                        continue;
                    }
                    if (position_ >= input_.size()) {
                        return false;
                    }
                    const char escaped = input_[position_++];
                    switch (escaped) {
                    case '"':
                        output.push_back('"');
                        break;
                    case '\\':
                        output.push_back('\\');
                        break;
                    case '/':
                        output.push_back('/');
                        break;
                    case 'b':
                        output.push_back('\b');
                        break;
                    case 'f':
                        output.push_back('\f');
                        break;
                    case 'n':
                        output.push_back('\n');
                        break;
                    case 'r':
                        output.push_back('\r');
                        break;
                    case 't':
                        output.push_back('\t');
                        break;
                    case 'u': {
                        std::uint32_t codepoint = 0;
                        if (!unicode_escape(codepoint)) {
                            return false;
                        }
                        if (codepoint >= 0xd800 && codepoint <= 0xdbff) {
                            if (position_ + 2 > input_.size() || input_[position_++] != '\\' ||
                                input_[position_++] != 'u') {
                                return false;
                            }
                            std::uint32_t low = 0;
                            if (!unicode_escape(low) || low < 0xdc00 || low > 0xdfff) {
                                return false;
                            }
                            codepoint = 0x10000 + ((codepoint - 0xd800) << 10) + (low - 0xdc00);
                        }
                        else if (codepoint >= 0xdc00 && codepoint <= 0xdfff) {
                            return false;
                        }
                        append_utf8(output, codepoint);
                        break;
                    }
                    default:
                        return false;
                    }
                }
                return false;
            }

            bool parse_value(JsonValue& output, int depth) {
                if (depth > 16) {
                    return false;
                }
                skip_space();
                if (position_ >= input_.size()) {
                    return false;
                }
                const char first = input_[position_];
                if (first == '"') {
                    output.kind = JsonKind::string;
                    return parse_string(output.string_value);
                }
                if (first == '{') {
                    output.kind = JsonKind::object;
                    ++position_;
                    if (take('}')) {
                        return true;
                    }
                    do {
                        std::string key;
                        JsonValue value;
                        if (!parse_string(key) || !take(':') || !parse_value(value, depth + 1)) {
                            return false;
                        }
                        output.object_values.emplace_back(std::move(key), std::move(value));
                        if (take('}')) {
                            return true;
                        }
                    } while (take(','));
                    return false;
                }
                if (first == '[') {
                    output.kind = JsonKind::array;
                    ++position_;
                    if (take(']')) {
                        return true;
                    }
                    do {
                        JsonValue value;
                        if (!parse_value(value, depth + 1)) {
                            return false;
                        }
                        output.array_values.push_back(std::move(value));
                        if (take(']')) {
                            return true;
                        }
                    } while (take(','));
                    return false;
                }
                if (input_.compare(position_, 4, "true") == 0) {
                    output.kind = JsonKind::boolean;
                    output.boolean_value = true;
                    position_ += 4;
                    return true;
                }
                if (input_.compare(position_, 5, "false") == 0) {
                    output.kind = JsonKind::boolean;
                    output.boolean_value = false;
                    position_ += 5;
                    return true;
                }
                if (input_.compare(position_, 4, "null") == 0) {
                    output.kind = JsonKind::null_value;
                    position_ += 4;
                    return true;
                }
                if (first < '0' || first > '9') {
                    return false;
                }
                output.kind = JsonKind::number;
                while (position_ < input_.size() && input_[position_] >= '0' && input_[position_] <= '9') {
                    const std::uint64_t digit = static_cast<std::uint64_t>(input_[position_++] - '0');
                    if (output.number_value > (std::numeric_limits<std::uint64_t>::max() - digit) / 10) {
                        return false;
                    }
                    output.number_value = output.number_value * 10 + digit;
                }
                return true;
            }
        };

        struct Connection {
            bool valid = false;
            unsigned short port = 0;
            std::string admin_token;
            std::string error;
        };

        enum class RequestKind { refresh, grant, grant_all, equip };

        struct PendingRequest {
            RequestKind kind = RequestKind::refresh;
            std::string weapon_id;
            std::string skin_id;
        };

        std::mutex g_mutex;
        std::condition_variable g_wakeup;
        std::atomic<bool> g_running{false};
        std::thread g_worker;
        SkinState g_state;
        Connection g_connection;
        PendingRequest g_pending;
        bool g_has_pending = false;
        bool g_refresh_requested = true;

        bool is_guid(const std::string& value) {
            if (value.size() != 36) {
                return false;
            }
            for (std::size_t index = 0; index < value.size(); ++index) {
                if (index == 8 || index == 13 || index == 18 || index == 23) {
                    if (value[index] != '-') {
                        return false;
                    }
                }
                else if (!is_hex_digit(value[index])) {
                    return false;
                }
            }
            return true;
        }

        bool parse_state(const std::string& body, SkinState& state) {
            JsonValue root;
            JsonParser parser(body);
            if (!parser.parse(root) || root.kind != JsonKind::object) {
                return false;
            }
            const JsonValue* version = root.field("version");
            const JsonValue* available = root.field("available");
            const JsonValue* equipped = root.field("equipped");
            if (!version || version->kind != JsonKind::number || !available || available->kind != JsonKind::array ||
                available->array_values.size() > 10000 || !equipped || equipped->kind != JsonKind::array ||
                equipped->array_values.size() > 128) {
                return false;
            }
            state.version = version->number_value;
            for (const JsonValue& entry : available->array_values) {
                const JsonValue* weapon_id = entry.field("weapon_id");
                const JsonValue* skin_id = entry.field("skin_id");
                const JsonValue* name = entry.field("name");
                const JsonValue* owned = entry.field("owned");
                if (!weapon_id || weapon_id->kind != JsonKind::string || !is_guid(weapon_id->string_value) ||
                    !skin_id || skin_id->kind != JsonKind::string || !is_guid(skin_id->string_value) || !name ||
                    name->kind != JsonKind::string || name->string_value.size() > 256 || !owned ||
                    owned->kind != JsonKind::boolean) {
                    return false;
                }
                state.available.push_back(
                    {weapon_id->string_value, skin_id->string_value, name->string_value, owned->boolean_value});
            }
            for (const JsonValue& entry : equipped->array_values) {
                const JsonValue* weapon_id = entry.field("weapon_id");
                const JsonValue* skin_id = entry.field("skin_id");
                if (!weapon_id || weapon_id->kind != JsonKind::string || !is_guid(weapon_id->string_value) ||
                    !skin_id || skin_id->kind != JsonKind::string || !is_guid(skin_id->string_value)) {
                    return false;
                }
                state.equipped.push_back({weapon_id->string_value, skin_id->string_value});
            }
            state.connected = true;
            return true;
        }

        Connection read_connection(HMODULE module) {
            Connection connection;
            const auto path = internal::session::runtime_directory() / L"internal.json";
            HANDLE file = CreateFileW(path.c_str(),
                                      GENERIC_READ,
                                      FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                                      nullptr,
                                      OPEN_EXISTING,
                                      FILE_ATTRIBUTE_NORMAL,
                                      nullptr);
            if (file == INVALID_HANDLE_VALUE) {
                connection.error = "No local backend connection file";
                return connection;
            }
            LARGE_INTEGER size = {};
            if (!GetFileSizeEx(file, &size) || size.QuadPart < 2 || size.QuadPart > 4096) {
                CloseHandle(file);
                connection.error = "Invalid connection file size";
                return connection;
            }
            std::string body(static_cast<std::size_t>(size.QuadPart), '\0');
            DWORD bytes_read = 0;
            const bool read_ok = ReadFile(file, body.data(), static_cast<DWORD>(body.size()), &bytes_read, nullptr) &&
                                 bytes_read == body.size();
            CloseHandle(file);
            if (!read_ok) {
                connection.error = "Cannot read connection file";
                return connection;
            }
            JsonValue root;
            JsonParser parser(body);
            if (!parser.parse(root) || root.kind != JsonKind::object) {
                connection.error = "Malformed connection file";
                return connection;
            }
            const JsonValue* host = root.field("host");
            const JsonValue* port = root.field("port");
            const JsonValue* token = root.field("admin_token");
            if (!host || host->kind != JsonKind::string || host->string_value != "127.0.0.1" || !port ||
                port->kind != JsonKind::number || port->number_value == 0 || port->number_value > 65535 || !token ||
                token->kind != JsonKind::string || token->string_value.size() < 32 ||
                token->string_value.size() > 256) {
                connection.error = "Connection file does not name the project-local admin listener";
                return connection;
            }
            for (unsigned char value : token->string_value) {
                if (!std::isalnum(value) && value != '-' && value != '_') {
                    connection.error = "Invalid admin token format";
                    return connection;
                }
            }
            connection.admin_token = token->string_value;
            connection.port = static_cast<unsigned short>(port->number_value);
            connection.valid = true;
            return connection;
        }

        struct HttpResponse {
            bool received = false;
            DWORD status_code = 0;
            std::string body;
            std::string error;
        };

        HttpResponse send_request(const Connection& connection,
                                  RequestKind kind,
                                  const std::string& weapon_id,
                                  const std::string& skin_id) {
            HttpResponse response;
            const wchar_t* method = kind == RequestKind::refresh ? L"GET"
                                    : kind == RequestKind::equip ? L"PUT"
                                                                 : L"POST";
            const wchar_t* path = kind == RequestKind::refresh     ? L"/dev/skins"
                                  : kind == RequestKind::grant     ? L"/dev/skins/grant"
                                  : kind == RequestKind::grant_all ? L"/dev/skins/grant-all"
                                                                   : L"/dev/skins/equip";
            HINTERNET session = WinHttpOpen(L"OfflineDevOverlay/1.0",
                                            WINHTTP_ACCESS_TYPE_NO_PROXY,
                                            WINHTTP_NO_PROXY_NAME,
                                            WINHTTP_NO_PROXY_BYPASS,
                                            0);
            if (!session) {
                response.error = "Could not open local HTTP session";
                return response;
            }
            WinHttpSetTimeouts(session, 1500, 1500, 1500, 1500);
            HINTERNET connection_handle = WinHttpConnect(session, L"127.0.0.1", connection.port, 0);
            HINTERNET request =
                connection_handle
                    ? WinHttpOpenRequest(
                          connection_handle, method, path, nullptr, WINHTTP_NO_REFERER, WINHTTP_DEFAULT_ACCEPT_TYPES, 0)
                    : nullptr;
            if (!request) {
                response.error = "Could not connect to local admin listener";
            }
            else {
                DWORD no_redirects = WINHTTP_DISABLE_REDIRECTS;
                if (!WinHttpSetOption(request, WINHTTP_OPTION_DISABLE_FEATURE, &no_redirects, sizeof(no_redirects))) {
                    response.error = "Cannot confine local admin request: redirects remain enabled";
                }
                else {
                    const std::wstring auth =
                        L"Authorization: Bearer " +
                        std::wstring(connection.admin_token.begin(), connection.admin_token.end()) + L"\r\n";
                    const bool auth_added =
                        WinHttpAddRequestHeaders(
                            request, auth.c_str(), static_cast<DWORD>(-1L), WINHTTP_ADDREQ_FLAG_ADD) != 0;
                    const bool content_type_added =
                        kind == RequestKind::refresh || WinHttpAddRequestHeaders(request,
                                                                                 L"Content-Type: application/json\r\n",
                                                                                 static_cast<DWORD>(-1L),
                                                                                 WINHTTP_ADDREQ_FLAG_ADD) != 0;
                    std::string request_body;
                    if (kind == RequestKind::grant) {
                        request_body = "{\"skin_id\":\"" + skin_id + "\"}";
                    }
                    if (kind == RequestKind::equip) {
                        request_body = "{\"weapon_id\":\"" + weapon_id + "\",\"skin_id\":\"" + skin_id + "\"}";
                    }
                    const bool sent =
                        auth_added && content_type_added &&
                        WinHttpSendRequest(request,
                                           WINHTTP_NO_ADDITIONAL_HEADERS,
                                           0,
                                           request_body.empty() ? WINHTTP_NO_REQUEST_DATA : request_body.data(),
                                           static_cast<DWORD>(request_body.size()),
                                           static_cast<DWORD>(request_body.size()),
                                           0) &&
                        WinHttpReceiveResponse(request, nullptr);
                    if (!sent) {
                        response.error = "Local admin request failed";
                    }
                    else {
                        DWORD size = sizeof(response.status_code);
                        if (!WinHttpQueryHeaders(request,
                                                 WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                                                 WINHTTP_HEADER_NAME_BY_INDEX,
                                                 &response.status_code,
                                                 &size,
                                                 WINHTTP_NO_HEADER_INDEX)) {
                            response.error = "Local admin response lacks HTTP status";
                        }
                        else {
                            response.received = true;
                            char buffer[4096];
                            DWORD bytes_read = 0;
                            BOOL read_ok = FALSE;
                            while ((read_ok = WinHttpReadData(request, buffer, sizeof(buffer), &bytes_read)) &&
                                   bytes_read) {
                                if (response.body.size() + bytes_read > 4194304) {
                                    response.received = false;
                                    response.error = "Local admin response too large";
                                    break;
                                }
                                response.body.append(buffer, bytes_read);
                            }
                            if (!read_ok && response.error.empty()) {
                                response.received = false;
                                response.error = "Local admin response interrupted";
                            }
                        }
                    }
                }
            }
            if (request) {
                WinHttpCloseHandle(request);
            }
            if (connection_handle) {
                WinHttpCloseHandle(connection_handle);
            }
            WinHttpCloseHandle(session);
            return response;
        }

        void worker_loop() {
            while (g_running.load()) {
                PendingRequest pending;
                {
                    std::unique_lock lock(g_mutex);
                    g_wakeup.wait_for(lock, std::chrono::seconds(2), [] {
                        return !g_running.load() || g_has_pending || g_refresh_requested;
                    });
                    if (!g_running.load()) {
                        return;
                    }
                    if (g_has_pending) {
                        pending = std::move(g_pending);
                        g_has_pending = false;
                    }
                    else {
                        pending.kind = RequestKind::refresh;
                    }
                    g_refresh_requested = false;
                    g_state.request_pending = true;
                }

                const HttpResponse response =
                    send_request(g_connection, pending.kind, pending.weapon_id, pending.skin_id);
                SkinState next_state;
                const bool valid_state =
                    response.received && response.status_code == 200 && parse_state(response.body, next_state);
                std::string error;
                if (!response.received) {
                    error = response.error;
                }
                else if (response.status_code != 200) {
                    JsonValue root;
                    JsonParser parser(response.body);
                    const JsonValue* server_error = parser.parse(root) ? root.field("error") : nullptr;
                    error = server_error && server_error->kind == JsonKind::string
                                ? server_error->string_value.substr(0, 200)
                                : "Local admin HTTP " + std::to_string(response.status_code);
                }
                else if (!valid_state) {
                    error = "Local admin response has an invalid skin schema";
                }
                {
                    std::lock_guard lock(g_mutex);
                    if (valid_state) {
                        g_state.available = std::move(next_state.available);
                        g_state.equipped = std::move(next_state.equipped);
                        g_state.version = next_state.version;
                        g_state.connected = true;
                        g_state.status = pending.kind == RequestKind::refresh
                                             ? "Local backend skin state loaded"
                                             : "Saved locally; restart the client to refresh Collection ownership";
                        if (pending.kind != RequestKind::refresh) {
                            g_state.last_action_error.clear();
                        }
                    }
                    else {
                        if (pending.kind == RequestKind::refresh) {
                            g_state.connected = false;
                            g_state.status = error;
                        }
                        else {
                            g_state.last_action_error = error;
                            g_state.status = "Local backend rejected the skin action";
                            if (!response.received || response.status_code == 401 || response.status_code >= 500) {
                                g_state.connected = false;
                            }
                        }
                    }
                    g_state.request_pending = false;
                }
            }
        }
    } // namespace detail

    void initialize(HMODULE module) {
        std::lock_guard lock(detail::g_mutex);
        if (detail::g_running.load()) {
            return;
        }
        detail::g_connection = detail::read_connection(module);
        if (!detail::g_connection.valid) {
            detail::g_state.status = detail::g_connection.error;
            return;
        }
        detail::g_state.status = "Connecting to local backend skin service";
        detail::g_refresh_requested = true;
        detail::g_running = true;
        detail::g_worker = std::thread(detail::worker_loop);
    }

    void shutdown() {
        detail::g_running = false;
        detail::g_wakeup.notify_one();
        if (detail::g_worker.joinable()) {
            detail::g_worker.join();
        }
        std::lock_guard lock(detail::g_mutex);
        detail::g_connection.admin_token.clear();
    }

    SkinState snapshot() {
        std::lock_guard lock(detail::g_mutex);
        return detail::g_state;
    }

    void request_refresh() {
        if (!detail::g_running.load()) {
            return;
        }
        {
            std::lock_guard lock(detail::g_mutex);
            detail::g_refresh_requested = true;
        }
        detail::g_wakeup.notify_one();
    }

    bool request_grant(const std::string& skin_id) {
        std::lock_guard lock(detail::g_mutex);
        if (!detail::g_running.load() || detail::g_has_pending || detail::g_state.request_pending ||
            !detail::is_guid(skin_id)) {
            return false;
        }
        const auto entry = std::find_if(
            detail::g_state.available.begin(), detail::g_state.available.end(), [&](const SkinRecord& record) {
                return record.skin_id == skin_id && !record.owned;
            });
        if (entry == detail::g_state.available.end()) {
            return false;
        }
        detail::g_pending = {detail::RequestKind::grant, "", skin_id};
        detail::g_has_pending = true;
        detail::g_wakeup.notify_one();
        return true;
    }

    bool request_grant_all() {
        std::lock_guard lock(detail::g_mutex);
        if (!detail::g_running.load() || detail::g_has_pending || detail::g_state.request_pending ||
            !detail::g_state.connected || detail::g_state.available.empty()) {
            return false;
        }
        detail::g_pending = {detail::RequestKind::grant_all, "", ""};
        detail::g_has_pending = true;
        detail::g_wakeup.notify_one();
        return true;
    }

    bool request_equip(const std::string& weapon_id, const std::string& skin_id) {
        std::lock_guard lock(detail::g_mutex);
        if (!detail::g_running.load() || detail::g_has_pending || detail::g_state.request_pending ||
            !detail::is_guid(weapon_id) || !detail::is_guid(skin_id)) {
            return false;
        }
        const auto entry = std::find_if(
            detail::g_state.available.begin(), detail::g_state.available.end(), [&](const SkinRecord& record) {
                return record.weapon_id == weapon_id && record.skin_id == skin_id && record.owned;
            });
        if (entry == detail::g_state.available.end()) {
            return false;
        }
        detail::g_pending = {detail::RequestKind::equip, weapon_id, skin_id};
        detail::g_has_pending = true;
        detail::g_wakeup.notify_one();
        return true;
    }
} // namespace skin_client
