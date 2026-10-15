#pragma once

#include "graphics.h"

namespace launcher {
    // KDE strength 14: one notch below upstream's default, with grain disabled.
    struct BlurSettings {
        UINT iterations = 4;
        float offset = 3.0f + (5.0f / 6.0f) * 5.0f;
        UINT noise_strength = 0;
        float brightness = 1.0f;
        float saturation = 1.0f;
        float contrast = 1.0f;
        float refraction_strength = 0.0f;
        float refraction_edge = 200.0f;
        float refraction_power = 1.0f;
        float refraction_fringing = 0.05f;
    };

    struct alignas(16) ShaderConstants {
        float half_pixel[2]{};
        float offset = 0.0f;
        UINT transform_colors = 0;

        float brightness = 1.0f;
        float saturation = 1.0f;
        float contrast = 1.0f;
        UINT final_pass = 0;

        float crop_origin[2]{};
        float crop_scale[2]{1.0f, 1.0f};

        float refraction_strength = 0.0f;
        float refraction_edge = 200.0f;
        float refraction_power = 1.0f;
        float refraction_fringing = 0.05f;

        float blur_size[2]{400.0f, 400.0f};
        float noise_scale = 1.0f;
        UINT noise_enabled = 0;

        UINT rotation = DXGI_MODE_ROTATION_IDENTITY;
        float rotation_padding[3]{};
    };

    class BlurRenderer {
    public:
        BlurRenderer(ID3D11Device* device,
                     ID3D11DeviceContext* context,
                     UINT width = square_size,
                     UINT height = square_size);

        auto crop(ID3D11ShaderResourceView* source,
                  UINT width,
                  UINT height,
                  LONG x,
                  LONG y,
                  DXGI_MODE_ROTATION rotation = DXGI_MODE_ROTATION_IDENTITY) -> void;

        auto upload(const std::vector<Pixel>& pixels) -> void;

        auto render(const BlurSettings& settings, float noise_scale = 1.0f) -> void;

        auto present(ID3D11RenderTargetView* target) -> void;

        auto input() const -> const Surface&;

        auto output() const -> const Surface&;

    private:
        auto draw(ID3D11ShaderResourceView* source,
                  ID3D11RenderTargetView* target,
                  UINT width,
                  UINT height,
                  ID3D11PixelShader* shader,
                  const ShaderConstants& constants) -> void;

        auto update_noise(UINT strength) -> void;

        ComPtr<ID3D11Device> device;
        ComPtr<ID3D11DeviceContext> context;
        ComPtr<ID3D11VertexShader> vertex_shader;
        ComPtr<ID3D11PixelShader> copy_shader;
        ComPtr<ID3D11PixelShader> downsample_shader;
        ComPtr<ID3D11PixelShader> upsample_shader;
        ComPtr<ID3D11Buffer> constant_buffer;
        ComPtr<ID3D11SamplerState> linear_sampler;
        ComPtr<ID3D11SamplerState> noise_sampler;
        ComPtr<ID3D11ShaderResourceView> noise_view;
        std::array<Surface, 5> levels;
        Surface result;
        UINT current_noise_strength = UINT_MAX;
    };
} // namespace launcher
