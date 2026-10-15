// kiero-style IDXGISwapChain::Present hook.
//
// Why this instead of UGameViewportClient vtable: previous attempts to hook UE's
// render path (shadow-VMT on PostRender helpers) crashed because UE 5.3's CFG
// obfuscates vtable offsets and nested text-helper hooks reenter ProcessEvent
// mid-draw. The DXGI SwapChain's Present method is one layer below UE — a
// Microsoft COM interface with a FIXED vtable layout (Present is slot 8), fires
// exactly once per frame at a safe point (right before scanout), and is the
// standard approach every published UE5 internal menu uses.

#pragma once
#include <d3d11.h>

namespace present_hook {
    // Called every frame BEFORE the real Present. Draw ImGui here.
    using RenderCallback = void (*)(IDXGISwapChain* swapchain);

    // Install the Present hook. Returns true on success. The callback fires every frame.
    bool install(RenderCallback cb);

    // Called when the swapchain resizes (window resize, resolution change).
    // ImGui's DX11 backend needs to recreate its RT view when this fires.
    using ResizeCallback = void (*)();
    void set_resize_callback(ResizeCallback cb);

    void uninstall();

    // The game's main window handle (captured during install).
    HWND get_game_window();
} // namespace present_hook
