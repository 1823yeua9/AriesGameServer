#include "graphics.h"

#include <fstream>
#include <iomanip>
#include <sstream>
#include <stdexcept>

namespace launcher {
    auto check(HRESULT result, std::string_view operation) -> void {
        if (FAILED(result)) {
            std::ostringstream message;
            message << operation << " failed (0x" << std::hex << static_cast<unsigned long>(result) << ')';

            throw std::runtime_error(message.str());
        }
    }

    auto make_surface(ID3D11Device* device, UINT width, UINT height, const Pixel* pixels) -> Surface {
        Surface surface;
        surface.width = width;
        surface.height = height;

        D3D11_TEXTURE2D_DESC description{};
        description.Width = width;
        description.Height = height;
        description.MipLevels = 1;
        description.ArraySize = 1;
        description.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
        description.SampleDesc.Count = 1;
        description.BindFlags = D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET;

        D3D11_SUBRESOURCE_DATA initial{};
        initial.pSysMem = pixels;
        initial.SysMemPitch = width * sizeof(Pixel);

        check(device->CreateTexture2D(&description, pixels ? &initial : nullptr, &surface.texture), "create texture");
        check(device->CreateShaderResourceView(surface.texture.Get(), nullptr, &surface.view), "create texture view");
        check(device->CreateRenderTargetView(surface.texture.Get(), nullptr, &surface.target), "create render target");

        return surface;
    }

    auto read_surface(ID3D11Device* device, ID3D11DeviceContext* context, const Surface& surface)
        -> std::vector<Pixel> {
        D3D11_TEXTURE2D_DESC description{};
        surface.texture->GetDesc(&description);
        description.BindFlags = 0;
        description.Usage = D3D11_USAGE_STAGING;
        description.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
        description.MiscFlags = 0;

        ComPtr<ID3D11Texture2D> staging;
        check(device->CreateTexture2D(&description, nullptr, &staging), "create readback");
        context->CopyResource(staging.Get(), surface.texture.Get());

        D3D11_MAPPED_SUBRESOURCE mapping{};
        check(context->Map(staging.Get(), 0, D3D11_MAP_READ, 0, &mapping), "map readback");

        std::vector<Pixel> pixels(static_cast<size_t>(surface.width) * surface.height);

        for (UINT row = 0; row < surface.height; ++row) {
            memcpy(pixels.data() + static_cast<size_t>(row) * surface.width,
                   static_cast<const uint8_t*>(mapping.pData) + static_cast<size_t>(row) * mapping.RowPitch,
                   static_cast<size_t>(surface.width) * sizeof(Pixel));
        }

        context->Unmap(staging.Get(), 0);

        return pixels;
    }

    auto save_bitmap(const std::filesystem::path& path, UINT width, UINT height, const std::vector<Pixel>& pixels)
        -> void {
        BITMAPFILEHEADER file_header{};
        file_header.bfType = 0x4d42;
        file_header.bfOffBits = sizeof(BITMAPFILEHEADER) + sizeof(BITMAPINFOHEADER);
        file_header.bfSize = file_header.bfOffBits + width * height * sizeof(Pixel);

        BITMAPINFOHEADER image_header{};
        image_header.biSize = sizeof(image_header);
        image_header.biWidth = static_cast<LONG>(width);
        image_header.biHeight = -static_cast<LONG>(height);
        image_header.biPlanes = 1;
        image_header.biBitCount = 32;
        image_header.biCompression = BI_RGB;

        std::ofstream stream(path, std::ios::binary);
        stream.exceptions(std::ios::failbit | std::ios::badbit);
        stream.write(reinterpret_cast<const char*>(&file_header), sizeof(file_header));
        stream.write(reinterpret_cast<const char*>(&image_header), sizeof(image_header));
        stream.write(reinterpret_cast<const char*>(pixels.data()),
                     static_cast<std::streamsize>(pixels.size() * sizeof(Pixel)));
    }
} // namespace launcher
