#pragma once

#include "blur.h"
#include "desktop_capture.h"

#include <fstream>
#include <memory>

namespace launcher {
    class Panel;

    class Application {
    public:
        explicit Application(bool smoke_test, bool preview = false);

        ~Application();

        auto run() -> int;

    private:
        static auto CALLBACK window_proc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) -> LRESULT;

        static auto CALLBACK backdrop_proc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) -> LRESULT;

        auto render_frame() -> void;

        auto finish_smoke_test() -> void;

        DesktopCapture capture;
        BlurRenderer renderer;
        ComPtr<IDXGISwapChain1> swap_chain;
        ComPtr<ID3D11RenderTargetView> target;
        HWND window = nullptr;
        HWND test_backdrop = nullptr;
        RECT last_bounds{};
        bool smoke_test = false;
        bool preview = false;
        UINT width = 760;
        UINT height = 450;
        std::unique_ptr<Panel> panel;
        bool quitting = false;
        bool failed = false;
        bool rendered = false;
        ULONGLONG started = 0;
        UINT frame_count = 0;
        UINT test_stage = 0;
        std::ofstream log;
        std::filesystem::path artifact_directory;
    };

    auto executable_directory() -> std::filesystem::path;

    auto run_blur_tests(const std::filesystem::path& directory) -> int;
} // namespace launcher
