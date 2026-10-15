// ImGui dev menu rendered from the DXGI Present hook. Backend requests run
// on a separate worker thread; no HTTP work occurs in Present.

#include "overlay.hpp"
#include "reflection_viewer.hpp"
#include "skin_client.hpp"
#include "../entity_viewer.h"
#include "../bot_actions.h"
#include <d3d11.h>
#include <algorithm>
#include <atomic>
#include <cctype>
#include <cstdio>
#include <string>
#include <stdexcept>
#include <vector>
#include "vendor/imgui/imgui.h"
#include "vendor/imgui/backends/imgui_impl_dx11.h"
#include "vendor/imgui/backends/imgui_impl_win32.h"

// SDK definition for the generated reflection adapter's image-base helper.
#include "../../../dumper7/output/5.3.2-5590001+++Ares-Core+release-13.06-ShooterGame/CppSDK/SDK/ShooterGame_classes.hpp"
#include "../../../dumper7/valorant_sdk_adapter.hpp"

namespace SDK::InSDKUtils {
    std::uintptr_t GetImageBase() {
        return reinterpret_cast<std::uintptr_t>(GetModuleHandleW(nullptr));
    }
} // namespace SDK::InSDKUtils

// forward decl from imgui_impl_win32.cpp
extern IMGUI_IMPL_API LRESULT ImGui_ImplWin32_WndProcHandler(HWND hWnd, UINT msg, WPARAM wParam, LPARAM lParam);

extern "C" void overlay_log(const char* msg); // from main.cpp

namespace overlay {
    namespace detail {
        std::atomic<bool> g_imgui_ready{false};
        ID3D11Device* g_dev = nullptr;
        ID3D11DeviceContext* g_ctx = nullptr;
        ID3D11RenderTargetView* g_rtv = nullptr;
        HWND g_hwnd = nullptr;
        WNDPROC g_original_wndproc = nullptr;
        bool g_open = true; // menu open by default so you can see it
        bool g_capture_input = true;

        // ---- menu UI ----
        static bool g_show_help = false;

        static std::string lower_ascii(std::string value) {
            std::transform(value.begin(), value.end(), value.begin(), [](unsigned char character) {
                return static_cast<char>(std::tolower(character));
            });
            return value;
        }

