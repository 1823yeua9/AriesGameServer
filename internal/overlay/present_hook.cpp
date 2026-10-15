// Implementation of the Present hook using MinHook + a dummy swapchain trick.
//
// Technique (standard kiero pattern):
//   1. Create a minimal hidden window
//   2. Create a throwaway D3D11 device + swapchain targeting that window
//   3. Read the swapchain's vtable — slot 8 is Present, slot 13 is ResizeBuffers
//   4. Use the default DXGI implementation's function pointers. Wrapped or
//      alternate swapchain implementations are not assumed to share them.
//   5. Hook those pointers with MinHook. When the game's real swapchain calls
//      Present, our hook fires with (this = real swapchain).

#include "present_hook.hpp"
#include <windows.h>
#include <d3d11.h>
#include <dxgi.h>
#include <atomic>
#include <cstdio>
#include "vendor/MinHook/include/MinHook.h"

#pragma comment(lib, "d3d11.lib")
#pragma comment(lib, "dxgi.lib")

namespace present_hook {
    namespace detail {
        using PresentFn = HRESULT(STDMETHODCALLTYPE*)(IDXGISwapChain*, UINT, UINT);
        using ResizeBuffersFn = HRESULT(STDMETHODCALLTYPE*)(IDXGISwapChain*, UINT, UINT, UINT, DXGI_FORMAT, UINT);

        PresentFn g_original_present = nullptr;
        ResizeBuffersFn g_original_resize = nullptr;
        RenderCallback g_render_cb = nullptr;
        ResizeCallback g_resize_cb = nullptr;
        HWND g_game_window = nullptr;
        std::atomic<bool> g_installed{false};
        std::atomic<bool> g_render_faulted{false};
        void* g_present_addr = nullptr;
        void* g_resize_addr = nullptr;

        extern "C" void overlay_log(const char* msg); // provided by main.cpp

        HRESULT STDMETHODCALLTYPE hooked_present(IDXGISwapChain* swapchain, UINT sync, UINT flags) {
            DXGI_SWAP_CHAIN_DESC desc{};

            if (FAILED(swapchain->GetDesc(&desc)) || !desc.OutputWindow || (flags & DXGI_PRESENT_TEST)) {
                return g_original_present(swapchain, sync, flags);
            }

            // Capture the game's window on the first real Present call.
            if (!g_game_window) {
                g_game_window = desc.OutputWindow;
            }
            __try {
                if (g_render_cb && !g_render_faulted && desc.OutputWindow == g_game_window) {
                    g_render_cb(swapchain);
                }
            }
            __except (EXCEPTION_EXECUTE_HANDLER) {
                if (!g_render_faulted.exchange(true)) {
                    overlay_log("FAILED: overlay disabled after a render exception");
                }
            }
            return g_original_present(swapchain, sync, flags);
        }

        HRESULT STDMETHODCALLTYPE
        hooked_resize(IDXGISwapChain* swapchain, UINT bufcount, UINT w, UINT h, DXGI_FORMAT fmt, UINT flags) {
            DXGI_SWAP_CHAIN_DESC desc{};

            if (g_resize_cb && SUCCEEDED(swapchain->GetDesc(&desc)) && desc.OutputWindow == g_game_window) {
                g_resize_cb();
            }
            return g_original_resize(swapchain, bufcount, w, h, fmt, flags);
        }

