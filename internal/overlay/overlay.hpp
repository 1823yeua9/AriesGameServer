#pragma once
#include <d3d11.h>

namespace overlay {
    // Called every frame from inside the Present hook.
    void on_present(IDXGISwapChain* swapchain);

    // Called when the swapchain resizes.
    void on_resize();

    // Install the Win32 WndProc hook so ImGui can see mouse/keyboard input.
    void install_wndproc(HWND hwnd);
    void uninstall_wndproc();

    // Release ImGui/DX11 resources after the Present hook has been removed.
    void shutdown();
} // namespace overlay
