#include "desktop_capture.h"

#include <stdexcept>

namespace launcher {
    DesktopCapture::DesktopCapture(HMONITOR monitor) {
        ComPtr<IDXGIFactory1> factory;
        check(CreateDXGIFactory1(IID_PPV_ARGS(&factory)), "create DXGI factory");

        ComPtr<IDXGIAdapter1> selected_adapter;

        for (UINT adapter_index = 0; !output; ++adapter_index) {
            ComPtr<IDXGIAdapter1> adapter;
            const auto adapter_result = factory->EnumAdapters1(adapter_index, &adapter);

            if (adapter_result == DXGI_ERROR_NOT_FOUND) {
                break;
            }

            check(adapter_result, "enumerate adapters");

            for (UINT output_index = 0;; ++output_index) {
                ComPtr<IDXGIOutput> candidate;
                const auto output_result = adapter->EnumOutputs(output_index, &candidate);

                if (output_result == DXGI_ERROR_NOT_FOUND) {
                    break;
                }

                check(output_result, "enumerate outputs");

                DXGI_OUTPUT_DESC description{};
                check(candidate->GetDesc(&description), "output description");

                if (description.Monitor == monitor && description.AttachedToDesktop) {
                    check(candidate.As(&output), "desktop capture output");
                    selected_adapter = adapter;
                    desktop_bounds = description.DesktopCoordinates;
                    desktop_rotation = description.Rotation;

                    break;
                }
            }
        }

        if (!selected_adapter) {
            throw std::runtime_error("No desktop output found for this monitor.");
        }

        const D3D_FEATURE_LEVEL feature_levels[] = {D3D_FEATURE_LEVEL_11_0};
        check(D3D11CreateDevice(selected_adapter.Get(),
                                D3D_DRIVER_TYPE_UNKNOWN,
                                nullptr,
                                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                                feature_levels,
                                1,
                                D3D11_SDK_VERSION,
                                &device,
                                nullptr,
                                &context),
              "create capture graphics device");
    }

    auto DesktopCapture::update() -> bool {
        if (!duplication) {
            const auto result = output->DuplicateOutput(device.Get(), &duplication);

            if (result == DXGI_ERROR_NOT_CURRENTLY_AVAILABLE || result == E_ACCESSDENIED ||
                result == DXGI_ERROR_UNSUPPORTED) {
                has_frame = false;

                return false;
            }

            check(result, "create desktop duplication");
        }

        DXGI_OUTDUPL_FRAME_INFO frame_information{};
        ComPtr<IDXGIResource> resource;
        const auto result = duplication->AcquireNextFrame(0, &frame_information, &resource);

        if (result == DXGI_ERROR_WAIT_TIMEOUT) {
            return false;
        }

        if (result == DXGI_ERROR_ACCESS_LOST) {
            duplication.Reset();
            has_frame = false;

            return false;
        }

        check(result, "acquire desktop frame");

        try {
            ComPtr<ID3D11Texture2D> texture;
            check(resource.As(&texture), "desktop frame texture");

            D3D11_TEXTURE2D_DESC description{};
            texture->GetDesc(&description);

            if (!desktop.texture || desktop.width != description.Width || desktop.height != description.Height) {
                desktop = make_surface(device.Get(), description.Width, description.Height);
            }

            context->CopyResource(desktop.texture.Get(), texture.Get());
            has_frame = true;
        }
        catch (...) {
            duplication->ReleaseFrame();

            throw;
        }

        check(duplication->ReleaseFrame(), "release desktop frame");

        return true;
    }

    auto DesktopCapture::graphics_device() const -> ID3D11Device* {
        return device.Get();
    }

    auto DesktopCapture::graphics_context() const -> ID3D11DeviceContext* {
        return context.Get();
    }

    auto DesktopCapture::image() const -> const Surface& {
        return desktop;
    }

    auto DesktopCapture::bounds() const -> RECT {
        return desktop_bounds;
    }

    auto DesktopCapture::ready() const -> bool {
        return has_frame;
    }

    auto DesktopCapture::rotation() const -> DXGI_MODE_ROTATION {
        return desktop_rotation;
    }
} // namespace launcher
