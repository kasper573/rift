use engine::core::vfx::{ParamKind, ShaderParam, Stage, VfxShaderDef};

const SOURCE: &str = "fn vfx(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(uv, 0.0, 1.0); }\n";

#[test]
fn a_shaders_parameters_bind_in_whole_16_byte_rows_as_webgl2_requires() {
    let shapes: [&[ParamKind]; 5] = [
        &[],
        &[ParamKind::Number],
        &[ParamKind::Bool, ParamKind::Number, ParamKind::Number],
        &[ParamKind::Color],
        &[ParamKind::Number, ParamKind::Color, ParamKind::Number],
    ];
    for kinds in shapes {
        let def = VfxShaderDef {
            name: "Probe".to_owned(),
            stage: Stage::Billboard,
            params: kinds
                .iter()
                .enumerate()
                .map(|(index, &kind)| ShaderParam {
                    name: format!("p{index}"),
                    kind,
                    default: kind.blank(),
                    min: None,
                    max: None,
                })
                .collect(),
        };
        let composed = def.check_source(SOURCE).expect("the shader compiles");
        let size = params_binding_size(&composed);
        assert!(
            size > 0 && size.is_multiple_of(16),
            "{kinds:?} bind {size} bytes of parameters"
        );
    }
}

fn params_binding_size(composed: &str) -> u32 {
    let module = naga::front::wgsl::parse_str(composed).expect("the shader parses");
    let mut layouter = naga::proc::Layouter::default();
    layouter
        .update(module.to_ctx())
        .expect("the shader lays out");
    let params = module
        .global_variables
        .iter()
        .find(|(_, var)| var.name.as_deref() == Some("params"))
        .expect("a params binding")
        .1;
    layouter[params.ty].size
}