        static void draw_skins() {
            static skin_client::SkinState cached_state;
            static double last_snapshot_time = -1.0;
            static char filter[128] = {};
            const double now = ImGui::GetTime();
            if (last_snapshot_time < 0.0 || now - last_snapshot_time >= 0.25) {
                cached_state = skin_client::snapshot();
                last_snapshot_time = now;
            }
            const auto& state = cached_state;
            ImGui::TextUnformatted("Project-local skin catalog and loadout");
            ImGui::SameLine();
            if (ImGui::SmallButton("Refresh skins")) {
                skin_client::request_refresh();
            }
            ImGui::Text("Backend version: %llu | Catalog: %zu | Equipped: %zu",
                        static_cast<unsigned long long>(state.version),
                        state.available.size(),
                        state.equipped.size());
            const std::size_t unowned_count = static_cast<std::size_t>(
                std::count_if(state.available.begin(), state.available.end(), [](const skin_client::SkinRecord& entry) {
                    return !entry.owned;
                }));
            ImGui::SameLine();
            ImGui::BeginDisabled(!state.connected || state.request_pending || unowned_count == 0);
            if (ImGui::SmallButton("Unlock all catalog skins")) {
                skin_client::request_grant_all();
            }
            ImGui::EndDisabled();
            if (!state.status.empty()) {
                ImGui::TextWrapped("%s", state.status.c_str());
            }
            if (!state.last_action_error.empty()) {
                ImGui::TextColored(
                    ImVec4(1.0f, 0.35f, 0.35f, 1.0f), "Last action error: %s", state.last_action_error.c_str());
            }
            ImGui::TextDisabled("Collection reflects local ownership after a fresh client launch.");
            ImGui::InputText("Filter skin or weapon ID", filter, sizeof(filter));

            const std::string query = lower_ascii(filter);
            static std::vector<std::size_t> visible_indices;
            static std::string cached_query;
            static std::uint64_t cached_version = 0;
            static std::size_t cached_catalog_size = 0;
            if (query != cached_query || state.version != cached_version ||
                state.available.size() != cached_catalog_size) {
                visible_indices.clear();
                visible_indices.reserve(state.available.size());
                for (std::size_t index = 0; index < state.available.size(); ++index) {
                    const auto& entry = state.available[index];
                    if (!query.empty() && lower_ascii(entry.name).find(query) == std::string::npos &&
                        lower_ascii(entry.weapon_id).find(query) == std::string::npos &&
                        lower_ascii(entry.skin_id).find(query) == std::string::npos) {
                        continue;
                    }
                    visible_indices.push_back(index);
                }
                cached_query = query;
                cached_version = state.version;
                cached_catalog_size = state.available.size();
            }
            if (state.available.empty()) {
                ImGui::TextDisabled("No verified catalog entries from the local backend.");
                return;
            }
            if (!ImGui::BeginTable("local_skin_catalog",
                                   4,
                                   ImGuiTableFlags_Borders | ImGuiTableFlags_RowBg | ImGuiTableFlags_ScrollY |
                                       ImGuiTableFlags_Resizable,
                                   ImVec2(0, 420))) {
                return;
            }
            ImGui::TableSetupColumn("Skin", ImGuiTableColumnFlags_WidthStretch);
            ImGui::TableSetupColumn("Weapon ID", ImGuiTableColumnFlags_WidthFixed, 110);
            ImGui::TableSetupColumn("State", ImGuiTableColumnFlags_WidthFixed, 75);
            ImGui::TableSetupColumn("Action", ImGuiTableColumnFlags_WidthFixed, 100);
            ImGui::TableHeadersRow();
            ImGuiListClipper clipper;
            clipper.Begin(static_cast<int>(visible_indices.size()));
            while (clipper.Step()) {
                for (int visible_index = clipper.DisplayStart; visible_index < clipper.DisplayEnd; ++visible_index) {
                    const std::size_t entry_index = visible_indices[visible_index];
                    const auto& entry = state.available[entry_index];
                    const bool equipped = std::any_of(
                        state.equipped.begin(), state.equipped.end(), [&](const skin_client::EquippedRecord& record) {
                            return record.weapon_id == entry.weapon_id && record.skin_id == entry.skin_id;
                        });
                    ImGui::PushID(static_cast<int>(entry_index));
                    ImGui::TableNextRow();
                    ImGui::TableSetColumnIndex(0);
                    ImGui::TextUnformatted(entry.name.c_str());
                    if (ImGui::IsItemHovered()) {
                        ImGui::SetTooltip("Skin ID: %s", entry.skin_id.c_str());
                    }
                    ImGui::TableSetColumnIndex(1);
                    ImGui::TextUnformatted(entry.weapon_id.substr(0, 8).c_str());
                    if (ImGui::IsItemHovered()) {
                        ImGui::SetTooltip("Weapon ID: %s", entry.weapon_id.c_str());
                    }
                    ImGui::TableSetColumnIndex(2);
                    ImGui::TextUnformatted(equipped ? "Equipped" : entry.owned ? "Owned" : "Unowned");
                    ImGui::TableSetColumnIndex(3);
                    ImGui::BeginDisabled(!state.connected || state.request_pending || equipped);
                    if (ImGui::SmallButton(entry.owned ? "Equip" : "Grant")) {
                        if (entry.owned) {
                            skin_client::request_equip(entry.weapon_id, entry.skin_id);
                        }
                        else {
                            skin_client::request_grant(entry.skin_id);
                        }
                    }
                    ImGui::EndDisabled();
                    ImGui::PopID();
                }
            }
            ImGui::EndTable();
        }

