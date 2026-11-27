// Where an effect layer stands and how far through its run it is, as the engine sets it each
// frame: what an effect's vertex and fragment stages read alike.
struct Fx {
    // xyz where the layer stands, w the heading its vfx travels.
    origin: vec4<f32>,
    // How the layer is turned, a quaternion.
    rotation: vec4<f32>,
    // xyz its extent in metres, w a seed of its own.
    scale: vec4<f32>,
    // x how far through its run, y its age in seconds, z the age at which it ends for good, 0 for
    // one that plays on, w the depth in metres over which it softens where it meets what stands
    // behind it, 0 for none.
    life: vec4<f32>,
    // An area's radius, a cone's angle in degrees, its length, and a path's length.
    site: vec4<f32>,
    // x how far a coat stands off its surface, in metres; y what it is drawn on: 0 a shape of its
    // own, 1 a model's surface, 2 the ground; z how far the thing it plays on has faded from sight.
    extra: vec4<f32>,
    // A sphere the layer is seen within, fading out past it: xyz its centre, w its radius, 0 for
    // none.
    mask: vec4<f32>,
}

@group(3) @binding(0) var<uniform> frame: Fx;
