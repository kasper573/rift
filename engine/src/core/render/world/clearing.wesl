import bevy_pbr::render::mesh_view_bindings::view;

struct FadeStyle {
    focus: vec2<f32>,
    shown: f32,
    focus_radius: f32,
}

@group(3) @binding(100) var<uniform> fade: FadeStyle;

// Where the clearing starts to soften, as a share of its radius.
const RIM: f32 = 0.55;

// Drops the pixel when the clearing takes it: a thing standing in front of the camera's subject
// is cleared in a circle of `focus_radius` around `focus`, where the subject shows, softening out
// to its rim, `shown` being how far that clearing has come (1 untouched, 0 the whole circle). The
// cleared share of a pixel is dropped by a dither that shifts every frame where the picture
// settles, which the temporal anti-aliasing turns into a plain translucency, so the thing keeps
// its depth and its lighting and never shows its inside. Every pass of the frame drops the same
// pixels.
fn clear_pixel(pixel: vec2<f32>) {
    let spread = distance(pixel, fade.focus) / max(fade.focus_radius, 1.0);
    let rim = smoothstep(RIM, 1.0, spread);
    let visibility = mix(rim, 1.0, fade.shown);
    if dither(pixel) >= visibility {
        discard;
    }
}

// Interleaved gradient noise, stepped on by the golden ratio each frame where the picture
// settles.
fn dither(pixel: vec2<f32>) -> f32 {
    var p = pixel;
    @if(SETTLING) {
        p += 5.588238 * f32(view.frame_count % 64u);
    }
    return fract(52.9829189 * fract(0.06711056 * p.x + 0.00583715 * p.y));
}
