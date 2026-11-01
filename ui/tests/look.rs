use egui::{Context, Pos2, Rect, Vec2};
use strum::VariantArray;
use ui::card::{self, Card};
use ui::face::{self, Face};
use ui::glyph::{CardKind, Icon, Mark, Weight};
use ui::motion::{self, Clock};
use ui::sheet::{Depth, Sheet};
use ui::{controls, ink};

fn context() -> Context {
    let ctx = Context::default();
    ui::apply(&ctx);
    ctx
}

fn pass(ctx: &Context, add: impl FnOnce(&mut egui::Ui)) {
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 720.0))),
        ..egui::RawInput::default()
    };
    let mut add = Some(add);
    let mut output = ctx.run_ui(input, |ui| {
        if let Some(add) = add.take() {
            add(ui);
        }
    });
    output.textures_delta.clear();
}

#[test]
fn every_face_lays_out_text_in_its_own_font() {
    let ctx = context();
    pass(&ctx, |_| {});
    for &face in Face::VARIANTS {
        let width = ctx.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap("Rusty Sword".to_owned(), face.id(15.0), ink::INK)
                .size()
                .x
        });
        assert!(width > 10.0, "{face:?} lays out nothing");
    }
}

#[test]
fn every_mark_is_an_icon_of_the_icon_font_in_every_weight() {
    let ctx = context();
    pass(&ctx, |_| {});
    for &mark in Mark::VARIANTS {
        for &weight in Weight::VARIANTS {
            let icon = Icon {
                weight,
                ..mark.icon()
            };
            let font = egui::FontId::new(16.0, weight.family());
            let glyph = icon.glyph.chars().next().expect("an icon is one glyph");
            assert!(
                ctx.fonts_mut(|fonts| fonts.has_glyph(&font, glyph)),
                "{mark:?} has no {weight:?} icon"
            );
        }
    }
}

#[test]
fn roman_numerals() {
    assert_eq!(face::roman(0), "0");
    assert_eq!(face::roman(1), "I");
    assert_eq!(face::roman(4), "IV");
    assert_eq!(face::roman(9), "IX");
    assert_eq!(face::roman(12), "XII");
    assert_eq!(face::roman(3999), "MMMCMXCIX");
    assert_eq!(face::roman(4000), "4000");
}

#[test]
fn every_kind_goes_by_a_word_of_its_own() {
    let words: std::collections::HashSet<&str> =
        CardKind::VARIANTS.iter().map(|kind| kind.word()).collect();
    assert_eq!(words.len(), CardKind::VARIANTS.len());
}

#[test]
fn every_kind_glyph_decodes_into_a_texture() {
    let ctx = context();
    pass(&ctx, |_| {});
    for &kind in CardKind::VARIANTS {
        let texture = kind.texture(&ctx);
        let size = ctx.tex_manager().read().meta(texture).map(|meta| meta.size);
        assert!(
            size.is_some_and(|[w, h]| w > 0 && h > 0),
            "{kind:?} has no picture"
        );
    }
}

#[test]
fn a_sheet_opens_in_its_time_closes_quicker_and_reverses_from_where_it_is() {
    let mut clock = Clock::CLOSED;
    assert!(clock.is_closed());
    clock.run(true, motion::SHEET_OPEN_SECS);
    assert!(clock.is_open());
    let mut opening = Clock::CLOSED;
    opening.run(true, motion::SHEET_CLOSE_SECS);
    assert!(
        !opening.is_open(),
        "a sheet opens no quicker than it closes"
    );
    clock.run(false, motion::SHEET_CLOSE_SECS / 2.0);
    let halfway = clock.progress();
    assert!(halfway > 0.0 && halfway < 1.0);
    clock.run(true, motion::SHEET_OPEN_SECS / 4.0);
    assert!(clock.progress() > halfway && !clock.is_open());
    clock.run(false, motion::SHEET_CLOSE_SECS);
    assert!(clock.is_closed());
}

#[test]
fn a_sheet_rests_flat_once_open_and_starts_faint_and_lifted() {
    let closed = Clock::CLOSED.pose();
    let open = Clock::OPEN.pose();
    assert_eq!(open, motion::SheetPose::REST);
    assert_eq!(closed.opacity, 0.0);
    assert!(closed.dy < 0.0);
    let mut mid = Clock::CLOSED;
    mid.run(true, motion::SHEET_OPEN_SECS / 2.0);
    let pose = mid.pose();
    assert!(pose.opacity > closed.opacity && pose.opacity < open.opacity);
    assert!(pose.dy > closed.dy && pose.dy < open.dy);
}

#[test]
fn a_card_is_dealt_small_and_unseen_from_its_slot_and_lands_straight_at_its_place() {
    let from = Rect::from_min_size(Pos2::new(100.0, 500.0), Vec2::splat(36.0));
    let to = Rect::from_min_size(Pos2::new(900.0, 96.0), Vec2::new(300.0, 380.0));
    let start = motion::deal(0.0, from, to);
    assert_eq!(start.center, from.center());
    assert!(start.scale < 1.0);
    assert_eq!(start.opacity, 0.0);
    let mid = motion::deal(0.5, from, to);
    assert!(mid.center.x > from.center().x && mid.center.x < to.center().x);
    let landed = motion::deal(1.0, from, to);
    assert_eq!(landed.center, to.center());
    assert!((landed.scale - 1.0).abs() < 1e-5);
    assert_eq!(landed.angle, 0.0);
    assert_eq!(landed.opacity, 1.0);
}

