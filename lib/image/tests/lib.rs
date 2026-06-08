use image::{Image, Pixels, Pos, Region, Size, blit, decode_png, encode_png, png_size};

fn source(pixels: &[[u8; 4]], width: u32) -> Image {
    let mut image = Image::new(width, pixels.len() as u32 / width);
    for (slot, pixel) in image.rgba.chunks_mut(4).zip(pixels) {
        slot.copy_from_slice(pixel);
    }
    image
}

fn pixel(image: &Image, x: usize, y: usize) -> [u8; 4] {
    image.pixel(x, y).try_into().unwrap()
}

fn region(x: f32, y: f32, w: f32, h: f32) -> Region {
    Region::new(
        Pos::new(Pixels(x), Pixels(y)),
        Size::new(Pixels(w), Pixels(h)),
    )
}

fn at(x: f32, y: f32) -> Pos<Pixels> {
    Pos::new(Pixels(x), Pixels(y))
}

fn size(w: f32, h: f32) -> Size<Pixels> {
    Size::new(Pixels(w), Pixels(h))
}

#[test]
fn blit_copies_opaque_skips_transparent_blends_translucent() {
    let src = source(&[[10, 20, 30, 255], [9, 9, 9, 0], [100, 100, 100, 128]], 3);
    let mut dst = Image::new(3, 1);
    dst.rgba.copy_from_slice(&[50; 12]);
    blit(
        &mut dst,
        &src,
        region(0.0, 0.0, 3.0, 1.0),
        at(0.0, 0.0),
        size(3.0, 1.0),
        0xFFFF_FFFF,
        (false, false),
    );
    assert_eq!(pixel(&dst, 0, 0), [10, 20, 30, 255]);
    assert_eq!(pixel(&dst, 1, 0), [50, 50, 50, 50]);
    assert_eq!(pixel(&dst, 2, 0), [75, 75, 75, 255]);
}

#[test]
fn blit_clips_to_the_destination() {
    let src = source(&[[255, 0, 0, 255]; 4], 2);
    let mut dst = Image::new(2, 2);
    blit(
        &mut dst,
        &src,
        region(0.0, 0.0, 2.0, 2.0),
        at(-1.0, 1.0),
        size(2.0, 2.0),
        0xFFFF_FFFF,
        (false, false),
    );
    assert_eq!(pixel(&dst, 0, 1), [255, 0, 0, 255]);
    assert_eq!(pixel(&dst, 1, 1), [0, 0, 0, 0]);
    assert_eq!(pixel(&dst, 0, 0), [0, 0, 0, 0]);
    blit(
        &mut dst,
        &src,
        region(0.0, 0.0, 2.0, 2.0),
        at(10.0, 10.0),
        size(2.0, 2.0),
        0xFFFF_FFFF,
        (false, false),
    );
}

#[test]
fn blit_applies_tint_and_flip() {
    let src = source(&[[200, 100, 50, 255], [0, 0, 0, 255]], 2);
    let mut dst = Image::new(2, 1);
    blit(
        &mut dst,
        &src,
        region(0.0, 0.0, 2.0, 1.0),
        at(0.0, 0.0),
        size(2.0, 1.0),
        0x7FFF_00FF,
        (false, false),
    );
    assert_eq!(pixel(&dst, 0, 0), [99, 100, 0, 255]);
    let mut flipped = Image::new(2, 1);
    blit(
        &mut flipped,
        &src,
        region(0.0, 0.0, 2.0, 1.0),
        at(0.0, 0.0),
        size(2.0, 1.0),
        0xFFFF_FFFF,
        (true, false),
    );
    assert_eq!(pixel(&flipped, 0, 0), [0, 0, 0, 255]);
    assert_eq!(pixel(&flipped, 1, 0), [200, 100, 50, 255]);
}

#[test]
fn blit_scales_by_nearest_sampling() {
    let src = source(&[[255, 0, 0, 255], [0, 255, 0, 255]], 2);
    let mut dst = Image::new(4, 1);
    blit(
        &mut dst,
        &src,
        region(0.0, 0.0, 2.0, 1.0),
        at(0.0, 0.0),
        size(4.0, 1.0),
        0xFFFF_FFFF,
        (false, false),
    );
    assert_eq!(pixel(&dst, 0, 0), [255, 0, 0, 255]);
    assert_eq!(pixel(&dst, 1, 0), [255, 0, 0, 255]);
    assert_eq!(pixel(&dst, 2, 0), [0, 255, 0, 255]);
    assert_eq!(pixel(&dst, 3, 0), [0, 255, 0, 255]);
}

#[test]
fn png_round_trips() {
    let mut image = Image::new(4, 4);
    for pixel in image.rgba.chunks_mut(4) {
        pixel.copy_from_slice(&[1, 2, 3, 255]);
    }
    let decoded = decode_png(&encode_png(&image));
    assert_eq!((decoded.width, decoded.height), (4, 4));
    assert_eq!(decoded.rgba, image.rgba);
}

#[test]
fn png_size_reads_the_header_only() {
    let encoded = encode_png(&Image::new(7, 9));
    assert_eq!(png_size(&encoded), Some(size(7.0, 9.0)));
    assert_eq!(png_size(&encoded[..20]), None);
    assert_eq!(png_size(b"not a png"), None);
}
