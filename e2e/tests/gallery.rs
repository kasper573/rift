use std::cell::Cell;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::thread::sleep;
use std::time::{Duration, Instant};

use enigo::{Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use xcap::Window;

const TITLE: &str = "rift ui gallery";
const SCENES: &[&str] = &[
    "Button intents",
    "Button sizes",
    "Toggle",
    "Tabs",
    "Checkbox",
    "Switch",
    "RadioGroup",
    "Slider",
    "Progress",
    "Avatar",
    "Separator",
];

struct Gallery(Child);

impl Drop for Gallery {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

#[test]
#[ignore = "gallery: needs a display; run with `just gallery`"]
fn every_component_state_is_visible_and_eases() {
    let launched = std::env::var("RIFT_GALLERY").ok().map(|binary| {
        Gallery(
            Command::new(PathBuf::from(binary))
                .spawn()
                .expect("launch gallery"),
        )
    });

    if launched.is_none() && find_window().is_none() {
        eprintln!("no gallery binary or window present; skipping the showcase");
        return;
    }

    let _gallery = launched;
    let window = wait_for_window(Duration::from_secs(60));
    // Focus and raise it so injected input lands here and the recording shows it on top.
    window.click(&mut input(), 0.5, 0.04);
    sleep(Duration::from_millis(500));

    let mut peak = 0.0f32;

    for name in SCENES {
        println!("scene: {name}");
        let mut enigo = input();

        window.move_to(&mut enigo, 0.5, 0.12);
        sleep(Duration::from_millis(450));
        let resting = window.capture();

        let mut best = (0.0f32, 0.5f32);
        for column in 0..6 {
            let fx = 0.28 + 0.44 * (column as f32 / 5.0);
            window.move_to(&mut enigo, fx, 0.5);
            sleep(Duration::from_millis(150));
            let moved = diff_fraction(&resting, &window.capture());
            if moved > best.0 {
                best = (moved, fx);
            }
        }
        peak = peak.max(best.0);
        println!(
            "  peak hover change {:.1}% at fx={:.2}",
            best.0 * 100.0,
            best.1
        );

        window.move_to(&mut enigo, best.1, 0.5);
        sleep(Duration::from_millis(350));
        enigo.button(Button::Left, Direction::Press).expect("press");
        sleep(Duration::from_millis(400));
        enigo
            .button(Button::Left, Direction::Release)
            .expect("release");
        sleep(Duration::from_millis(250));

        window.move_to(&mut enigo, 0.5, 0.12);
        sleep(Duration::from_millis(250));
        enigo.key(Key::Space, Direction::Click).expect("advance");
        sleep(Duration::from_millis(500));
    }

    assert!(
        peak > 0.001,
        "real pointer input never moved any component's paint (peak {:.2}%); the interaction/motion \
         pipeline is not being driven",
        peak * 100.0
    );
}

fn input() -> Enigo {
    Enigo::new(&Settings {
        open_prompt_to_get_permissions: false,
        ..Settings::default()
    })
    .expect("OS input")
}

fn wait_for_window(timeout: Duration) -> Win {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(window) = find_window() {
            sleep(Duration::from_millis(500));
            return Win {
                id: window.id().expect("window id"),
                cursor: Cell::new((0, 0)),
            };
        }
        assert!(
            Instant::now() < deadline,
            "the gallery window never appeared"
        );
        sleep(Duration::from_millis(200));
    }
}

fn find_window() -> Option<Window> {
    Window::all().ok()?.into_iter().find(|window| {
        window
            .title()
            .map(|title| title.to_lowercase().contains(TITLE))
            .unwrap_or(false)
    })
}

/// Tracks pointer position so moves can be glided in small steps — a single absolute warp often fails to register as a `CursorMoved`, but a stream of small ones reliably does.
struct Win {
    id: u32,
    cursor: Cell<(i32, i32)>,
}

impl Win {
    fn window(&self) -> Window {
        Window::all()
            .expect("windows")
            .into_iter()
            .find(|window| window.id().ok() == Some(self.id))
            .expect("the window is gone")
    }

    fn point(&self, fx: f32, fy: f32) -> (i32, i32) {
        let window = self.window();
        (
            window.x().expect("x") + (window.width().expect("w") as f32 * fx) as i32,
            window.y().expect("y") + (window.height().expect("h") as f32 * fy) as i32,
        )
    }

    /// Glide in small steps: a stream of small moves registers as `CursorMoved` far more reliably than one warp.
    fn move_to(&self, enigo: &mut Enigo, fx: f32, fy: f32) {
        let (tx, ty) = self.point(fx, fy);
        let (sx, sy) = self.cursor.get();
        for step in 1..=8 {
            let f = step as f32 / 8.0;
            let x = sx + ((tx - sx) as f32 * f) as i32;
            let y = sy + ((ty - sy) as f32 * f) as i32;
            enigo.move_mouse(x, y, Coordinate::Abs).expect("move");
            sleep(Duration::from_millis(8));
        }
        self.cursor.set((tx, ty));
    }

    fn click(&self, enigo: &mut Enigo, fx: f32, fy: f32) {
        self.move_to(enigo, fx, fy);
        sleep(Duration::from_millis(120));
        enigo.button(Button::Left, Direction::Click).expect("click");
    }

    fn capture(&self) -> Frame {
        let image = self.window().capture_image().expect("capture");
        let (width, height) = (image.width(), image.height());
        Frame {
            width,
            height,
            rgba: image.into_vec(),
        }
    }
}

struct Frame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn diff_fraction(a: &Frame, b: &Frame) -> f32 {
    if a.width != b.width || a.height != b.height || a.rgba.len() != b.rgba.len() {
        return 1.0;
    }
    let changed = a
        .rgba
        .chunks_exact(4)
        .zip(b.rgba.chunks_exact(4))
        .filter(|(p, q)| {
            let d = (p[0] as i32 - q[0] as i32).abs()
                + (p[1] as i32 - q[1] as i32).abs()
                + (p[2] as i32 - q[2] as i32).abs();
            d > 24
        })
        .count();
    changed as f32 / (a.rgba.len() / 4) as f32
}
