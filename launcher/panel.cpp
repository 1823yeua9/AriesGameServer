#include "panel.h"
#include "layout.h"
#include "application.h"
#include "widgets.h"
#include "../loader/src/utils/config/settings.h"
#include "../internal/overlay/vendor/imgui/imgui.h"
#include "../internal/overlay/vendor/imgui/backends/imgui_impl_dx11.h"
#include "../internal/overlay/vendor/imgui/backends/imgui_impl_win32.h"

#include <commdlg.h>
#include <shellapi.h>
#include <fstream>

namespace launcher {
    static auto path_text(const std::filesystem::path& path) -> std::string {
        const auto value = path.u8string();

        return {value.begin(), value.end()};
    }

    Panel::Panel(HWND window_value, ID3D11Device* device, ID3D11DeviceContext* context_value)
        : window(window_value), context(context_value) {
        root = executable_directory();

        // Search ancestors, not the process working directory or a hardcoded user path.
        while (!std::filesystem::exists(root / "AriesGameServer.slnx") && root.has_parent_path() &&
               root != root.parent_path()) {
            root = root.parent_path();
        }

        if (!std::filesystem::exists(root / "AriesGameServer.slnx")) {
            root = executable_directory();
        }

        config_path = root / "aries.val";
        load_username();
        loader_path = executable_directory() / "loader.exe";

        if (!std::filesystem::is_regular_file(loader_path)) {
            loader_path = root / "x64/Release/loader.exe";
        }

        IMGUI_CHECKVERSION();
        ImGui::CreateContext();
        auto& io = ImGui::GetIO();
        io.IniFilename = nullptr;
        io.ConfigFlags |= ImGuiConfigFlags_NavEnableKeyboard;

        wchar_t windows_directory[32768]{};
        GetWindowsDirectoryW(windows_directory, 32768);
        const auto fonts = std::filesystem::path(windows_directory) / "Fonts";
        const auto regular = path_text(fonts / "segoeui.ttf");
        const auto semibold =
            path_text(fonts / (std::filesystem::exists(fonts / "seguisb.ttf") ? "seguisb.ttf" : "segoeui.ttf"));
        ImFontConfig config;
        config.OversampleH = 3;
        config.OversampleV = 2;
        // the first font added is the default
        widgets::fonts.body = io.Fonts->AddFontFromFileTTF(regular.c_str(), 15, &config);
        widgets::fonts.caption = io.Fonts->AddFontFromFileTTF(regular.c_str(), 13, &config);
        widgets::fonts.label = io.Fonts->AddFontFromFileTTF(semibold.c_str(), 12, &config);
        widgets::fonts.button = io.Fonts->AddFontFromFileTTF(semibold.c_str(), 15, &config);
        widgets::fonts.heading = io.Fonts->AddFontFromFileTTF(semibold.c_str(), 18, &config);
        widgets::fonts.title = io.Fonts->AddFontFromFileTTF(semibold.c_str(), 26, &config);
        ImGui::StyleColorsDark();
        auto& style = ImGui::GetStyle();
        style.WindowPadding = {0, 0};
        style.FramePadding = {14, 10};
        style.ItemSpacing = {10, 10};
        style.FrameRounding = 5;
        style.ChildRounding = 10;
        style.PopupRounding = 10;
        style.WindowRounding = 10;
        style.WindowBorderSize = 0;
        style.PopupBorderSize = 1;
        style.WindowTitleAlign = {0.5f, 0.5f};
        style.Colors[ImGuiCol_Border] = {1, 1, 1, 0.08f};
        style.Colors[ImGuiCol_Text] = {0.92f, 0.93f, 0.91f, 1};
        style.Colors[ImGuiCol_TextDisabled] = {0.59f, 0.63f, 0.62f, 1};
        style.Colors[ImGuiCol_FrameBg] = {0x39 / 255.0f, 0x3e / 255.0f, 0x41 / 255.0f, 1};
        style.Colors[ImGuiCol_FrameBgHovered] = {0x45 / 255.0f, 0x4b / 255.0f, 0x4f / 255.0f, 1};
        style.Colors[ImGuiCol_FrameBgActive] = {0x45 / 255.0f, 0x4b / 255.0f, 0x4f / 255.0f, 1};
        style.Colors[ImGuiCol_TextSelectedBg] = {0x54 / 255.0f, 0xa4 / 255.0f, 1, 0.40f};
        style.Colors[ImGuiCol_NavHighlight] = {0, 0, 0, 0};
        style.Colors[ImGuiCol_PopupBg] = {0x18 / 255.0f, 0x1a / 255.0f, 0x1b / 255.0f, 1};
        style.Colors[ImGuiCol_Border] = {0x54 / 255.0f, 0xa4 / 255.0f, 1, 0.35f};
        style.Colors[ImGuiCol_ModalWindowDimBg] = {0, 0, 0, 0.55f};
        style.Colors[ImGuiCol_ScrollbarBg] = {0, 0, 0, 0};
        style.Colors[ImGuiCol_ScrollbarGrab] = {1, 1, 1, 0.12f};
        style.Colors[ImGuiCol_ScrollbarGrabHovered] = {1, 1, 1, 0.20f};
        style.Colors[ImGuiCol_ScrollbarGrabActive] = {1, 1, 1, 0.26f};

        if (!ImGui_ImplWin32_Init(window) || !ImGui_ImplDX11_Init(device, context.Get())) {
            throw std::runtime_error("Cannot initialize launcher UI");
        }
    }

