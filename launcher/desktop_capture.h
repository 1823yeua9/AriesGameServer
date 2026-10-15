#pragma once

#include "graphics.h"

namespace launcher {
    class DesktopCapture {
    public:
        explicit DesktopCapture(HMONITOR monitor);

        auto update() -> bool;

        auto graphics_device() const -> ID3D11Device*;

        auto graphics_context() const -> ID3D11DeviceContext*;

        auto image() const -> const Surface&;

        auto bounds() const -> RECT;

        auto ready() const -> bool;

        auto rotation() const -> DXGI_MODE_ROTATION;

    private:
        ComPtr<ID3D11Device> device;
        ComPtr<ID3D11DeviceContext> context;
        ComPtr<IDXGIOutput1> output;
        ComPtr<IDXGIOutputDuplication> duplication;
        Surface desktop;
        RECT desktop_bounds{};
        bool has_frame = false;
        DXGI_MODE_ROTATION desktop_rotation = DXGI_MODE_ROTATION_IDENTITY;
    };
} // namespace launcher
