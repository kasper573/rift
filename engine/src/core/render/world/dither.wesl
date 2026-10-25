// A 4 × 4 Bayer matrix: the pixels of the picture are dropped in this fixed order as a thing fades,
// so what fades needs no sorting, keeps its shadow and stays crisp in the low-resolution picture.
fn bayer(pixel: vec2<f32>) -> f32 {
    let x = u32(pixel.x) % 4u;
    let y = u32(pixel.y) % 4u;
    let matrix = array<f32, 16>(
        0.0, 8.0, 2.0, 10.0,
        12.0, 4.0, 14.0, 6.0,
        3.0, 11.0, 1.0, 9.0,
        15.0, 7.0, 13.0, 5.0,
    );
    return (matrix[y * 4u + x] + 0.5) / 16.0;
}

// Drops the pixel when the thing drawn shows less than its share: `shown` 1 keeps every pixel.
fn dither_discard(pixel: vec2<f32>, shown: f32) {
    if shown < 1.0 && bayer(pixel) >= shown {
        discard;
    }
}
