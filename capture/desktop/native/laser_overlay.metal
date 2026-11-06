#include <metal_stdlib>
using namespace metal;

struct LaserUniforms {
    float2 resolution;
    float scale;
    float now_sec;
    float tail_duration;
    float tip_active;
    float2 tip_pos;
    uint segment_count;
    uint pin_count;
    float constrain_rect;
    float _pad1;
    float4 target_rect;
    // x: 1 when a full-screen recording started straight from the shortcut (no picker).
    float4 glow;
    uint tile_cols;
    uint tile_rows;
    float tile_size;
    float _pad2;
};

// Each segment connects A -> B in logical screen points.
// kind == 0.0: tapered laser segment (w0..w1 = combined arc + time weight 0..1)
// kind == 1.0: persistent ink segment (w0..w1 = simulated pressure scale)
struct PathSegment {
    float2 a;
    float2 b;
    float w0;
    float w1;
    float kind;
    float width;
    uint rgb;
};

// A pin's brief live pulse: a thin ring at `radius`, `alpha` opaque.
struct PinMarker {
    float2 pos;
    float radius;
    float alpha;
    uint rgb;
};

float3 recorded_srgb(uint packed) {
    uint value = packed - 1u;
    // The overlay's BGRA8Unorm attachment stores display sRGB channel values directly.
    return float3((value >> 16u) & 255u, (value >> 8u) & 255u, value & 255u) / 255.0;
}

struct VertexOutput {
    float4 position [[position]];
    float2 uv;
};

vertex VertexOutput laser_vertex(uint vertex_id [[vertex_id]]) {
    const float2 positions[] = {
        float2(-1.0, -1.0),
        float2( 3.0, -1.0),
        float2(-1.0,  3.0),
    };
    VertexOutput out;
    out.position = float4(positions[vertex_id], 0.0, 1.0);
    out.uv = positions[vertex_id] * float2(0.5, -0.5) + 0.5;
    return out;
}

// Each frame picks a specialisation (`encode_overlay`), so it compiles only the work it needs:
// an unused tile loop or early return still costs every pixel registers, and so occupancy.
// `has_ink`: some segment is binned. `skip_clear`: no picker, constrained target or intro effect
// shows (`LaserUniforms::has_backdrop`), so a pixel nothing reaches can return clear at once.
constant bool has_ink [[function_constant(0)]];
constant bool skip_clear [[function_constant(1)]];

