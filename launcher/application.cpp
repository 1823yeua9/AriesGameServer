#include "application.h"
#include "layout.h"
#include "panel.h"
#include "../internal/overlay/vendor/imgui/imgui.h"
#include "../internal/overlay/vendor/imgui/backends/imgui_impl_win32.h"

#include <dwmapi.h>

#include <algorithm>
#include <stdexcept>
#include <windowsx.h>

extern IMGUI_IMPL_API LRESULT ImGui_ImplWin32_WndProcHandler(HWND, UINT, WPARAM, LPARAM);

namespace launcher {
    auto executable_directory() -> std::filesystem::path {
        std::wstring buffer(32768, L'\0');
        const auto length = GetModuleFileNameW(nullptr, buffer.data(), static_cast<DWORD>(buffer.size()));

        if (!length || length >= buffer.size()) {
            throw std::runtime_error("Cannot resolve launcher directory.");
        }

        buffer.resize(length);

        return std::filesystem::path(buffer).parent_path();
    }

    Application::Application(bool smoke_test_value, bool preview_value)
        : capture(MonitorFromPoint({0, 0}, MONITOR_DEFAULTTOPRIMARY)), renderer(capture.graphics_device(),
                                                                                capture.graphics_context(),
                                                                                smoke_test_value ? 400 : 760,
                                                                                smoke_test_value ? 400 : 450),
          smoke_test(smoke_test_value), preview(preview_value), width(smoke_test_value ? 400 : 760),
          height(smoke_test_value ? 400 : 450), artifact_directory(executable_directory() / "blur-artifacts") {
        std::filesystem::create_directories(artifact_directory);
        log.open(artifact_directory / (smoke_test ? "smoke-test.txt" : "launcher.log"));
        log << std::unitbuf;

        WNDCLASSEXW window_class{sizeof(WNDCLASSEXW)};
        window_class.lpfnWndProc = window_proc;
        window_class.hInstance = GetModuleHandleW(nullptr);
        window_class.hCursor = LoadCursorW(nullptr, IDC_ARROW);
        window_class.lpszClassName = L"AriesKdeBlurSquare";

        if (!RegisterClassExW(&window_class)) {
            throw std::runtime_error("Cannot register blur window.");
        }

        const auto bounds = capture.bounds();
        const auto x = bounds.left + (bounds.right - bounds.left - width) / 2;
        const auto y = bounds.top + (bounds.bottom - bounds.top - height) / 2;

        if (smoke_test) {
            window_class.lpfnWndProc = backdrop_proc;
            window_class.lpszClassName = L"AriesBlurTestBackdrop";
            RegisterClassExW(&window_class);
            test_backdrop = CreateWindowExW(0,
                                            window_class.lpszClassName,
                                            L"Blur verification backdrop",
                                            WS_POPUP,
                                            x - 64,
                                            y - 64,
                                            528,
                                            528,
                                            nullptr,
                                            nullptr,
                                            window_class.hInstance,
                                            nullptr);
            ShowWindow(test_backdrop, SW_SHOWNOACTIVATE);
            UpdateWindow(test_backdrop);
        }

        window = CreateWindowExW(WS_EX_APPWINDOW,
                                 L"AriesKdeBlurSquare",
                                 L"Aries Launcher",
                                 WS_POPUP,
                                 x,
                                 y,
                                 width,
                                 height,
                                 nullptr,
                                 nullptr,
                                 GetModuleHandleW(nullptr),
                                 this);

        if (!window) {
            throw std::runtime_error("Cannot create blur window.");
        }

        RECT client_bounds{};
        GetClientRect(window, &client_bounds);

        if (client_bounds.right != static_cast<LONG>(width) || client_bounds.bottom != static_cast<LONG>(height)) {
            throw std::runtime_error("Launcher client size differs from the render surface.");
        }

        // Do not add a native backdrop: the KDE shader supplies every pixel.
        const DWMNCRENDERINGPOLICY rendering_policy = DWMNCRP_DISABLED;
        DwmSetWindowAttribute(window, DWMWA_NCRENDERING_POLICY, &rendering_policy, sizeof(rendering_policy));

        ComPtr<IDXGIDevice> dxgi_device;
        ComPtr<IDXGIAdapter> adapter;
        ComPtr<IDXGIFactory2> factory;
        check(capture.graphics_device()->QueryInterface(IID_PPV_ARGS(&dxgi_device)), "DXGI device");
        check(dxgi_device->GetAdapter(&adapter), "graphics adapter");
        check(adapter->GetParent(IID_PPV_ARGS(&factory)), "swap-chain factory");

        DXGI_SWAP_CHAIN_DESC1 description{};
        description.Width = width;
        description.Height = height;
        description.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
        description.SampleDesc.Count = 1;
        description.BufferUsage = DXGI_USAGE_RENDER_TARGET_OUTPUT;
        description.BufferCount = 2;
        description.SwapEffect = DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL;
        description.AlphaMode = DXGI_ALPHA_MODE_IGNORE;
        check(factory->CreateSwapChainForHwnd(
                  capture.graphics_device(), window, &description, nullptr, nullptr, &swap_chain),
              "create swap chain");
        check(factory->MakeWindowAssociation(window, DXGI_MWA_NO_ALT_ENTER), "disable fullscreen switching");

        ComPtr<ID3D11Texture2D> back_buffer;
        check(swap_chain->GetBuffer(0, IID_PPV_ARGS(&back_buffer)), "swap-chain buffer");
        check(capture.graphics_device()->CreateRenderTargetView(back_buffer.Get(), nullptr, &target),
              "swap-chain target");

        const float clear_color[] = {0.08f, 0.08f, 0.08f, 1.0f};
        capture.graphics_context()->ClearRenderTargetView(target.Get(), clear_color);
        check(swap_chain->Present(0, 0), "initial present");

        log << width << "x" << height << " physical pixels; KDE strength=14, levels=4, offset=7.166667, noise=0\n"
            << "Backdrop: DXGI desktop duplication, own window excluded; no Acrylic/DWM injection\n";

        if (!smoke_test) {
            panel = std::make_unique<Panel>(window, capture.graphics_device(), capture.graphics_context());
        }

        ShowWindow(window, SW_SHOW);
        started = GetTickCount64();

        if (!SetTimer(window, 1, 16, nullptr)) {
            throw std::runtime_error("Cannot start rendering timer.");
        }
    }

