// KDE blur shader port. See reference/NOTICE.txt and reference/COPYING.
#include "blur.h"

#include <d3dcompiler.h>

#include <algorithm>
#include <random>
#include <stdexcept>
#include <string>

namespace launcher {
    auto compile_shader(const char* entry, const char* profile) -> ComPtr<ID3DBlob> {
        const auto resource = FindResourceW(nullptr, MAKEINTRESOURCEW(101), RT_RCDATA);

        if (!resource) {
            throw std::runtime_error("embedded blur shader missing");
        }

        const auto data = LockResource(LoadResource(nullptr, resource));
        const auto size = SizeofResource(nullptr, resource);
        ComPtr<ID3DBlob> bytecode;
        ComPtr<ID3DBlob> errors;
        const auto result = D3DCompile(data,
                                       size,
                                       "blur.hlsl",
                                       nullptr,
                                       nullptr,
                                       entry,
                                       profile,
                                       D3DCOMPILE_ENABLE_STRICTNESS | D3DCOMPILE_OPTIMIZATION_LEVEL3,
                                       0,
                                       &bytecode,
                                       &errors);

        if (FAILED(result) && errors) {
            throw std::runtime_error(
                std::string(static_cast<const char*>(errors->GetBufferPointer()), errors->GetBufferSize()));
        }

        check(result, "compile blur shader");

        return bytecode;
    }

    BlurRenderer::BlurRenderer(ID3D11Device* device_value, ID3D11DeviceContext* context_value, UINT width, UINT height)
        : device(device_value), context(context_value) {
        if (width < 16 || height < 16 || width > 8192 || height > 8192) {
            throw std::runtime_error("Invalid blur surface dimensions");
        }
        const auto vertex = compile_shader("vertex_main", "vs_5_0");
        check(device->CreateVertexShader(vertex->GetBufferPointer(), vertex->GetBufferSize(), nullptr, &vertex_shader),
              "vertex shader");

        const auto create_pixel = [&](const char* entry, ComPtr<ID3D11PixelShader>& shader) {
            const auto bytecode = compile_shader(entry, "ps_5_0");
            check(device->CreatePixelShader(bytecode->GetBufferPointer(), bytecode->GetBufferSize(), nullptr, &shader),
                  entry);
        };

        create_pixel("copy_main", copy_shader);
        create_pixel("downsample_main", downsample_shader);
        create_pixel("upsample_main", upsample_shader);

        D3D11_BUFFER_DESC buffer{};
        buffer.ByteWidth = sizeof(ShaderConstants);
        buffer.Usage = D3D11_USAGE_DEFAULT;
        buffer.BindFlags = D3D11_BIND_CONSTANT_BUFFER;
        check(device->CreateBuffer(&buffer, nullptr, &constant_buffer), "constant buffer");

        D3D11_SAMPLER_DESC sampler{};
        sampler.Filter = D3D11_FILTER_MIN_MAG_MIP_LINEAR;
        sampler.AddressU = D3D11_TEXTURE_ADDRESS_CLAMP;
        sampler.AddressV = D3D11_TEXTURE_ADDRESS_CLAMP;
        sampler.AddressW = D3D11_TEXTURE_ADDRESS_CLAMP;
        sampler.MaxLOD = D3D11_FLOAT32_MAX;
        check(device->CreateSamplerState(&sampler, &linear_sampler), "linear clamp sampler");

        sampler.Filter = D3D11_FILTER_MIN_MAG_MIP_POINT;
        sampler.AddressU = D3D11_TEXTURE_ADDRESS_WRAP;
        sampler.AddressV = D3D11_TEXTURE_ADDRESS_WRAP;
        sampler.AddressW = D3D11_TEXTURE_ADDRESS_WRAP;
        check(device->CreateSamplerState(&sampler, &noise_sampler), "noise sampler");

        for (UINT index = 0; index < levels.size(); ++index) {
            levels[index] = make_surface(device.Get(), width >> index, height >> index);
        }

        result = make_surface(device.Get(), width, height);
        update_noise(5);
    }

    auto BlurRenderer::update_noise(UINT strength) -> void {
        if (strength == current_noise_strength) {
            return;
        }

        // Same static 256x256, nearest/repeat, [0, strength) noise distribution as KDE.
        // Fixed seed makes regression tests reproducible; KDE seeds from the current time.
        std::mt19937 random(0x4b4445);
        std::vector<Pixel> pixels(256 * 256);

        for (auto& pixel : pixels) {
            const auto value = static_cast<uint8_t>(strength ? random() % strength : 0);
            pixel = {value, value, value, 255};
        }

        noise_view = make_surface(device.Get(), 256, 256, pixels.data()).view;
        current_noise_strength = strength;
    }