        static void draw_menu() {
            if (!g_open) {
                return;
            }
            ImGui::SetNextWindowSize(ImVec2(760, 680), ImGuiCond_Once);
            ImGui::SetNextWindowPos(ImVec2(40, 60), ImGuiCond_Once);
            ImGui::Begin("Offline dev menu  [INSERT]", &g_open, ImGuiWindowFlags_NoCollapse);
            ImGui::TextDisabled("Project-local client | build 13.06");
            ImGui::SameLine();
            if (ImGui::SmallButton("Help")) {
                g_show_help = !g_show_help;
            }
            if (g_show_help) {
                ImGui::TextWrapped(
                    "Viewers inspects loaded engine objects. Skins unlocks verified project-local catalog entries and saves equipped choices in the independent backend. A fresh client launch loads new Collection ownership. Other action tabs are disabled until their backend paths are implemented.");
            }
            ImGui::Separator();
            if (ImGui::BeginTabBar("dev_sections")) {
                if (ImGui::BeginTabItem("Session / Maps")) {
                    ImGui::TextUnformatted("Session: project-local client and backend");
                    ImGui::TextUnformatted("Current world: Viewers > Refresh world snapshot");
                    ImGui::TextDisabled("Travel is controlled by the local session flow.");
                    ImGui::BeginDisabled();
                    ImGui::Button("Return to Main Menu");
                    ImGui::SameLine();
                    ImGui::Button("Enter The Range");
                    ImGui::EndDisabled();
                    internal::bot_actions::draw();
                    ImGui::EndTabItem();
                }
                if (ImGui::BeginTabItem("Player")) {
                    ImGui::TextUnformatted("Local controller, pawn, and inventory: Viewers > Refresh world snapshot");
                    ImGui::TextDisabled("Account and inventory changes are not implemented in this overlay.");
                    ImGui::BeginDisabled();
                    ImGui::Button("Unlock all");
                    ImGui::SameLine();
                    ImGui::Button("Infinite credits");
                    ImGui::Button("Set level");
                    ImGui::SameLine();
                    ImGui::Button("Remove level");
                    ImGui::EndDisabled();
                    ImGui::EndTabItem();
                }
                if (ImGui::BeginTabItem("Agents")) {
                    ImGui::TextUnformatted("Agent ownership: supplied by the local backend");
                    ImGui::TextDisabled("Search loaded agent objects in Viewers; ownership editing is unavailable.");
                    ImGui::BeginDisabled();
                    ImGui::Button("Unlock all agents");
                    ImGui::Button("Add agent");
                    ImGui::SameLine();
                    ImGui::Button("Remove agent");
                    ImGui::EndDisabled();
                    ImGui::EndTabItem();
                }
                if (ImGui::BeginTabItem("Skins")) {
                    draw_skins();
                    ImGui::EndTabItem();
                }
                if (ImGui::BeginTabItem("Viewers")) {
                    internal::entity_viewer::draw_options();
                    ImGui::Separator();
                    reflection_viewer::draw();
                    ImGui::EndTabItem();
                }
                ImGui::EndTabBar();
            }
            ImGui::Separator();
            ImGui::Checkbox("Capture input (uncheck to let game receive mouse)", &g_capture_input);
            ImGui::End();
        }

        static bool ensure_rtv(IDXGISwapChain* swapchain) {
            if (g_rtv) {
                return true;
            }
            ID3D11Texture2D* back = nullptr;
            if (FAILED(swapchain->GetBuffer(0, __uuidof(ID3D11Texture2D), (void**)&back)) || !back) {
                return false;
            }
            HRESULT hr = g_dev->CreateRenderTargetView(back, nullptr, &g_rtv);
            back->Release();
            return SUCCEEDED(hr) && g_rtv;
        }
    } // namespace detail

    using namespace detail;

    static LRESULT CALLBACK wndproc_hook(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp) {
        // Toggle on INSERT (edge-triggered).
        static bool ins_prev = false;
        if (msg == WM_KEYDOWN && wp == VK_INSERT && !ins_prev) {
            g_open = !g_open;
            ins_prev = true;
        }
        if (msg == WM_KEYUP && wp == VK_INSERT) {
            ins_prev = false;
        }

        if (g_imgui_ready.load() && g_open) {
            // Keep the menu interactive after capture is unchecked; only swallowing
            // input is optional. Otherwise the checkbox cannot be checked again.
            const bool handled = ImGui_ImplWin32_WndProcHandler(hwnd, msg, wp, lp);
            if (g_capture_input) {
                if (handled) {
                    return 1;
                }
                auto& io = ImGui::GetIO();
                if (io.WantCaptureMouse && (msg >= WM_MOUSEFIRST && msg <= WM_MOUSELAST)) {
                    return 1;
                }
                if (io.WantCaptureKeyboard && (msg == WM_KEYDOWN || msg == WM_KEYUP || msg == WM_CHAR)) {
                    return 1;
                }
            }
        }
        return CallWindowProcA(g_original_wndproc, hwnd, msg, wp, lp);
    }

