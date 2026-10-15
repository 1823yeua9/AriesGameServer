#include "application.h"

#include <algorithm>
#include <cmath>
#include <fstream>
#include <stdexcept>

namespace launcher::tests {
    struct Image {
        UINT size;
        std::vector<Pixel> pixels;
    };

    auto sample(const Image& image, double x, double y) -> std::array<double, 4> {
        x = std::clamp(x, 0.0, static_cast<double>(image.size - 1));
        y = std::clamp(y, 0.0, static_cast<double>(image.size - 1));
        const auto x0 = static_cast<UINT>(x);
        const auto y0 = static_cast<UINT>(y);
        const auto x1 = std::min(x0 + 1, image.size - 1);
        const auto y1 = std::min(y0 + 1, image.size - 1);
        const auto horizontal = x - x0;
        const auto vertical = y - y0;
        const auto pixel = [&](UINT column, UINT row, UINT channel) {
            const auto& value = image.pixels[static_cast<size_t>(row) * image.size + column];

            return reinterpret_cast<const uint8_t*>(&value)[channel];
        };

        std::array<double, 4> result{};

        for (UINT channel = 0; channel < 4; ++channel) {
            const auto top = pixel(x0, y0, channel) * (1.0 - horizontal) + pixel(x1, y0, channel) * horizontal;
            const auto bottom = pixel(x0, y1, channel) * (1.0 - horizontal) + pixel(x1, y1, channel) * horizontal;
            result[channel] = top * (1.0 - vertical) + bottom * vertical;
        }

        return result;
    }

    auto reference_pass(const Image& source, UINT size, bool downsample, const BlurSettings& settings, bool transform)
        -> Image {
        // Independent CPU evaluation of the pinned upstream GLSL, including RGBA8
        // quantization after every framebuffer pass and GL_LINEAR/CLAMP_TO_EDGE.
        constexpr double down_offsets[][3] = {{0, 0, 4}, {-1, -1, 1}, {1, 1, 1}, {1, -1, 1}, {-1, 1, 1}};
        constexpr double up_offsets[][3] = {
            {-2, 0, 1}, {-1, 1, 2}, {0, 2, 1}, {1, 1, 2}, {2, 0, 1}, {1, -1, 2}, {0, -2, 1}, {-1, -1, 2}};
        const auto* offsets = downsample ? down_offsets : up_offsets;
        const UINT count = downsample ? 5 : 8;
        const auto divisor = downsample ? 8.0 : 12.0;
        Image result{size, std::vector<Pixel>(static_cast<size_t>(size) * size)};

        for (UINT y = 0; y < size; ++y) {
            for (UINT x = 0; x < size; ++x) {
                const auto source_x = (x + 0.5) * source.size / size - 0.5;
                const auto source_y = (y + 0.5) * source.size / size - 0.5;
                std::array<double, 4> sum{};

                for (UINT index = 0; index < count; ++index) {
                    const auto value = sample(source,
                                              source_x + offsets[index][0] * settings.offset * 0.5,
                                              source_y + offsets[index][1] * settings.offset * 0.5);

                    for (UINT channel = 0; channel < 4; ++channel) {
                        sum[channel] += value[channel] * offsets[index][2] / divisor;
                    }
                }

                if (transform) {
                    for (UINT channel = 0; channel < 3; ++channel) {
                        sum[channel] = sum[channel] * settings.contrast + (1.0 - settings.contrast) * 0.5 * sum[3];
                    }

                    const auto luminance = sum[2] * 0.2126 + sum[1] * 0.7152 + sum[0] * 0.0722;

                    for (UINT channel = 0; channel < 3; ++channel) {
                        sum[channel] = (sum[channel] * settings.saturation + luminance * (1.0 - settings.saturation)) *
                                       settings.brightness;
                    }
                }

                auto& pixel = result.pixels[static_cast<size_t>(y) * size + x];
                auto* channels = reinterpret_cast<uint8_t*>(&pixel);

                for (UINT channel = 0; channel < 4; ++channel) {
                    channels[channel] = static_cast<uint8_t>(std::clamp(std::lround(sum[channel]), 0l, 255l));
                }
            }
        }

        return result;
    }

    auto reference_blur(Image input, const BlurSettings& settings) -> Image {
        for (UINT index = 0; index < settings.iterations; ++index) {
            input = reference_pass(input, input.size / 2, true, settings, index == 0);
        }

        for (UINT index = settings.iterations; index > 0; --index) {
            input = reference_pass(input, square_size >> (index - 1), false, settings, false);
        }

        return input;
    }

    auto fixture(UINT kind) -> Image {
        Image image{square_size, std::vector<Pixel>(square_size * square_size)};

        for (UINT y = 0; y < square_size; ++y) {
            for (UINT x = 0; x < square_size; ++x) {
                auto& pixel = image.pixels[static_cast<size_t>(y) * square_size + x];

                if (kind == 0) {
                    pixel = {64, 128, 192, 255};
                }
                else if (kind == 1) {
                    pixel = {
                        static_cast<uint8_t>(x < 200 ? 16 : 240), 16, static_cast<uint8_t>(x < 200 ? 240 : 16), 255};
                }
                else {
                    pixel = {static_cast<uint8_t>((x * 255) / 399),
                             static_cast<uint8_t>((y * 255) / 399),
                             static_cast<uint8_t>(((x / 7 + y / 11) % 2) ? 240 : 16),
                             255};
                }
            }
        }

        return image;
    }