    Panel::~Panel() {
        stop();
        ImGui_ImplDX11_Shutdown();
        ImGui_ImplWin32_Shutdown();
        ImGui::DestroyContext();
    }

    auto Panel::running() const -> bool {
        return process && process->running();
    }

    auto Panel::stop() -> void {
        // This job contains only the loader started by this panel. The loader's
        // nested job owns its servers and game; no name-based process killing.
        job.reset();
        process.reset();
        preflight_ok = false;
        status = "Session closed.";
    }

    auto Panel::start(const std::wstring& mode) -> void {
        if (running()) {
            return;
        }

        try {
            process.reset();
            job.reset();
            preflight_ok = false;

            if (!std::filesystem::is_regular_file(loader_path)) {
                throw std::runtime_error("Build x64 Release loader first; its executable is missing.");
            }

            if (mode != L"--write-config" && !std::filesystem::is_regular_file(config_path)) {
                throw std::runtime_error("Choose a settings.val file or create the default profile in Settings.");
            }

            if (mode.empty() && !save_username()) {
                return;
            }

            const auto directory = root / "runtime/launcher";
            std::filesystem::create_directories(directory);
            log_path = directory / (loader::init::random_token() + ".log");
            std::vector<std::wstring> arguments;

            if (mode == L"--write-config") {
                arguments = {mode, config_path.wstring()};
            }
            else {
                arguments = {L"--config", config_path.wstring()};

                if (!mode.empty()) {
                    arguments.push_back(mode);
                }
            }

            job.emplace();
            process.emplace(job->start(loader_path, arguments, root, true, log_path));
            current_mode = mode;
            status = mode.empty() ? "Loader starting the local session..." : "Checking local configuration...";
            log_text.clear();
        }
        catch (const std::exception& error) {
            status = error.what();
        }
    }

