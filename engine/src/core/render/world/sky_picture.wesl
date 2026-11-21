// The sky's picture: the whole sky above the horizon in one round picture, straight up at its
// middle and the horizon on the circle that touches its sides, each way as far from the middle as
// it leans from straight up.

const SKY_QUARTER_TURN: f32 = 1.5707964;
// How far inside the horizon's circle a way under the horizon is read, as a share of the
// circle's radius: past it the picture holds the ground's side of the horizon's blur.
const SKY_PICTURE_RIM: f32 = 0.01;

// Where in the picture the sky toward `way` lies, a unit vector up the y axis; a way under the
// horizon is held to it.
fn sky_round(way: vec3<f32>) -> vec2<f32> {
    let leaning = min(acos(clamp(way.y, -1.0, 1.0)) / SKY_QUARTER_TURN, 1.0 - SKY_PICTURE_RIM);
    let across = way.xz / max(length(way.xz), 1e-6);
    return 0.5 + 0.5 * across * leaning;
}

// The way of the sky a place in the picture shows: past the horizon's circle, the horizon's own
// on that side, so the picture's coarser sizes hold no ground at their rim.
fn sky_way(round: vec2<f32>) -> vec3<f32> {
    let off = round * 2.0 - 1.0;
    let leaning = min(length(off), 1.0) * SKY_QUARTER_TURN;
    let across = off / max(length(off), 1e-6);
    return vec3<f32>(across.x * sin(leaning), cos(leaning), across.y * sin(leaning));
}