    void install_wndproc(HWND hwnd) {
        if (!hwnd || g_original_wndproc) {
            return;
        }
        g_hwnd = hwnd;
        g_original_wndproc = (WNDPROC)SetWindowLongPtrA(hwnd, GWLP_WNDPROC, (LONG_PTR)&wndproc_hook);

        if (!g_original_wndproc) {
            throw std::runtime_error("Cannot attach dev-menu window input");
        }

        if (!internal::entity_viewer::attach(hwnd)) {
            overlay_log("Entity sampler unavailable: window-thread hook/timer failed");
        }
    }
    void uninstall_wndproc() {
        if (g_hwnd && g_original_wndproc) {
            SetWindowLongPtrA(g_hwnd, GWLP_WNDPROC, (LONG_PTR)g_original_wndproc);
            g_original_wndproc = nullptr;
            g_hwnd = nullptr;
        }
    }

    void on_resize() {
        if (g_ctx) {
            g_ctx->OMSetRenderTargets(0, nullptr, nullptr);
        }
        if (g_rtv) {
            g_rtv->Release();
            g_rtv = nullptr;
        }
    }

    void shutdown() {
        if (g_imgui_ready.exchange(false)) {
            ImGui_ImplDX11_Shutdown();
            ImGui_ImplWin32_Shutdown();
            ImGui::DestroyContext();
        }
        uninstall_wndproc();
        on_resize();
        if (g_ctx) {
            g_ctx->Release();
            g_ctx = nullptr;
        }
        if (g_dev) {
            g_dev->Release();
            g_dev = nullptr;
        }
    }

    void on_present(IDXGISwapChain* swapchain) {
        if (!g_imgui_ready.load()) {
            // First call: initialize DX11 state and ImGui.
            if (FAILED(swapchain->GetDevice(__uuidof(ID3D11Device), (void**)&g_dev)) || !g_dev) {
                return;
            }
            g_dev->GetImmediateContext(&g_ctx);
            DXGI_SWAP_CHAIN_DESC desc;
            if (FAILED(swapchain->GetDesc(&desc))) {
                return;
            }

            install_wndproc(desc.OutputWindow);

            IMGUI_CHECKVERSION();
            ImGui::CreateContext();
            ImGuiIO& io = ImGui::GetIO();
            io.IniFilename = nullptr; // don't write imgui.ini
            io.ConfigFlags |= ImGuiConfigFlags_NavEnableKeyboard;
            ImGui::StyleColorsDark();
            if (!ImGui_ImplWin32_Init(desc.OutputWindow) || !ImGui_ImplDX11_Init(g_dev, g_ctx)) {
                throw std::runtime_error("Cannot initialize ImGui backends");
            }
            g_imgui_ready = true;
            overlay_log("overlay: ImGui ready");
        }

        if (!ensure_rtv(swapchain)) {
            return;
        }

        ImGui_ImplDX11_NewFrame();
        ImGui_ImplWin32_NewFrame();
        ImGui::NewFrame();

        internal::entity_viewer::draw_world();
        draw_menu();

        ImGui::Render();
        ID3D11RenderTargetView* previous_rtv = nullptr;
        ID3D11DepthStencilView* previous_depth = nullptr;
        g_ctx->OMGetRenderTargets(1, &previous_rtv, &previous_depth);
        g_ctx->OMSetRenderTargets(1, &g_rtv, nullptr);
        ImGui_ImplDX11_RenderDrawData(ImGui::GetDrawData());
        g_ctx->OMSetRenderTargets(1, &previous_rtv, previous_depth);
        if (previous_rtv) {
            previous_rtv->Release();
        }
        if (previous_depth) {
            previous_depth->Release();
        }
    }
} // namespace overlay