    auto exercise_device(D3D_DRIVER_TYPE driver, std::ofstream& log, const std::filesystem::path& directory) -> void {
        ComPtr<ID3D11Device> device;
        ComPtr<ID3D11DeviceContext> context;
        const D3D_FEATURE_LEVEL feature_levels[] = {D3D_FEATURE_LEVEL_11_0};
        check(D3D11CreateDevice(nullptr,
                                driver,
                                nullptr,
                                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                                feature_levels,
                                1,
                                D3D11_SDK_VERSION,
                                &device,
                                nullptr,
                                &context),
              "test graphics device");

        BlurRenderer renderer(device.Get(), context.Get());

        for (UINT index = 0; index < 4; ++index) {
            auto input = fixture(std::min(index, 2u));
            BlurSettings settings;
            settings.noise_strength = 0;

            if (index == 3) {
                settings.brightness = 0.8f;
                settings.saturation = 0.6f;
                settings.contrast = 1.2f;
                settings.iterations = 3;
                settings.offset = 4.4f;
            }

            renderer.upload(input.pixels);
            renderer.render(settings);
            const auto actual = read_surface(device.Get(), context.Get(), renderer.output());
            const auto expected = reference_blur(input, settings);
            int maximum_error = 0;
            double total_error = 0.0;

            for (size_t pixel = 0; pixel < actual.size(); ++pixel) {
                for (UINT channel = 0; channel < 3; ++channel) {
                    const auto observed = reinterpret_cast<const uint8_t*>(&actual[pixel])[channel];
                    const auto reference = reinterpret_cast<const uint8_t*>(&expected.pixels[pixel])[channel];
                    const auto difference = std::abs(static_cast<int>(observed) - reference);
                    maximum_error = std::max(maximum_error, difference);
                    total_error += difference;
                }

                if (actual[pixel].alpha != 255) {
                    throw std::runtime_error("Opaque square alpha mismatch.");
                }
            }

            const auto mean_error = total_error / (actual.size() * 3);
            log << "driver=" << driver << " fixture=" << index << " max_error=" << maximum_error
                << "/255 mean_error=" << mean_error << "/255\n";

            if (maximum_error > 3 || mean_error > 1.0) {
                throw std::runtime_error("GPU blur diverges from the upstream sampling reference.");
            }

            if (driver == D3D_DRIVER_TYPE_HARDWARE && index == 2) {
                save_bitmap(directory / "fixture-source.bmp", square_size, square_size, input.pixels);
                save_bitmap(directory / "fixture-gpu.bmp", square_size, square_size, actual);
                save_bitmap(directory / "fixture-reference.bmp", square_size, square_size, expected.pixels);
            }
        }

        const auto constant = fixture(0);
        renderer.upload(constant.pixels);
        renderer.render(BlurSettings{});
        const auto smooth = read_surface(device.Get(), context.Get(), renderer.output());

        if (memcmp(smooth.data(), constant.pixels.data(), smooth.size() * sizeof(Pixel)) != 0) {
            throw std::runtime_error("Default finish adds grain or changes a uniform backdrop.");
        }

        log << "PASS: default finish preserves uniform backdrop exactly, no grain\n";

        BlurSettings grain_settings;
        grain_settings.noise_strength = 5;
        renderer.render(grain_settings);
        const auto grain = read_surface(device.Get(), context.Get(), renderer.output());
        renderer.render(grain_settings);
        const auto repeated = read_surface(device.Get(), context.Get(), renderer.output());
        std::array<bool, 5> observed_values{};

        for (size_t index = 0; index < grain.size(); ++index) {
            const auto delta = static_cast<int>(grain[index].blue) - constant.pixels[index].blue;

            if (delta < 0 || delta > 4 || memcmp(&grain[index], &repeated[index], sizeof(Pixel)) != 0) {
                throw std::runtime_error("Noise is outside KDE's distribution or is temporally unstable.");
            }

            observed_values[delta] = true;
        }

        if (!std::all_of(observed_values.begin(), observed_values.end(), [](bool value) {
                return value;
            })) {
            throw std::runtime_error("Noise texture does not cover all default values.");
        }

        log << "PASS: static grain distribution 0..4, repeat frame identical, alpha=255\n";
    }
} // namespace launcher::tests

namespace launcher {
    auto run_blur_tests(const std::filesystem::path& directory) -> int {
        std::filesystem::create_directories(directory);
        std::ofstream log(directory / "shader-tests.txt");
        log << std::unitbuf;

        try {
            tests::exercise_device(D3D_DRIVER_TYPE_WARP, log, directory);
            tests::exercise_device(D3D_DRIVER_TYPE_HARDWARE, log, directory);
            log << "PASS: both software and hardware D3D11, four reference fixtures each.\n";

            return 0;
        }
        catch (const std::exception& error) {
            log << "FAIL: " << error.what() << '\n';

            return 1;
        }
    }
} // namespace launcher
