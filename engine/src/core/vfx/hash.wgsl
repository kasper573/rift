// A number in 0..1 drawn from a seed, the same on every device: hashed on the seed's bits, where
// a hash built on sin() breaks into blocks wherever a device computes sin() loosely.
fn hash1(seed: f32) -> f32 {
    return f32(hash_bits(bitcast<u32>(seed))) / 4294967295.0;
}

// A number in 0..1 drawn from a point, the same on every device.
fn hash2(point: vec2<f32>) -> f32 {
    return f32(hash_bits(bitcast<u32>(point.x) ^ hash_bits(bitcast<u32>(point.y)))) / 4294967295.0;
}

fn hash_bits(bits: u32) -> u32 {
    var state = bits * 747796405u + 2891336453u;
    state = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (state >> 22u) ^ state;
}
