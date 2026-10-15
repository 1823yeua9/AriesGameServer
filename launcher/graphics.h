#pragma once

#define NOMINMAX
#define WIN32_LEAN_AND_MEAN
#include <Windows.h>
#include <d3d11.h>
#include <dxgi1_2.h>
#include <wrl/client.h>

#include <array>
#include <cstdint>
#include <filesystem>
#include <string_view>
#include <vector>

namespace launcher {
    template <typename T> using ComPtr = Microsoft::WRL::ComPtr<T>;

    inline constexpr UINT square_size = 400;

    auto check(HRESULT result, std::string_view operation) -> void;

    struct Pixel {
        uint8_t blue;
        uint8_t green;
        uint8_t red;
        uint8_t alpha;
    };

    struct Surface {
        UINT width = 0;
        UINT height = 0;
        ComPtr<ID3D11Texture2D> texture;
        ComPtr<ID3D11ShaderResourceView> view;
        ComPtr<ID3D11RenderTargetView> target;
    };

    auto make_surface(ID3D11Device* device, UINT width, UINT height, const Pixel* pixels = nullptr) -> Surface;

    auto read_surface(ID3D11Device* device, ID3D11DeviceContext* context, const Surface& surface) -> std::vector<Pixel>;

    auto save_bitmap(const std::filesystem::path& path, UINT width, UINT height, const std::vector<Pixel>& pixels)
        -> void;
} // namespace launcher
