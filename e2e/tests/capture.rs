use std::thread::sleep;
use std::time::{Duration, Instant};

use xcap::Window;

#[test]
#[ignore = "capture: records the gallery window; run with `just gallery`"]
fn capture_gallery() {
    let seconds: f32 = std::env::var("RIFT_CAPTURE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(140.0);
    let frames_dir = std::env::var("RIFT_CAPTURE_DIR").unwrap_or_else(|_| "/tmp/frames".to_owned());
    let _ = std::fs::remove_dir_all(&frames_dir);
    std::fs::create_dir_all(&frames_dir).expect("frames dir");

    let target = Duration::from_millis(40);
    let start = Instant::now();
    let mut frame = 0u32;
    while start.elapsed().as_secs_f32() < seconds {
        let tick = Instant::now();
        if let Some(window) = gallery()
            && let Ok(image) = window.capture_image()
        {
            image
                .save(format!("{frames_dir}/f{frame:05}.png"))
                .expect("save frame");
            frame += 1;
        }
        if let Some(rest) = target.checked_sub(tick.elapsed()) {
            sleep(rest);
        }
    }
    let fps = frame as f32 / start.elapsed().as_secs_f32();
    println!(
        "captured {frame} frames over {:.1}s ({fps:.1} fps)",
        start.elapsed().as_secs_f32()
    );
}

fn gallery() -> Option<Window> {
    Window::all().ok()?.into_iter().find(|window| {
        window
            .title()
            .map(|title| title.to_lowercase().contains("rift ui gallery"))
            .unwrap_or(false)
    })
}