    auto Panel::poll() -> void {
        if (GetTickCount64() - last_poll < 250) {
            return;
        }

        last_poll = GetTickCount64();
        profile_found = std::filesystem::is_regular_file(config_path);
        loader_found = std::filesystem::is_regular_file(loader_path);

        if (!log_path.empty()) {
            std::ifstream file(log_path, std::ios::binary);

            if (file) {
                file.seekg(0, std::ios::end);
                const auto length = static_cast<std::streamoff>(file.tellg());
                file.seekg(std::max<std::streamoff>(0, length - 16000));
                log_text.assign(std::istreambuf_iterator<char>(file), {});
            }
        }

        if (process && !process->running()) {
            DWORD code = 1;
            GetExitCodeProcess(process->process.get(), &code);
            preflight_ok = code == 0 && current_mode == L"--check";
            status =
                code == 0
                    ? (preflight_ok ? "Preflight passed. Ready to launch."
                       : current_mode == L"--install-firewall"
                           ? "Offline firewall setup verified. Run Check setup next."
                       : current_mode == L"--write-config" ? "Profile created. Edit paths in Settings, then check."
                                                           : "Session ended.")
                : current_mode == L"--install-firewall"
                    ? "Firewall setup failed. Run the loader's --install-firewall mode in an Administrator terminal for details."
                    : "Loader stopped with an error. Open Logs for the exact reason.";
            process.reset();
            job.reset();
        }
        else if (running() && current_mode.empty()) {
            status = log_text.find("MainMenuV2 observed") != std::string::npos
                         ? "Main menu reached. Local session running."
                     : log_text.find("Platform ready") != std::string::npos ? "Platform ready. Initializing the game..."
                                                                            : "Starting local services...";
        }
    }

    auto Panel::browse_config() -> void {
        wchar_t path[32768]{};
        OPENFILENAMEW dialog{sizeof(dialog)};
        dialog.hwndOwner = window;
        dialog.lpstrFilter = L"Aries profile (*.val;*.json)\0*.val;*.json\0All files\0*.*\0";
        dialog.lpstrFile = path;
        dialog.nMaxFile = 32768;
        dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;

        if (GetOpenFileNameW(&dialog)) {
            config_path = path;
            load_username();
            preflight_ok = false;
            status = "Profile selected. Check it before launching.";
        }
    }

    auto Panel::load_username() -> void {
        loader::config::Settings profile(config_path);
        const auto name = profile.load_settings() ? profile.get_setting("username") : "OfflinePlayer";
        if (name.size() < sizeof(username)) {
            strcpy_s(username, name.c_str());
        }
        else {
            strcpy_s(username, "OfflinePlayer");
            status = "Profile username is too long. Choose a shorter name.";
        }
    }

    auto Panel::save_username() -> bool {
        wchar_t decoded[129]{};
        const auto length = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, username, -1, decoded, 129);
        size_t characters = 0;
        bool invalid = length <= 1;
        for (int index = 0; index < length - 1; ++index) {
            const auto character = decoded[index];
            characters += !(character >= 0xdc00 && character <= 0xdfff);
            invalid = invalid || iswcntrl(character) || character == L'#';
        }
        invalid = invalid || characters > 32 || (length > 1 && (iswspace(decoded[0]) || iswspace(decoded[length - 2])));

        if (invalid) {
            status = "Use 1-32 characters, without #, control characters, or surrounding whitespace.";
            return false;
        }

        loader::config::Settings profile(config_path);
        if (!profile.load_settings()) {
            status = "Choose a valid profile before saving your username.";
            return false;
        }

        profile.set_setting("username", username);
        if (!profile.save_settings()) {
            status = "Could not save the username. Your profile was not replaced.";
            return false;
        }