    Application::~Application() {
        panel.reset();

        if (IsWindow(window)) {
            DestroyWindow(window);
        }

        if (IsWindow(test_backdrop)) {
            DestroyWindow(test_backdrop);
        }
    }

    auto Application::render_frame() -> void {
        if (quitting || IsIconic(window)) {
            return;
        }

        if (smoke_test && test_stage == 0 && GetTickCount64() - started > 800) {
            SetWindowLongPtrW(test_backdrop, GWLP_USERDATA, 1);
            InvalidateRect(test_backdrop, nullptr, FALSE);
            UpdateWindow(test_backdrop);
            test_stage = 1;
        }

        if (smoke_test && test_stage == 1 && GetTickCount64() - started > 1500) {
            RECT bounds{};
            GetWindowRect(window, &bounds);
            SetWindowPos(
                window, nullptr, bounds.left + 24, bounds.top + 16, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            test_stage = 2;
        }

        if (smoke_test && GetTickCount64() - started > 2500) {
            finish_smoke_test();
            quitting = true;
            PostMessageW(window, WM_CLOSE, 0, 0);

            return;
        }

        const auto updated = capture.update();
        RECT bounds{};
        GetWindowRect(window, &bounds);
        const auto moved = !EqualRect(&bounds, &last_bounds);

        if (!capture.ready() || (!updated && !moved && rendered && !panel)) {
            return;
        }

        const auto desktop_bounds = capture.bounds();
        const auto& desktop = capture.image();
        if (updated || moved || !rendered) {
            renderer.crop(desktop.view.Get(),
                          desktop.width,
                          desktop.height,
                          bounds.left - desktop_bounds.left,
                          bounds.top - desktop_bounds.top,
                          capture.rotation());
            renderer.render(BlurSettings{}, std::max(1.0f, GetDpiForWindow(window) / 96.0f));
        }

        renderer.present(target.Get());

        if (panel) {
            panel->draw(target.Get());
        }

        if (preview && GetTickCount64() - started > 2000) {
            Surface frame;
            frame.width = width;
            frame.height = height;
            check(swap_chain->GetBuffer(0, IID_PPV_ARGS(&frame.texture)), "preview buffer");
            save_bitmap(artifact_directory / "launcher-preview.bmp",
                        width,
                        height,
                        read_surface(capture.graphics_device(), capture.graphics_context(), frame));
            PostMessageW(window, WM_CLOSE, 0, 0);
            preview = false;
        }

        check(swap_chain->Present(1, 0), "present blur");
        last_bounds = bounds;
        rendered = true;
        ++frame_count;

        if (frame_count == 1) {
            log << "First live backdrop frame rendered.\n";
        }
    }

    auto Application::finish_smoke_test() -> void {
        if (!rendered) {
            throw std::runtime_error("No desktop frame arrived during the live test.");
        }

        const auto pixels = read_surface(capture.graphics_device(), capture.graphics_context(), renderer.input());
        const auto output_pixels =
            read_surface(capture.graphics_device(), capture.graphics_context(), renderer.output());
        const auto sample = [&](UINT x, UINT y) {
            return pixels[static_cast<size_t>(y) * square_size + x];
        };

        // The backdrop changes from red/blue to blue/red, then the square moves.
        // Compare every source pixel to catch stale captures, offset/orientation
        // errors, black redaction, and recursive blur, not just a plausible screenshot.
        const auto left = sample(100, 200);
        const auto right = sample(300, 200);
        UINT mismatched_pixels = 0;

        for (UINT y = 0; y < square_size; ++y) {
            for (UINT x = 0; x < square_size; ++x) {
                const auto pixel = sample(x, y);
                const auto expected_red = x < 176 ? 16 : 240;
                const auto expected_blue = x < 176 ? 240 : 16;

                if (std::abs(static_cast<int>(pixel.red) - expected_red) > 1 ||
                    std::abs(static_cast<int>(pixel.blue) - expected_blue) > 1 ||
                    std::abs(static_cast<int>(pixel.green) - 16) > 1) {
                    ++mismatched_pixels;
                }
            }
        }

        log << "Live frames: " << frame_count << "\n"
            << "Backdrop left rgb=" << static_cast<int>(left.red) << ',' << static_cast<int>(left.green) << ','
            << static_cast<int>(left.blue) << "\n"
            << "Backdrop right rgb=" << static_cast<int>(right.red) << ',' << static_cast<int>(right.green) << ','
            << static_cast<int>(right.blue) << "\n";

        save_bitmap(artifact_directory / "live-source.bmp", square_size, square_size, pixels);
        save_bitmap(artifact_directory / "live-blur.bmp", square_size, square_size, output_pixels);

        log << "Mismatched source pixels after background change + movement: " << mismatched_pixels << "/160000\n";

        if (test_stage != 2 || mismatched_pixels != 0) {
            throw std::runtime_error(
                "Live backdrop test failed: own-window exclusion is not producing the expected underlying pixels.");
        }

        log << "PASS: 400x400 live backdrop, background updates, movement alignment, own-window exclusion.\n";
    }

    auto Application::run() -> int {
        MSG message{};
        BOOL result = 0;

        while ((result = GetMessageW(&message, nullptr, 0, 0)) > 0) {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        return failed || result == -1 ? 1 : 0;
    }

    auto CALLBACK Application::window_proc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) -> LRESULT {
        auto* application = reinterpret_cast<Application*>(GetWindowLongPtrW(window, GWLP_USERDATA));

        if (message == WM_NCCREATE) {
            application = static_cast<Application*>(reinterpret_cast<CREATESTRUCTW*>(lparam)->lpCreateParams);
            SetWindowLongPtrW(window, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(application));
        }

        if (!application) {
            return DefWindowProcW(window, message, wparam, lparam);
        }

        try {
            if (application->panel && ImGui_ImplWin32_WndProcHandler(window, message, wparam, lparam)) {
                return 1;
            }

            switch (message) {
            case WM_TIMER:
                application->render_frame();

                return 0;

            case WM_NCHITTEST:
                if (!application->panel) {
                    return HTCAPTION;
                }
                {
                    POINT point{GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam)};
                    ScreenToClient(window, &point);
                    return layout::is_drag_region(point.x, point.y) ? HTCAPTION : HTCLIENT;
                }

            case WM_MOVING: {
                // Keep the prototype on its capture adapter/output.
                auto* rectangle = reinterpret_cast<RECT*>(lparam);
                const auto bounds = application->capture.bounds();
                rectangle->left =
                    std::clamp(rectangle->left, bounds.left, bounds.right - static_cast<LONG>(application->width));
                rectangle->top =
                    std::clamp(rectangle->top, bounds.top, bounds.bottom - static_cast<LONG>(application->height));
                rectangle->right = rectangle->left + application->width;
                rectangle->bottom = rectangle->top + application->height;

                return TRUE;
            }

            case WM_KEYDOWN:
                if (wparam == VK_ESCAPE) {
                    PostMessageW(window, WM_CLOSE, 0, 0);
                }

                return 0;

            case WM_ERASEBKGND:
                return 1;

            case WM_PAINT: {
                PAINTSTRUCT paint{};
                BeginPaint(window, &paint);
                EndPaint(window, &paint);

                return 0;
            }

            case WM_CLOSE:
                if (application->panel && application->panel->running() &&
                    MessageBoxW(window,
                                L"Close this launcher and its game/server session?",
                                L"Aries",
                                MB_YESNO | MB_ICONQUESTION) != IDYES) {
                    return 0;
                }

                application->panel.reset();
                DestroyWindow(window);

                return 0;

            case WM_DESTROY:
                KillTimer(window, 1);
                PostQuitMessage(0);

                return 0;
            }
        }
        catch (const std::exception& error) {
            application->log << "FAIL: " << error.what() << '\n';
            application->failed = true;
            application->quitting = true;
            PostMessageW(window, WM_CLOSE, 0, 0);
        }

        return DefWindowProcW(window, message, wparam, lparam);
    }

    auto CALLBACK Application::backdrop_proc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) -> LRESULT {
        if (message == WM_PAINT) {
            PAINTSTRUCT paint{};
            const auto dc = BeginPaint(window, &paint);
            RECT left{0, 0, 264, 528};
            RECT right{264, 0, 528, 528};
            const auto red = CreateSolidBrush(RGB(240, 16, 16));
            const auto blue = CreateSolidBrush(RGB(16, 16, 240));
            const auto swapped = GetWindowLongPtrW(window, GWLP_USERDATA) != 0;
            FillRect(dc, &left, swapped ? blue : red);
            FillRect(dc, &right, swapped ? red : blue);
            DeleteObject(red);
            DeleteObject(blue);
            EndPaint(window, &paint);

            return 0;
        }

        return DefWindowProcW(window, message, wparam, lparam);
    }
} // namespace launcher
