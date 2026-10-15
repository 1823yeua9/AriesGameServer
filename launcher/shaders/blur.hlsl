// HLSL translation of can1357/kde-blur at 4f86e779512716366554d59dbad2beed277dba6a.
// See ../reference/NOTICE.txt, upstream GLSL, and GPL license in ../reference/COPYING.

cbuffer BlurConstants : register(b0) {
    float2 half_pixel;
    float sample_offset;
    uint transform_colors;

    float brightness;
    float saturation;
    float contrast;
    uint final_pass;

    float2 crop_origin;
    float2 crop_scale;

    float refraction_strength;
    float refraction_edge;
    float refraction_power;
    float refraction_fringing;

    float2 blur_size;
    float noise_scale;
    uint noise_enabled;

    uint rotation;
    float3 rotation_padding;
};

Texture2D image_texture : register(t0);
Texture2D noise_texture : register(t1);
SamplerState linear_sampler : register(s0);
SamplerState noise_sampler : register(s1);

struct VertexOutput {
    float4 position : SV_Position;
    float2 uv : TEXCOORD0;
};

VertexOutput vertex_main(uint vertex_id : SV_VertexID) {
    VertexOutput output;
    output.uv = float2((vertex_id << 1) & 2, vertex_id & 2);
    output.position = float4(output.uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);

    return output;
}

float4 copy_main(VertexOutput input) : SV_Target {
    float2 uv = crop_origin + input.uv * crop_scale;

    if (rotation == 2) {
        uv = float2(uv.y, 1.0 - uv.x);
    }
    else if (rotation == 3) {
        uv = 1.0 - uv;
    }
    else if (rotation == 4) {
        uv = float2(1.0 - uv.y, uv.x);
    }

    return float4(image_texture.SampleLevel(linear_sampler, uv, 0).rgb, 1.0);
}

float4 downsample_main(VertexOutput input) : SV_Target {
    float2 displacement = half_pixel * sample_offset;
    float4 sum = image_texture.SampleLevel(linear_sampler, input.uv, 0) * 4.0;
    sum += image_texture.SampleLevel(linear_sampler, input.uv - displacement, 0);
    sum += image_texture.SampleLevel(linear_sampler, input.uv + displacement, 0);
    sum += image_texture.SampleLevel(linear_sampler, input.uv + displacement * float2(1.0, -1.0), 0);
    sum += image_texture.SampleLevel(linear_sampler, input.uv - displacement * float2(1.0, -1.0), 0);
    sum /= 8.0;

    if (transform_colors) {
        // GLSL uses a row vector: color * contrast * saturation * brightness.
        float3 color = sum.rgb * contrast + (1.0 - contrast) * 0.5 * sum.a;
        float luminance = dot(color, float3(0.2126, 0.7152, 0.0722));
        sum.rgb = (color * saturation + luminance * (1.0 - saturation)) * brightness;
    }

    return sum;
}

float rounded_rectangle_distance(float2 position, float2 half_size, float radius) {
    float2 q = abs(position) - half_size + radius;

    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - radius;
}

float4 upsample_main(VertexOutput input) : SV_Target {
    const float2 offsets[8] = {
        float2(-2.0, 0.0), float2(-1.0, 1.0), float2(0.0, 2.0), float2(1.0, 1.0),
        float2(2.0, 0.0), float2(1.0, -1.0), float2(0.0, -2.0), float2(-1.0, -1.0)
    };
    const float weights[8] = {1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0};
    float2 coordinate_r = input.uv;
    float2 coordinate_g = input.uv;
    float2 coordinate_b = input.uv;
    bool refract = final_pass && refraction_strength > 0.0;

    if (refract) {
        float2 half_size = blur_size * 0.5;
        float2 position = input.uv * blur_size - half_size;
        float distance = rounded_rectangle_distance(position, half_size, refraction_edge);
        float concave = pow(saturate(1.0 + distance / refraction_edge), refraction_power);
        float2 gradient = float2(
            rounded_rectangle_distance(position + float2(1.0, 0.0), half_size, refraction_edge)
                - rounded_rectangle_distance(position - float2(1.0, 0.0), half_size, refraction_edge),
            rounded_rectangle_distance(position + float2(0.0, 1.0), half_size, refraction_edge)
                - rounded_rectangle_distance(position - float2(0.0, 1.0), half_size, refraction_edge));
        float2 normal = length(gradient) > 1e-6 ? -normalize(gradient) : float2(0.0, 1.0);
        float strength = 0.2 * concave * refraction_strength;
        float fringing = refraction_fringing * 0.3;
        coordinate_r = saturate(input.uv - normal * strength * (1.0 + fringing));
        coordinate_g = saturate(input.uv - normal * strength);
        coordinate_b = saturate(input.uv - normal * strength * (1.0 - fringing));
    }

    float4 sum = 0.0;

    [unroll]
    for (uint index = 0; index < 8; ++index) {
        float2 displacement = offsets[index] * half_pixel * sample_offset;

        if (refract) {
            sum.r += image_texture.SampleLevel(linear_sampler, coordinate_r + displacement, 0).r * weights[index];
            sum.g += image_texture.SampleLevel(linear_sampler, coordinate_g + displacement, 0).g * weights[index];
            sum.b += image_texture.SampleLevel(linear_sampler, coordinate_b + displacement, 0).b * weights[index];
        }
        else {
            sum += image_texture.SampleLevel(linear_sampler, input.uv + displacement, 0) * weights[index];
        }
    }

    sum /= 12.0;

    if (final_pass && noise_enabled) {
        // Static grain, not temporal noise. No shimmer on an unchanged backdrop.
        sum.rgb += noise_texture.SampleLevel(noise_sampler, input.position.xy / (256.0 * noise_scale), 0).rrr;
    }

    // The requested square matches KDE's default corner radius 0 and opacity 100%.
    return float4(sum.rgb, 1.0);
}