        preflight_ok = false;
        status = "Username saved. It applies on the next game launch.";
        return true;
    }

    auto Panel::install_containment() -> void {
        if (running() || !std::filesystem::is_regular_file(loader_path) ||
            !std::filesystem::is_regular_file(config_path)) {
            status = "A built loader and valid profile are required for setup.";
            return;
        }

        // Elevation is requested only by this explicit button, for this bounded
        // setup mode. Normal game launches never silently elevate.
        const auto arguments =
            L"--config " + loader::init::quote_argument(config_path.wstring()) + L" --install-firewall";
        SHELLEXECUTEINFOW request{sizeof(request)};
        request.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
        request.hwnd = window;
        request.lpVerb = L"runas";
        request.lpFile = loader_path.c_str();
        request.lpParameters = arguments.c_str();
        request.lpDirectory = root.c_str();
        request.nShow = SW_HIDE;

        if (!ShellExecuteExW(&request) || !request.hProcess) {
            status = "Firewall setup was canceled or could not start. No game was launched.";
            return;
        }

        process.reset();
        job.reset();
        process.emplace(loader::init::Process{
            loader::init::Handle(request.hProcess), loader::init::Handle{}, GetProcessId(request.hProcess)});
        current_mode = L"--install-firewall";
        log_path.clear();
        log_text.clear();
        status = "Installing scoped loopback-only rules. No game is being launched.";
    }

    // spt's MainWindow: 760 x 450, a slim title bar, 10-12 px margins
    constexpr float WIDTH = 760;
    constexpr float HEIGHT = 450;
    constexpr float BAR = 32;
    constexpr float MARGIN = 12;

    /// how far the launch has come, for the launch bar's progress (spt: xp to next level)
    auto Panel::phase() const -> float {
        if (!running() || !current_mode.empty()) {
            return preflight_ok ? 1.0f : 0.0f;
        }
        if (log_text.find("MainMenuV2 observed") != std::string::npos) {
            return 1.0f;
        }
        if (log_text.find("Platform ready") != std::string::npos) {
            return 0.66f;
        }
        return 0.33f;
    }

    auto Panel::title_bar() -> void {
        using namespace widgets;
        auto* draw = ImGui::GetWindowDrawList();
        draw->AddRectFilled({0, 0}, {WIDTH, BAR}, palette::card());
        label("Aries Launcher", {10, 0}, {200, BAR}, palette::tan(), fonts.body);

        constexpr float window_button = 35;
        const auto right = WIDTH - window_button * 2 - 10;
        if (button_at("Logs", {right - 170, 0}, {64, BAR}, ButtonStyle::Link)) {
            back_page = page < 2 ? page : back_page;
            page = 3;
        }
        if (button_at("Settings", {right - 100, 0}, {96, BAR}, ButtonStyle::Link, Icon::Gear)) {
            back_page = page < 2 ? page : back_page;
            page = 2;
        }
        if (button_at("Minimize", {WIDTH - window_button * 2, 0}, {window_button, BAR}, ButtonStyle::Window, Icon::Minimize)) {
            ShowWindow(window, SW_MINIMIZE);
        }
        if (button_at("Close", {WIDTH - window_button, 0}, {window_button, BAR}, ButtonStyle::Close, Icon::Close)) {
            PostMessageW(window, WM_CLOSE, 0, 0);
        }

        // spt's version tag: an accent label hanging under the bar
        const auto* tag = "13.06";
        const auto width = text_width(tag, fonts.label) + 20;
        const ImVec2 start{std::floor(WIDTH - window_button * 2 - 10 - width), BAR};
        draw->AddRectFilled(start, {start.x + width, start.y + 20}, palette::accent(), 5, ImDrawFlags_RoundCornersBottom);
        text_in(tag, start, {width, 20}, color(0x2f, 0x4f, 0x4f), fonts.label);
    }

    /// spt LoginView: the profile card in the middle, the login box under it.
    auto Panel::login() -> void {
        using namespace widgets;
        constexpr float card_width = 236;
        constexpr float card_height = 262;
        constexpr float card_x = (WIDTH - card_width) / 2;
        constexpr float card_y = BAR + 30;

        // the profile card (spt DetailedProfileCard): click it to continue
        ImGui::SetCursorPos({card_x, card_y});
        const auto chosen = ImGui::InvisibleButton("profile_card", {card_width, card_height});
        const auto hovered = ImGui::IsItemHovered();
        const auto lift = hovered ? -3.0f : 0.0f;
        card({card_x, card_y + lift}, {card_width, card_height}, hovered);
        const auto top = card_y + lift;
        label("Aries 13.06", {card_x, top + 12}, {card_width, 20}, palette::tan(), fonts.label, Align::Center);

        if (profile_found) {
            avatar(username, {card_x + 18, top + 44}, 52);
            label(username, {card_x + 82, top + 44}, {card_width - 96, 26}, palette::ink(), fonts.heading);
            label("Jett  \xC2\xB7  Range", {card_x + 82, top + 70}, {card_width - 96, 22}, palette::accent(), fonts.body);
            auto* draw = ImGui::GetWindowDrawList();
            const auto origin = window_origin();
            draw->AddLine({origin.x + card_x + 18, origin.y + top + 112}, {origin.x + card_x + card_width - 18, origin.y + top + 112}, color(255, 255, 255, 18));
            indicator("Profile found", profile_found, {card_x + 22, top + 128});
            indicator(loader_found ? "Loader built" : "Loader not built", loader_found, {card_x + 22, top + 152});
            indicator(preflight_ok ? "Setup checked" : "Setup not checked", preflight_ok, {card_x + 22, top + 176});
            label(running() ? "Session running" : "Click to continue", {card_x, top + card_height - 40}, {card_width, 22}, running() ? palette::good() : palette::tan_dim(), fonts.caption, Align::Center);
        }
        else {
            label("No profile data", {card_x, top}, {card_width, card_height}, palette::tan(), fonts.button, Align::Center);
        }

        if (chosen) {
            if (profile_found) {
                page = 1;
            }
            else {
                back_page = 0;
                page = 2;
            }
        }

        // the login box (spt LoginBox): username, then the accent button
        constexpr float box_width = 420;
        constexpr float box_height = 64;
        constexpr float box_x = (WIDTH - box_width) / 2;
        constexpr float box_y = HEIGHT - box_height - 24;
        card({box_x, box_y}, {box_width, box_height});
        ImGui::BeginDisabled(running());
        ImGui::SetCursorPos({box_x + 14, box_y + 14});
        ImGui::SetNextItemWidth(box_width - 14 * 2 - 10 - 130);
        ImGui::PushFont(fonts.body);
        ImGui::PushStyleVar(ImGuiStyleVar_FramePadding, {10, 9});
        ImGui::InputTextWithHint("##local_username", "Username", username, sizeof(username));
        ImGui::PopStyleVar();
        ImGui::PopFont();
        if (button_at("Login", {box_x + box_width - 14 - 130, box_y + 14}, {130, 36}, ButtonStyle::Accent) && save_username()) {
            page = 1;
        }
        ImGui::EndDisabled();
    }

    /// spt ProfileView: profile card and the setup card on the left, the launch bar along the bottom.
    auto Panel::home() -> void {
        using namespace widgets;
        constexpr float top = BAR + 28;
        constexpr float left_width = 300;

        // profile card (spt ProfileCard)
        card({MARGIN, top}, {left_width, 176});
        avatar(username, {MARGIN + 14, top + 14}, 44);
        label(username, {MARGIN + 70, top + 12}, {left_width - 84, 24}, palette::ink(), fonts.heading);
        label("Offline  \xC2\xB7  Build 13.06", {MARGIN + 70, top + 36}, {left_width - 84, 20}, palette::accent(), fonts.caption);
        const auto profile = path_text(config_path.filename());
        label("Profile", {MARGIN + 16, top + 74}, {80, 20}, palette::tan(), fonts.caption);
        label(profile_found ? profile.c_str() : "missing", {MARGIN + 96, top + 74}, {left_width - 112, 20}, profile_found ? palette::soft() : palette::bad(), fonts.caption);
        label("Server", {MARGIN + 16, top + 96}, {80, 20}, palette::tan(), fonts.caption);
        label("127.0.0.1 (loopback only)", {MARGIN + 96, top + 96}, {left_width - 112, 20}, palette::soft(), fonts.caption);
        ImGui::BeginDisabled(running());
        if (button_at("Change profile", {MARGIN + 16, top + 134}, {112, 28}, ButtonStyle::Link)) {
            browse_config();
        }
        ImGui::BeginDisabled(!profile_found);
        if (button_at("Edit profile", {MARGIN + 136, top + 134}, {96, 28}, ButtonStyle::Link)) {
            back_page = 1;
            page = 2;
        }
        ImGui::EndDisabled();
        ImGui::EndDisabled();

        // setup card (spt TotalModsCard)
        constexpr float setup_top = top + 176 + 12;
        card({MARGIN, setup_top}, {left_width, 84});
        label(preflight_ok ? "Setup checked" : "Setup not checked", {MARGIN + 16, setup_top + 12}, {left_width - 32, 24}, preflight_ok ? palette::good() : palette::ink(), fonts.heading);
        indicator("Profile", profile_found, {MARGIN + 16, setup_top + 46});
        indicator("Loader", loader_found, {MARGIN + 96, setup_top + 46});
        ImGui::BeginDisabled(running());
        if (button_at("Run check", {MARGIN + left_width - 100, setup_top + 40}, {88, 28}, ButtonStyle::Link)) {
            start(L"--check");
        }
        ImGui::EndDisabled();

        // status card on the right, where spt shows its background art
        constexpr float right = MARGIN + left_width + 12;
        constexpr float right_width = WIDTH - right - MARGIN;
        card({right, top}, {right_width, 272});
        label("SESSION", {right + 16, top + 14}, {right_width - 32, 18}, palette::tan(), fonts.label);
        label(running() ? "Running" : preflight_ok ? "Ready to launch" : "Idle", {right + 16, top + 34}, {right_width - 32, 30}, running() ? palette::good() : palette::ink(), fonts.title);
        label(status.c_str(), {right + 16, top + 70}, {right_width - 32, 40}, palette::soft(), fonts.caption);
        auto* draw = ImGui::GetWindowDrawList();
        const auto origin = window_origin();
        draw->AddRectFilled({origin.x + right + 16, origin.y + top + 118}, {origin.x + right + right_width - 16, origin.y + top + 258}, color(0, 0, 0, 90), 5);
        // the newest loader lines, newest last
        std::string tail = log_text.empty() ? std::string("No launch yet.") : log_text;
        size_t cut = tail.size();
        for (int lines = 0; lines < 7 && cut > 0; ++lines) {
            const auto previous = tail.rfind('\n', cut - 1);
            if (previous == std::string::npos) {
                cut = 0;
                break;
            }
            cut = previous == tail.size() - 1 && lines == 0 ? previous : previous;
        }
        tail = tail.substr(cut == 0 ? 0 : cut + 1);
        ImGui::SetCursorPos({right + 24, top + 124});
        ImGui::PushFont(fonts.caption);
        ImGui::PushStyleColor(ImGuiCol_Text, ImGui::ColorConvertU32ToFloat4(palette::tan()));
        ImGui::PushTextWrapPos(right + right_width - 24);
        ImGui::TextUnformatted(tail.c_str());
        ImGui::PopTextWrapPos();
        ImGui::PopStyleColor();
        ImGui::PopFont();

        launch_bar();
    }

    /// spt GameLaunchBar: who is playing and how far along, then logout and start.
    auto Panel::launch_bar() -> void {
        using namespace widgets;
        constexpr float height = 70;
        constexpr float top = HEIGHT - height - MARGIN;
        constexpr float width = WIDTH - MARGIN * 2;
        card({MARGIN, top}, {width, height});
        avatar(username, {MARGIN + 12, top + 13}, 44);
        label(username, {MARGIN + 66, top + 14}, {150, 22}, palette::accent(), fonts.button);
        label("Jett  \xC2\xB7  Range", {MARGIN + 66, top + 36}, {150, 20}, palette::soft(), fonts.caption);

        const auto progress_value = phase();
        const auto* stage = !running() ? (preflight_ok ? "Checked" : "Idle") : progress_value >= 1 ? "In menu" : progress_value >= 0.66f ? "Platform" : "Services";
        label(stage, {MARGIN + 232, top + 10}, {200, 20}, palette::tan(), fonts.caption, Align::Center);
        progress({MARGIN + 232, top + 32}, {200, 8}, progress_value);
        label(running() ? "starting the game" : "next: start game", {MARGIN + 232, top + 44}, {200, 20}, palette::tan_dim(), fonts.caption, Align::Center);

        if (button_at("Logout", {MARGIN + width - 12 - 150 - 80, top + 20}, {70, 30}, ButtonStyle::Link) && !running()) {
            page = 0;
        }
        ImGui::SetCursorPos({MARGIN + width - 12 - 150, top + 10});
        if (running()) {
            if (button("Stop game", {150, height - 20}, ButtonStyle::Accent)) {
                ImGui::OpenPopup("Stop this session?");
            }
        }
        else if (button("Start Game", {150, height - 20}, ButtonStyle::Accent)) {
            start(L"");
        }

        ImGui::SetNextWindowSize({420, 0});
        ImGui::SetNextWindowPos(ImGui::GetMainViewport()->GetCenter(), ImGuiCond_Appearing, {0.5f, 0.5f});
        ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, {20, 20});
        ImGui::PushStyleVar(ImGuiStyleVar_WindowRounding, 5);
        if (ImGui::BeginPopupModal("Stop this session?", nullptr, ImGuiWindowFlags_NoTitleBar | ImGuiWindowFlags_AlwaysAutoResize)) {
            const auto popup = ImGui::GetCursorScreenPos();
            text_in("Stop this session?", popup, {380, 28}, palette::ink(), fonts.heading, Align::Center);
            text_in("This closes the game and the servers this launcher started.", {popup.x, popup.y + 30}, {380, 36}, palette::tan(), fonts.caption, Align::Center);
            ImGui::Dummy({380, 76});
            ImGui::SetCursorPosX((ImGui::GetWindowWidth() - 256) / 2);
            if (button("Stop", {120, 36}, ButtonStyle::Accent)) {
                stop();
                ImGui::CloseCurrentPopup();
            }
            ImGui::SameLine(0, 16);
            if (button("Cancel", {120, 36})) {
                ImGui::CloseCurrentPopup();
            }
            ImGui::EndPopup();
        }
        ImGui::PopStyleVar(2);
    }

    /// spt SettingsView: a full page with a way back.
    auto Panel::settings() -> void {
        using namespace widgets;
        constexpr float top = BAR + 28;
        constexpr float width = WIDTH - MARGIN * 2;
        card({MARGIN, top}, {width, HEIGHT - top - MARGIN});
        if (button_at("< Go back", {MARGIN + 10, top + 10}, {84, 26}, ButtonStyle::Link)) {
            page = back_page;
        }
        label("Settings", {MARGIN, top + 10}, {width, 26}, palette::ink(), fonts.heading, Align::Center);

        // profile file
        label("PROFILE FILE", {MARGIN + 24, top + 56}, {width - 48, 18}, palette::tan(), fonts.label);
        label(path_text(config_path).c_str(), {MARGIN + 24, top + 76}, {width - 48, 22}, profile_found ? palette::soft() : palette::bad(), fonts.caption);
        ImGui::BeginDisabled(running());
        if (button_at("Choose profile", {MARGIN + 24, top + 106}, {150, 34})) {
            browse_config();
        }
        ImGui::BeginDisabled(!profile_found);
        if (button_at("Edit in Notepad", {MARGIN + 24 + 160, top + 106}, {150, 34})) {
            // open a known system editor; never the .val file association
            wchar_t system_directory[32768]{};
            GetSystemDirectoryW(system_directory, 32768);
            const auto editor = std::filesystem::path(system_directory) / L"notepad.exe";
            const auto argument = loader::init::quote_argument(config_path.wstring());
            ShellExecuteW(window, L"open", editor.c_str(), argument.c_str(), root.c_str(), SW_SHOW);
        }
        ImGui::EndDisabled();
        ImGui::BeginDisabled(profile_found);
        if (button_at("Create profile", {MARGIN + 24 + 320, top + 106}, {150, 34})) {
            start(L"--write-config");
        }
        ImGui::EndDisabled();

        auto* draw = ImGui::GetWindowDrawList();
        const auto origin = window_origin();
        draw->AddLine({origin.x + MARGIN + 24, origin.y + top + 162}, {origin.x + MARGIN + width - 24, origin.y + top + 162}, color(255, 255, 255, 18));

        // firewall
        label("OFFLINE FIREWALL", {MARGIN + 24, top + 178}, {width - 48, 18}, palette::tan(), fonts.label);
        label("Blocks the game and its crash reporter from the internet. Needs administrator approval once.",
              {MARGIN + 24, top + 198}, {width - 48, 22}, palette::soft(), fonts.caption);
        if (button_at("Set up offline firewall", {MARGIN + 24, top + 228}, {220, 34})) {
            install_containment();
        }
        if (button_at("Check setup", {MARGIN + 24 + 230, top + 228}, {150, 34})) {
            start(L"--check");
        }
        ImGui::EndDisabled();
        label(status.c_str(), {MARGIN + 24, top + 288}, {width - 48, 40}, palette::accent(), fonts.caption);
    }

    auto Panel::logs() -> void {
        using namespace widgets;
        constexpr float top = BAR + 28;
        constexpr float width = WIDTH - MARGIN * 2;
        constexpr float height = HEIGHT - top - MARGIN;
        card({MARGIN, top}, {width, height});
        if (button_at("< Go back", {MARGIN + 10, top + 10}, {84, 26}, ButtonStyle::Link)) {
            page = back_page;
        }
        label("Loader output", {MARGIN, top + 10}, {width, 26}, palette::ink(), fonts.heading, Align::Center);
        ImGui::SetCursorPos({MARGIN + 14, top + 46});
        ImGui::PushStyleColor(ImGuiCol_ChildBg, ImVec4(0, 0, 0, 0.35f));
        ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, {12, 10});
        ImGui::PushStyleVar(ImGuiStyleVar_ChildRounding, 5);
        ImGui::BeginChild("log_text", {width - 28, height - 60}, ImGuiChildFlags_AlwaysUseWindowPadding);
        ImGui::PushFont(widgets::fonts.caption);
        ImGui::PushStyleColor(ImGuiCol_Text, ImGui::ColorConvertU32ToFloat4(palette::soft()));
        // the newest line stays in view unless the reader scrolled up
        ImGui::TextWrapped("%s", log_text.empty() ? "No launch yet." : log_text.c_str());
        if (ImGui::GetScrollY() >= ImGui::GetScrollMaxY() - 4) {
            ImGui::SetScrollHereY(1.0f);
        }
        ImGui::PopStyleColor();
        ImGui::PopFont();
        ImGui::EndChild();
        ImGui::PopStyleVar(2);
        ImGui::PopStyleColor();
    }

    auto Panel::draw(ID3D11RenderTargetView* target) -> void {
        poll();
        ImGui_ImplDX11_NewFrame();
        ImGui_ImplWin32_NewFrame();
        ImGui::NewFrame();
        ImGui::SetNextWindowPos({0, 0});
        ImGui::SetNextWindowSize(ImGui::GetIO().DisplaySize);
        ImGui::SetNextWindowBgAlpha(0);
        ImGui::PushStyleVar(ImGuiStyleVar_WindowRounding, 0);
        ImGui::Begin("Aries",
                     nullptr,
                     ImGuiWindowFlags_NoDecoration | ImGuiWindowFlags_NoMove | ImGuiWindowFlags_NoSavedSettings |
                         ImGuiWindowFlags_NoScrollWithMouse);
        ImGui::PopStyleVar();
        // the blurred desktop stays visible: only a light warm tint (spt AltBackgroundBrush)
        ImGui::GetWindowDrawList()->AddRectFilled({0, 0}, {WIDTH, HEIGHT}, widgets::palette::backdrop());

        if (page == 0) {
            login();
        }
        else if (page == 1) {
            home();
        }
        else if (page == 2) {
            settings();
        }
        else {
            logs();
        }
        title_bar();

        ImGui::End();
        ImGui::Render();
        context->OMSetRenderTargets(1, &target, nullptr);
        ImGui_ImplDX11_RenderDrawData(ImGui::GetDrawData());
    }
} // namespace launcher