    auto BlurRenderer::draw(ID3D11ShaderResourceView* source,
                            ID3D11RenderTargetView* target,
                            UINT width,
                            UINT height,
                            ID3D11PixelShader* shader,
                            const ShaderConstants& constants) -> void {
        context->UpdateSubresource(constant_buffer.Get(), 0, nullptr, &constants, 0, 0);
        context->OMSetRenderTargets(1, &target, nullptr);
        context->OMSetBlendState(nullptr, nullptr, UINT_MAX);
        context->IASetInputLayout(nullptr);
        context->IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        context->VSSetShader(vertex_shader.Get(), nullptr, 0);
        context->PSSetShader(shader, nullptr, 0);
        context->PSSetConstantBuffers(0, 1, constant_buffer.GetAddressOf());

        ID3D11ShaderResourceView* sources[] = {source, noise_view.Get()};
        ID3D11SamplerState* samplers[] = {linear_sampler.Get(), noise_sampler.Get()};
        context->PSSetShaderResources(0, 2, sources);
        context->PSSetSamplers(0, 2, samplers);

        D3D11_VIEWPORT viewport{0.0f, 0.0f, static_cast<float>(width), static_cast<float>(height), 0.0f, 1.0f};
        context->RSSetViewports(1, &viewport);
        context->Draw(3, 0);

        ID3D11ShaderResourceView* empty_views[] = {nullptr, nullptr};
        context->PSSetShaderResources(0, 2, empty_views);
        context->OMSetRenderTargets(0, nullptr, nullptr);
    }

    auto BlurRenderer::crop(
        ID3D11ShaderResourceView* source, UINT width, UINT height, LONG x, LONG y, DXGI_MODE_ROTATION rotation)
        -> void {
        if (rotation == DXGI_MODE_ROTATION_ROTATE90 || rotation == DXGI_MODE_ROTATION_ROTATE270) {
            std::swap(width, height);
        }

        ShaderConstants constants;
        constants.rotation = rotation;
        constants.crop_origin[0] = static_cast<float>(x) / width;
        constants.crop_origin[1] = static_cast<float>(y) / height;
        constants.crop_scale[0] = static_cast<float>(result.width) / width;
        constants.crop_scale[1] = static_cast<float>(result.height) / height;

        draw(source, levels[0].target.Get(), result.width, result.height, copy_shader.Get(), constants);
    }

    auto BlurRenderer::upload(const std::vector<Pixel>& pixels) -> void {
        if (pixels.size() != static_cast<size_t>(result.width) * result.height) {
            throw std::runtime_error("blur input size does not match the surface");
        }

        context->UpdateSubresource(levels[0].texture.Get(), 0, nullptr, pixels.data(), result.width * sizeof(Pixel), 0);
    }

    auto BlurRenderer::render(const BlurSettings& settings, float noise_scale) -> void {
        if (settings.iterations < 1 || settings.iterations > 4 || settings.noise_strength > 255) {
            throw std::runtime_error("invalid blur settings");
        }

        update_noise(settings.noise_strength);

        ShaderConstants constants;
        constants.offset = settings.offset;
        constants.brightness = settings.brightness;
        constants.saturation = settings.saturation;
        constants.contrast = settings.contrast;
        constants.blur_size[0] = static_cast<float>(result.width);
        constants.blur_size[1] = static_cast<float>(result.height);

        for (UINT index = 1; index <= settings.iterations; ++index) {
            const auto& source = levels[index - 1];
            const auto& target = levels[index];
            constants.half_pixel[0] = 0.5f / source.width;
            constants.half_pixel[1] = 0.5f / source.height;
            constants.transform_colors = index == 1;

            draw(source.view.Get(),
                 target.target.Get(),
                 target.width,
                 target.height,
                 downsample_shader.Get(),
                 constants);
        }

        constants.transform_colors = 0;

        for (UINT index = settings.iterations; index > 0; --index) {
            const auto& source = levels[index];
            const auto& target = index == 1 ? result : levels[index - 1];
            constants.half_pixel[0] = 0.5f / source.width;
            constants.half_pixel[1] = 0.5f / source.height;
            constants.final_pass = index == 1;
            constants.noise_enabled = settings.noise_strength > 0;
            constants.noise_scale = noise_scale;
            constants.refraction_strength = settings.refraction_strength;
            constants.refraction_edge =
                std::clamp(settings.refraction_edge, 0.001f, std::min(result.width, result.height) * 0.5f);
            constants.refraction_power = settings.refraction_power;
            constants.refraction_fringing = settings.refraction_fringing;

            draw(source.view.Get(), target.target.Get(), target.width, target.height, upsample_shader.Get(), constants);
        }
    }

    auto BlurRenderer::present(ID3D11RenderTargetView* target) -> void {
        draw(result.view.Get(), target, result.width, result.height, copy_shader.Get(), ShaderConstants{});
    }

    auto BlurRenderer::input() const -> const Surface& {
        return levels[0];
    }

    auto BlurRenderer::output() const -> const Surface& {
        return result;
    }
} // namespace launcher