fragment float4 laser_fragment(
    VertexOutput in [[stage_in]],
    constant LaserUniforms &u [[buffer(0)]],
    const device PathSegment *segments [[buffer(1)]],
    const device PinMarker *pins [[buffer(2)]],
    // `tile_cols * tile_rows + 1` offsets, then each tile's segment indices (`bin_segments`).
    const device uint *tiles [[buffer(3)]]
) {
    // Pixel position in logical screen points (top-left origin).
    float2 p = (in.uv * u.resolution) / max(u.scale, 1.0);

    float3 laser_red = float3(1.0, 0.16, 0.19);
    float3 laser_color = laser_red;
    float laser_color_weight = 0.0;
    float3 laser_hot = float3(1.0, 0.97, 0.97);

    float laser_core_cov = 0.0;
    float laser_body_cov = 0.0;
    float laser_glow_cov = 0.0;
    float ink_red_cov = 0.0;
    float3 ink_color = float3(1.0, 0.23, 0.19);

    uint tile_count = u.tile_cols * u.tile_rows;
    uint2 cell = min(uint2(max(p, float2(0.0)) / u.tile_size), uint2(u.tile_cols, u.tile_rows) - 1u);
    uint tile = cell.y * u.tile_cols + cell.x;
    uint first = has_ink && tile_count > 0u ? tiles[tile] : 0u;
    uint last = has_ink && tile_count > 0u ? tiles[tile + 1u] : 0u;
    // No segment, pin or Shift tip reaches this pixel and nothing is behind it: it stays clear.
    if (skip_clear && first == last && u.pin_count == 0u
        && (u.tip_active <= 0.01 || length(p - u.tip_pos) >= 5.0)) {
        return float4(0.0);
    }
    for (uint j = first; j < last; ++j) {
        PathSegment seg = segments[tiles[tile_count + 1u + j]];
        float2 ab = seg.b - seg.a;
        float len2 = max(dot(ab, ab), 0.0001);
        float h = clamp(dot(p - seg.a, ab) / len2, 0.0, 1.0);
        float2 closest = seg.a + ab * h;
        float d = length(p - closest);
        float w = clamp(mix(seg.w0, seg.w1, h), 0.0, 1.25);

        if (seg.kind < 0.5) {
            if (w <= 0.002) {
                continue;
            }
            float glow_sigma = max(1.4, 6.2 * (0.28 + 0.72 * w));
            if (d < glow_sigma * 3.0) {
                float body_r = 2.45 * pow(w, 0.68);
                float core_r = 0.90 * pow(w, 0.82);
                float body = smoothstep(body_r + 0.85, max(0.0, body_r - 0.85), d) * pow(w, 0.40);
                float core = smoothstep(core_r + 0.75, max(0.0, core_r - 0.75), d) * pow(w, 0.72);
                float glow = exp(-(d * d) / (glow_sigma * glow_sigma)) * pow(w, 0.92);
                // The bloom reaches beyond the body. Choose hue from the full
                // optical contribution, including pixels with no body coverage.
                float color_weight = glow * 0.42 + body * 0.88 + core * 0.95;
                if (color_weight > laser_color_weight) {
                    laser_color_weight = color_weight;
                    laser_color = seg.rgb != 0u ? recorded_srgb(seg.rgb) : laser_red;
                }
                laser_body_cov = max(laser_body_cov, body);
                laser_core_cov = max(laser_core_cov, core);
                laser_glow_cov = max(laser_glow_cov, glow);
            }
        } else {
            // Simple tldraw-style thin red marker stroke (~4.5px wide, smooth anti-aliased edge, fades out after release).
            float r = 2.25 * clamp(w, 0.55, 1.15);
            if (d < r + 2.0) {
                float fill = smoothstep(r + 0.70, max(0.0, r - 0.70), d) * clamp(seg.width, 0.0, 1.0);
                if (fill > ink_red_cov) {
                    ink_red_cov = fill;
                    ink_color = seg.rgb != 0u ? recorded_srgb(seg.rgb) : float3(1.0, 0.23, 0.19);
                }
            }
        }
    }

    // Pin pulses: a 1.5 pt ring, no fill, so nothing under it is hidden.
    uint pcount = min(u.pin_count, 64u);
    for (uint i = 0; i < pcount; ++i) {
        PinMarker pin = pins[i];
        float off = abs(length(p - pin.pos) - pin.radius);
        if (off < 2.0) {
            float ring = smoothstep(1.25, 0.25, off) * pin.alpha;
            if (ring > ink_red_cov) {
                ink_red_cov = ring;
                ink_color = pin.rgb != 0u ? recorded_srgb(pin.rgb) : float3(1.0, 0.23, 0.19);
            }
        }
    }

    // Subtle red marker dot at the cursor tip while Shift is held.
    if (u.tip_active > 0.01) {
        float d_tip = length(p - u.tip_pos);
        if (d_tip < 5.0) {
            float tip_fill = smoothstep(2.6, 1.4, d_tip) * u.tip_active;
            if (tip_fill > ink_red_cov) {
                ink_red_cov = tip_fill;
                uint packed_tip = as_type<uint>(u._pad2);
                ink_color = (packed_tip & 0x80000000u) != 0u
                    ? recorded_srgb(packed_tip & 0x01ffffffu)
                    : float3(1.0, 0.23, 0.19);
            }
        }
    }

    float4 out = float4(0.0);

    // Smooth 240ms transition from the picker's exact border/dim geometry into active recording.
    // `u.tail_duration` carries `picker_fade` (0..1 during the initial 150ms picker open, 1.0 during recording).
    float picker_fade = clamp(u.tail_duration, 0.0, 1.0);
    float t_intro = clamp(u.now_sec / 0.24, 0.0, 1.0);
    float intro_ease = 1.0 - pow(1.0 - t_intro, 3.0);
    float intro_fade = 1.0 - intro_ease;

    if (u.constrain_rect > 0.5) {
        bool is_device = u._pad1 > 2.5;
        bool is_window = u._pad1 > 1.5 && !is_device;
        float corner_r = is_device
            ? (min(u.target_rect.z, u.target_rect.w) * 0.135)
            : (is_window ? 10.0 : 0.0);
        float2 box_min = u.target_rect.xy;
        float2 box_max = u.target_rect.xy + u.target_rect.zw;
        float2 box_center = (box_min + box_max) * 0.5;
        float2 half_size = max((box_max - box_min) * 0.5 - float2(corner_r), float2(0.0));
        float2 q = abs(p - box_center) - half_size;
        float sdf = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - corner_r;

        // 1. Outside the target rect (`sdf > 0.0`): 0.32 dim during picker -> eases to 0.22 during recording.
        float out_cov = clamp(sdf + 0.5, 0.0, 1.0);
        if (out_cov > 0.001) {
            float dim_a = out_cov * mix(0.32, 0.22, intro_ease) * picker_fade;
            out.rgb = float3(0.0);
            out.a = dim_a;
        }

        // 2. Inside the selected window (`sdf <= 0.0`): dissolve the picker's blue tint (`0x3b5cf621`).
        float in_cov = 1.0 - out_cov;
        if (in_cov > 0.001 && is_window && intro_fade > 0.002) {
            float fill_a = in_cov * 0.13 * intro_fade * picker_fade;
            float3 fill_col = float3(0.231, 0.361, 0.965);
            out.rgb = fill_col * fill_a + out.rgb * (1.0 - fill_a);
            out.a = fill_a + out.a * (1.0 - fill_a);
        }

        // 3. Inset border (`sdf in [-border_w, 0.0]`): rendered by this exact same Metal layer during
        //    BOTH Selecting (`now_sec == 0`) and Recording (`now_sec > 0`), so there is zero window swap or jump.
        //    (`is_device` already renders its own titanium PhoneCaseView bezel.)
        if (!is_device) {
            float border_w = is_window ? mix(2.0, 1.0, intro_ease) : 1.0;
            float border_cov = clamp(border_w + 0.5 + sdf, 0.0, 1.0) * clamp(0.5 - sdf, 0.0, 1.0);
            if (border_cov > 0.001) {
                float3 picker_border = is_window ? float3(0.357, 0.486, 0.980) : float3(1.0, 1.0, 1.0);
                float3 border_col = mix(picker_border, float3(1.0, 1.0, 1.0), intro_ease);
                float border_alpha = border_cov * mix(is_window ? 0.93 : 0.92, 0.28, intro_ease) * picker_fade;
                out.rgb = border_col * border_alpha + out.rgb * (1.0 - border_alpha);
                out.a = border_alpha + out.a * (1.0 - border_alpha);
            }
        }
    } else if (u._pad1 < -0.0001) {
        // Picker open before any drag starts: uniform 0.32 * picker_fade dim.
        out.rgb = float3(0.0);
        out.a = 0.32 * clamp(-u._pad1, 0.0, 1.0);
    } else if (u.glow.x > 0.5) {
        // Full-screen recording started from the shortcut: a soft red glow along the display's
        // edges rises in 0.12s and is gone by 1.1s, so the start is visible without the picker.
        float env = smoothstep(0.0, 0.12, u.now_sec) * (1.0 - smoothstep(0.35, 1.1, u.now_sec));
        if (env > 0.002) {
            float2 lo = u.target_rect.xy;
            float2 hi = lo + u.target_rect.zw;
            float d = max(min(min(p.x - lo.x, hi.x - p.x), min(p.y - lo.y, hi.y - p.y)), 0.0);
            out.rgb = float3(1.0, 0.23, 0.19);
            out.a = exp(-d / 22.0) * 0.45 * env;
        }
    } else if (intro_fade > 0.002) {
        // Full-screen recording: simple 0.15s dim fade out.
        out.rgb = float3(0.0);
        out.a = 0.32 * intro_fade;
    }

    // 1. Composite ink (red fill)
    if (ink_red_cov > 0.001) {
        float a = ink_red_cov;
        out.rgb = ink_color * a + out.rgb * (1.0 - a);
        out.a = a + out.a * (1.0 - a);
    }

    // 2. Composite optical laser (soft atmospheric bloom + anti-aliased ruby ribbon + white-hot core)
    float laser_alpha = clamp(laser_glow_cov * 0.42 + laser_body_cov * 0.88 + laser_core_cov * 0.95, 0.0, 1.0);
    if (laser_alpha > 0.002) {
        float3 col = mix(laser_color, laser_hot, clamp(laser_core_cov * 0.92, 0.0, 1.0));
        out.rgb = col * laser_alpha + out.rgb * (1.0 - laser_alpha);
        out.a = laser_alpha + out.a * (1.0 - laser_alpha);
    }

    return float4(out.rgb * out.a, out.a);
}