        // Create a throwaway swapchain to read the vtable.
        bool find_present_pointers(void** out_present, void** out_resize) {
            WNDCLASSEXA wc{sizeof(wc)};
            wc.lpfnWndProc = DefWindowProcA;
            wc.hInstance = GetModuleHandleA(nullptr);
            wc.lpszClassName = "DevOverlayDummy";
            if (!RegisterClassExA(&wc) && GetLastError() != ERROR_CLASS_ALREADY_EXISTS) {
                overlay_log("present_hook: RegisterClassEx failed");
                return false;
            }

            HWND dummy_wnd = CreateWindowExA(0,
                                             "DevOverlayDummy",
                                             "d",
                                             WS_OVERLAPPEDWINDOW,
                                             0,
                                             0,
                                             100,
                                             100,
                                             nullptr,
                                             nullptr,
                                             wc.hInstance,
                                             nullptr);
            if (!dummy_wnd) {
                overlay_log("present_hook: CreateWindow failed");
                return false;
            }

            DXGI_SWAP_CHAIN_DESC sd{};
            sd.BufferCount = 1;
            sd.BufferDesc.Width = 100;
            sd.BufferDesc.Height = 100;
            sd.BufferDesc.Format = DXGI_FORMAT_R8G8B8A8_UNORM;
            sd.BufferUsage = DXGI_USAGE_RENDER_TARGET_OUTPUT;
            sd.OutputWindow = dummy_wnd;
            sd.SampleDesc.Count = 1;
            sd.Windowed = TRUE;
            sd.SwapEffect = DXGI_SWAP_EFFECT_DISCARD;

            D3D_FEATURE_LEVEL fl;
            IDXGISwapChain* sc = nullptr;
            ID3D11Device* dev = nullptr;
            ID3D11DeviceContext* ctx = nullptr;
            D3D_FEATURE_LEVEL want[] = {D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_10_0};
            HRESULT hr = D3D11CreateDeviceAndSwapChain(nullptr,
                                                       D3D_DRIVER_TYPE_HARDWARE,
                                                       nullptr,
                                                       0,
                                                       want,
                                                       ARRAYSIZE(want),
                                                       D3D11_SDK_VERSION,
                                                       &sd,
                                                       &sc,
                                                       &dev,
                                                       &fl,
                                                       &ctx);
            if (FAILED(hr) || !sc) {
                char buf[80];
                wsprintfA(buf, "present_hook: D3D11CreateDeviceAndSwapChain hr=0x%08X", hr);
                overlay_log(buf);
                DestroyWindow(dummy_wnd);
                return false;
            }

            // IDXGISwapChain vtable layout (fixed by Microsoft COM contract):
            //   0: QueryInterface
            //   1: AddRef
            //   2: Release
            //   3: SetPrivateData
            //   4: SetPrivateDataInterface
            //   5: GetPrivateData
            //   6: GetParent
            //   7: GetDevice
            //   8: Present                    <-- what we want
            //   9: GetBuffer
            //  10: SetFullscreenState
            //  11: GetFullscreenState
            //  12: GetDesc
            //  13: ResizeBuffers              <-- what we want
            //  14: ResizeTarget
            //  15: GetContainingOutput
            //  16: GetFrameStatistics
            //  17: GetLastPresentCount
            void** vtbl = *reinterpret_cast<void***>(sc);
            *out_present = vtbl[8];
            *out_resize = vtbl[13];

            sc->Release();
            if (ctx) {
                ctx->Release();
            }
            if (dev) {
                dev->Release();
            }
            DestroyWindow(dummy_wnd);
            return true;
        }
    } // namespace detail

    using namespace detail;

    bool install(RenderCallback cb) {
        if (g_installed.load()) {
            overlay_log("present_hook: already installed");
            return false;
        }
        g_render_cb = cb;

        void* present_addr = nullptr;
        void* resize_addr = nullptr;
        if (!find_present_pointers(&present_addr, &resize_addr)) {
            return false;
        }

        char buf[128];
        wsprintfA(buf, "present_hook: Present=%p ResizeBuffers=%p", present_addr, resize_addr);
        overlay_log(buf);

        MH_STATUS st = MH_Initialize();
        if (st != MH_OK && st != MH_ERROR_ALREADY_INITIALIZED) {
            wsprintfA(buf, "present_hook: MH_Initialize failed (%d)", st);
            overlay_log(buf);
            return false;
        }

        st = MH_CreateHook(present_addr, (LPVOID)&hooked_present, (LPVOID*)&g_original_present);
        if (st != MH_OK) {
            wsprintfA(buf, "present_hook: CreateHook(Present) failed (%d)", st);
            overlay_log(buf);
            return false;
        }

        st = MH_CreateHook(resize_addr, (LPVOID)&hooked_resize, (LPVOID*)&g_original_resize);
        if (st != MH_OK) {
            MH_RemoveHook(present_addr);
            wsprintfA(buf, "present_hook: CreateHook(ResizeBuffers) failed (%d)", st);
            overlay_log(buf);
            return false;
        }

        st = MH_EnableHook(present_addr);
        if (st == MH_OK) {
            st = MH_EnableHook(resize_addr);
        }
        if (st != MH_OK) {
            MH_DisableHook(present_addr);
            MH_RemoveHook(present_addr);
            MH_RemoveHook(resize_addr);
            wsprintfA(buf, "present_hook: EnableHook failed (%d)", st);
            overlay_log(buf);
            return false;
        }

        g_present_addr = present_addr;
        g_resize_addr = resize_addr;
        g_installed = true;
        overlay_log("present_hook: install OK");
        return true;
    }

    void set_resize_callback(ResizeCallback cb) {
        g_resize_cb = cb;
    }

    void uninstall() {
        if (!g_installed.load()) {
            return;
        }
        MH_DisableHook(g_present_addr);
        MH_DisableHook(g_resize_addr);
        MH_RemoveHook(g_present_addr);
        MH_RemoveHook(g_resize_addr);
        g_present_addr = nullptr;
        g_resize_addr = nullptr;
        g_installed = false;
    }

    HWND get_game_window() {
        return g_game_window;
    }
} // namespace present_hook