#[test]
fn a_flicked_card_lifts_away_unseen_and_a_replaced_one_drops_and_dims() {
    let at = Rect::from_min_size(Pos2::new(900.0, 96.0), Vec2::new(300.0, 380.0));
    assert_eq!(motion::flick(0.0, at).opacity, 1.0);
    let gone = motion::flick(1.0, at);
    assert!(gone.center.y < at.center().y);
    assert_eq!(gone.opacity, 0.0);
    let under = motion::replaced(1.0, at);
    assert!(under.center.y > at.center().y);
    assert!(under.opacity > 0.0 && under.opacity < 1.0);
}

#[test]
fn a_posed_layer_comes_back_as_meshes_moved_into_the_pose() {
    let ctx = context();
    let layer = egui::LayerId::new(egui::Order::Middle, egui::Id::new("card in motion"));
    let rest = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(300.0, 380.0));
    let mut meshes = Vec::new();
    pass(&ctx, |ui| {
        ui.ctx()
            .layer_painter(layer)
            .rect_filled(rest, 0.0, ink::CREAM);
        let pose = motion::CardPose {
            center: rest.center() + Vec2::new(200.0, 0.0),
            scale: 1.0,
            angle: 0.0,
            opacity: 0.5,
        };
        meshes = motion::posed(ui.ctx(), layer, rest, pose);
    });
    assert!(!meshes.is_empty());
    let moved = meshes[0].calc_bounds();
    assert!((moved.center().x - (rest.center().x + 200.0)).abs() < 1.0);
    assert!(meshes[0].vertices[0].color.a() < 200);
}

#[test]
fn a_card_keeps_its_width_and_grows_with_what_it_holds_up_to_its_most() {
    let ctx = context();
    let mut short = None;
    let mut long = None;
    pass(&ctx, |ui| {
        short = Some(
            Card::new(CardKind::Equipment, "Rusty Sword")
                .show(ui, |ui, _| card::fact(ui, "+2 Damage"))
                .rect,
        );
        long = Some(
            Card::new(CardKind::Quest, "A long errand")
                .show(ui, |ui, _| {
                    for line in 0..40 {
                        card::fact(ui, &format!("Objective {line}"));
                    }
                })
                .rect,
        );
    });
    let (short, long) = (short.unwrap(), long.unwrap());
    assert_eq!(short.width(), card::WIDTH);
    assert!(short.height() > card::KIND_BAND + card::ART + card::NAME_BAND);
    assert!(short.height() < card::MAX_HEIGHT);
    assert!(
        (long.height() - card::MAX_HEIGHT).abs() <= 1.0,
        "{}",
        long.height()
    );
}

#[test]
fn a_card_cuts_a_long_name_and_tells_the_plate_so() {
    let ctx = context();
    let mut cut = Vec::new();
    pass(&ctx, |ui| {
        Card::new(CardKind::Resource, "Bundle").show(ui, |_, plate| cut.push(plate.name_cut));
        Card::new(
            CardKind::Resource,
            "A bundle of seventeen very long dried herbs from the far marsh",
        )
        .show(ui, |_, plate| cut.push(plate.name_cut));
    });
    assert_eq!(cut, vec![false, true]);
}

#[test]
fn a_closable_card_has_a_cross_and_a_page_card_has_none() {
    let ctx = context();
    let mut crosses = Vec::new();
    pass(&ctx, |ui| {
        let mut closable = Card::new(CardKind::Active, "Cleave");
        closable.closable = true;
        crosses.push(closable.show(ui, |_, _| {}).close.is_some());
        crosses.push(
            Card::new(CardKind::Passive, "Thick Skin")
                .show(ui, |_, _| {})
                .close
                .is_some(),
        );
    });
    assert_eq!(crosses, vec![true, false]);
}

#[test]
fn a_sheet_lays_its_parts_out_and_shrinks_its_cross_when_small() {
    let ctx = context();
    pass(&ctx, |ui| {
        let sheet = Sheet {
            title: Some("Inventory"),
            closable: true,
            curl: true,
            depth: Depth::Front,
        };
        let rect = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(400.0, 230.0));
        let parts = sheet.paint(ui.painter(), rect);
        assert!(rect.contains_rect(parts.body));
        assert!(parts.body.min.y >= parts.band.max.y);
        let cross = parts.cross.unwrap();
        assert!(rect.contains_rect(cross));
        let curl = parts.curl.unwrap();
        assert_eq!(curl.max, rect.max);
        let small = sheet.paint(
            ui.painter(),
            Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::splat(100.0)),
        );
        assert!(small.cross.unwrap().width() < cross.width());
    });
}

#[test]
fn controls_stand_a_control_tall() {
    let ctx = context();
    let mut heights = Vec::new();
    pass(&ctx, |ui| {
        heights.push(controls::button(ui, "Buy").rect.height());
        heights.push(controls::primary(ui, "Confirm").rect.height());
        heights.push(controls::danger(ui, "Abandon").rect.height());
        let mut value = 0.5;
        heights.push(controls::slider(ui, &mut value, 0.0..=1.0).rect.height());
    });
    assert!(
        heights
            .iter()
            .all(|height| *height == controls::CONTROL_HEIGHT)
    );
}

#[test]
fn a_sheet_in_motion_keeps_its_pin_where_it_falls_within_a_pixel_at_rest() {
    let ctx = context();
    let rect = Rect::from_min_size(Pos2::new(210.3, 140.6), Vec2::new(400.0, 230.0));
    let pin = Pos2::new(355.25, 148.75);
    let within = |at: Pos2| at - Pos2::new(at.x.round(), at.y.round());
    let mut clock = Clock::CLOSED;
    for _ in 0..12 {
        clock.run(true, motion::SHEET_OPEN_SECS / 12.0);
        let posed = clock.pose().transform(&ctx, rect, pin) * pin;
        let drift = within(posed) - within(pin);
        assert!(drift.length() < 1e-3, "drifted {drift:?} within its pixel");
    }
}
