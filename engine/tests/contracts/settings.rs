use engine::core::os::Profile;
use engine::core::render::video::VideoPreferences;
use engine::systems::settings;

// What a client saved when every view was a window with a launcher, and the HUD had a compass, a
// character widget and a hotbar: a shape no type reads any more.
const SAVED_BY_THE_OLD_INTERFACE: &str = r#"{
  "ui": {
    "snap": 16.0,
    "widgets": {
      "Inventory": [1200.0, 664.0],
      "Item": [320.0, 96.0],
      "Skill": [640.0, 112.0],
      "character": [16.0, 16.0],
      "compass": [560.0, 16.0],
      "hotbar": [400.0, 640.0],
      "quests": [16.0, 240.0]
    },
    "windows": {
      "Inventory": { "pos": [1080.0, 605.0], "size": [480.0, 224.0] },
      "Terminal": { "pos": [1088.0, 560.0], "size": [400.0, 230.0] }
    },
    "hotbar": {
      "shape": { "columns": 5, "rows": 2 },
      "keys": { "3": { "key": "KeyQ" }, "4": null }
    }
  },
  "text": { "letters_per_second": 42.0 },
  "sound": { "master": 0.25, "music": 0.5, "ambience": 0.5, "effects": 0.5, "voice": 0.75 },
  "screen": { "transition": "Crumble", "frame_rate": true },
  "skills": { "quick_cast": true },
  "camera": { "turn_ease": null }
}"#;

#[test]
fn settings_saved_by_an_older_interface_keep_what_still_applies_and_lose_what_is_gone() {
    let folder = tempfile::tempdir().expect("a folder for the profile");
    let profile = Profile(folder.path().to_path_buf());
    profile.save_settings(SAVED_BY_THE_OLD_INTERFACE);
    settings::save_video(&profile, VideoPreferences::default());
    let saved = profile.settings().expect("the settings are saved");
    for kept in [
        "\"letters_per_second\": 42.0",
        "\"master\": 0.25",
        "\"voice\": 0.75",
        "\"frame_rate\": true",
        "\"quick_cast\": true",
        "\"Item\"",
        "\"Skill\"",
    ] {
        assert!(saved.contains(kept), "{kept} is kept in:\n{saved}");
    }
    for gone in [
        "\"windows\"",
        "\"hotbar\"",
        "\"Inventory\"",
        "\"Terminal\"",
        "\"character\"",
        "\"compass\"",
        "\"quests\"",
        "KeyQ",
    ] {
        assert!(!saved.contains(gone), "{gone} is gone from:\n{saved}");
    }
}
