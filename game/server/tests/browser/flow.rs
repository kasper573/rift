use std::time::{Duration, Instant};

use crate::cdp::Page;

pub fn wait(seconds: f32, mut ready: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs_f32(seconds);
    while Instant::now() < deadline {
        if ready() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    ready()
}

pub fn wait_for(page: &Page, condition: &str) {
    assert!(
        wait(30.0, || page.try_eval(condition) == serde_json::json!(true)),
        "page never reached: {condition}"
    );
}

pub fn click_text(page: &Page, selector: &str, text: &str) {
    page.eval(&format!(
        "[...document.querySelectorAll('{selector}')].find(el => el.textContent.includes('{text}')).click()"
    ));
}

pub fn fill(page: &Page, id: &str, value: &str) {
    page.eval(&format!(
        "(el => {{ el.value = '{value}'; el.dispatchEvent(new Event('input', {{bubbles: true}})); el.dispatchEvent(new Event('change', {{bubbles: true}})); }})(document.getElementById('{id}'))"
    ));
}

pub fn canvas_rect(page: &Page) -> (f64, f64, f64, f64) {
    let rect = page.eval(
        "(() => { const r = document.getElementById('glcanvas').getBoundingClientRect(); \
         return [r.x, r.y, r.width, r.height]; })()",
    );
    let value = |i: usize| rect[i].as_f64().unwrap_or(0.0);
    (value(0), value(1), value(2), value(3))
}

pub fn canvas_click_point(page: &Page, dx: f64, dy: f64) -> (f64, f64) {
    let (left, top, width, height) = canvas_rect(page);
    (left + width / 2.0 + dx, top + height / 2.0 + dy)
}
