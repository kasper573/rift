// The ground an armed area skill would land on. It draws a crisp rim exactly on the shape, a soft
// glow stepping off it both ways, a faint floor and pulses that show where the cast goes: a
// ranged cast's rings close in on its centre, a melee cast's sweep out from the caster. A
// hostile cast swaps to its own colours, marches a dashed band round its rim and pulses
// faster. The shape comes from the site: a circle, a rectangle, or a cone with its tip at the
// caster.

// Signed distance in metres to the site's shape, negative inside; p is x right, y ahead.
fn shape_sdf(p: vec2<f32>) -> f32 {
    let s = frame.site;
    if s.y >= 359.0 {
        return length(p) - s.x;
    }
    if s.y <= 0.5 {
        let q = abs(p) - vec2<f32>(s.w, s.z) * 0.5;
        return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0);
    }
    let half = radians(s.y * 0.5);
    let c = vec2<f32>(sin(half), cos(half));
    let q = vec2<f32>(abs(p.x), p.y);
    let l = length(q) - s.z;
    let m = length(q - c * clamp(dot(q, c), 0.0, s.z));
    return max(l, m * sign(c.y * q.x - c.x * q.y));
}

fn fx(i: FxIn) -> vec4<f32> {
    let hostile = clamp(params.hostile, 0.0, 1.0);
    // Snaps open over its first tenth of a second.
    let open = 1.0 - pow(1.0 - clamp(i.age / 0.12, 0.0, 1.0), 3.0);
    let p = vec2<f32>(i.local.x * i.size.x, i.local.z * i.size.z) / mix(0.88, 1.0, open);
    let d = shape_sdf(p);
    let px = max(fwidth(d), 1e-4);
    let inside = 1.0 - smoothstep(-px, px, d);
    let color = mix(params.color.rgb, params.hostile_color.rgb, hostile);
    let rim_color = mix(params.rim.rgb, params.hostile_rim.rgb, hostile);

    let line = 1.0 - smoothstep(0.014, 0.014 + px * 1.5, abs(d));
    let glow = exp(-abs(d) / 0.06) * 0.5;
    let band = inside * exp(d / 0.25) * 0.3;
    let floor_fill = inside * 0.16;

    // Where the cast travels: inward from the rim for a ranged cast, outward from the caster
    // for a melee one.
    let speed = mix(0.7, 1.15, hostile);
    var travel: f32;
    if params.melee > 0.5 {
        var from_caster = length(p);
        if frame.site.y <= 0.5 {
            from_caster = p.y + frame.site.z * 0.5;
        }
        travel = from_caster / 0.55 - i.age * speed * 1.4;
    } else {
        travel = -d / 0.55 - i.age * speed;
    }
    let pulse = (1.0 - smoothstep(0.0, 0.08, abs(fract(travel) - 0.5) * 2.0 - 0.9)) * inside;
    let pulse_fade = smoothstep(0.0, 0.25, -d);

    // A hostile rim's dashes march round it, just outside the line.
    let around = atan2(p.x, p.y) * max(length(p), 0.5);
    let dash = step(0.5, fract(around / 0.22 - i.age * 1.2));
    let dashes = hostile * dash * window(d, 0.045, 0.075, px) ;

    // A ranged cast marks the point it is aimed at.
    var mark = 0.0;
    if params.melee < 0.5 {
        let r = length(p);
        let ring = 1.0 - smoothstep(0.012, 0.012 + px * 1.5, abs(r - 0.14));
        let dot = 1.0 - smoothstep(0.03, 0.03 + px, r);
        mark = max(ring, dot) * 0.8;
    }

    let a = clamp(max(line, max(glow, band + floor_fill + pulse * 0.3 * pulse_fade)) + dashes + mark, 0.0, 1.0);
    let rgb = mix(color * 1.1, rim_color * 2.2, clamp(line + dashes + mark + glow * 0.5, 0.0, 1.0));
    return vec4<f32>(rgb, a * open * params.strength);
}
