use egui::{Context, Pos2, Rect, Vec2};
use strum::VariantArray;
use ui::card::{
    self, ArtLook, BorderLook, Card, CardDress, CardPane, CardTilt, CornerLook, CrestLook,
    DividerLook, HaloLook, LabelLineLook, NameBandLook, PaperLook,
};
use ui::controls::{RowHover, RowLook};
use ui::face::Face;
use ui::glyph::{CardKind, Icon, Mark, Weight};
use ui::motion::{self, SheetClock};
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
    let mut clock = SheetClock::CLOSED;
    assert!(clock.is_closed());
    clock.run(true, motion::SHEET_OPEN_SECS);
    assert!(clock.is_open());
    let mut opening = SheetClock::CLOSED;
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
    let closed = SheetClock::CLOSED.pose();
    let open = SheetClock::OPEN.pose();
    assert_eq!(open, motion::SheetPose::REST);
    assert_eq!(closed.opacity, 0.0);
    assert!(closed.dy < 0.0);
    let mut mid = SheetClock::CLOSED;
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
fn what_a_layer_was_painted_since_a_mark_is_lifted_off_it_as_the_triangles_it_would_draw() {
    let ctx = context();
    let layer = egui::LayerId::new(egui::Order::Middle, egui::Id::new("card on its layer"));
    let under = Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::new(60.0, 60.0));
    let card = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(300.0, 380.0));
    let mut lifted = Vec::new();
    let mut left = 0;
    pass(&ctx, |ui| {
        let painter = ui.ctx().layer_painter(layer);
        painter.rect_filled(under, 0.0, ink::CREAM);
        let mark = motion::mark(ui.ctx(), layer);
        painter.rect_filled(card, 0.0, ink::CREAM);
        lifted = motion::lifted(ui.ctx(), layer, mark);
        left = ui
            .ctx()
            .graphics_mut(|graphics| graphics.entry(layer).all_entries().count());
    });
    assert_eq!(left, 1, "what was painted before the mark stays");
    assert_eq!(lifted.len(), 1);
    let taken = lifted[0].mesh.calc_bounds();
    assert!(
        (taken.min - card.min).length() < 2.0 && (taken.max - card.max).length() < 2.0,
        "{taken:?}"
    );
}

#[test]
fn every_card_is_one_size_whatever_it_holds_and_however_it_is_dressed() {
    let ctx = context();
    let crest = 18.0;
    let dress = CardDress {
        label: Some("Epic"),
        border: BorderLook::Double {
            width: 3.0,
            gap: 2.0,
        },
        corners: CornerLook::Notched { size: 4.0 },
        crest: CrestLook::Icon { size: crest },
        name_band: NameBandLook::Ribbon {
            overhang: 6.0,
            icon: true,
        },
        label_line: LabelLineLook::Strip,
        art: ArtLook::Inset { margin: 6.0 },
        paper: PaperLook::Tinted { strength: 0.25 },
        dividers: DividerLook::Gradient,
        halo: HaloLook::Glow { spread: 12.0 },
        ..CardDress::PLAIN
    };
    let mut shown = Vec::new();
    pass(&ctx, |ui| {
        for (name, lines, dress) in [
            ("Rusty Sword", 1, CardDress::PLAIN),
            ("A long errand", 40, CardDress::PLAIN),
            ("Moon Charm", 1, dress),
        ] {
            let mut card = Card::new(CardKind::Equipment, "Equipment", name);
            card.dress = dress;
            let response = card.show(ui, |ui, plate| {
                card::heading(ui, plate, "grants while worn");
                for line in 0..lines {
                    card::fact(ui, &format!("Line {line}"));
                }
            });
            shown.push((response.rect, response.frame));
        }
    });
    let [
        (short, short_frame),
        (long, long_frame),
        (dressed, dressed_frame),
    ] = shown[..]
    else {
        panic!("three cards");
    };
    let size = Vec2::new(card::WIDTH, card::HEIGHT);
    assert_eq!(short.size(), size);
    assert_eq!(long.size(), size);
    assert_eq!(short_frame.size(), long_frame.size());
    assert_eq!(dressed_frame.size(), short_frame.size());
    assert_eq!(dressed.width(), card::WIDTH);
    assert!(
        dressed.height() - card::HEIGHT >= crest,
        "a crest rises over the card: {}",
        dressed.height()
    );
    assert!(dressed.contains_rect(dressed_frame));
}

#[test]
fn a_card_holding_more_than_it_has_room_for_scrolls_it_by_a_bar_inside_its_frame() {
    let ctx = context();
    let mut shown = Vec::new();
    // A card learns how much it holds as it writes it, so its bar stands from the next pass on.
    for _ in 0..2 {
        shown.clear();
        pass(&ctx, |ui| {
            for (name, lines) in [("Rusty Sword", 1), ("A long errand", 40)] {
                let mut room = Rect::NOTHING;
                let response =
                    Card::new(CardKind::Equipment, "Equipment", name).show(ui, |ui, _| {
                        room = ui.max_rect();
                        for line in 0..lines {
                            card::fact(ui, &format!("Line {line}"));
                        }
                    });
                shown.push((room, response.frame));
            }
        });
    }
    let [(short, short_frame), (long, long_frame)] = shown[..] else {
        panic!("two cards");
    };
    let beside = |room: Rect, frame: Rect| frame.right() - room.right();
    assert!(
        beside(long, long_frame) > beside(short, short_frame),
        "what overruns the card leaves its bar a lane inside the frame: {long:?} in {long_frame:?}"
    );
    assert_eq!(
        long.left() - long_frame.left(),
        short.left() - short_frame.left()
    );
}

#[test]
fn a_card_turns_toward_the_pointer_by_degrees_and_lies_flat_again_once_it_leaves() {
    let flat = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(300.0, 400.0));
    let (most, settles_in, frame) = (4.0, 0.25, 1.0 / 60.0);
    let settled = |from: CardTilt, pointer: Option<Pos2>| {
        (0..60).fold(from, |tilt, _| {
            tilt.toward(pointer, flat, most, settles_in, frame)
        })
    };
    let near = |tilt: CardTilt, aside: f32, above: f32| {
        (tilt.aside - aside).abs() < 0.01 && (tilt.above - above).abs() < 0.01
    };

    let first = CardTilt::FLAT.toward(Some(flat.right_center()), flat, most, settles_in, frame);
    assert!(first.aside > 0.0 && first.aside < most / 2.0, "{first:?}");

    let right = settled(CardTilt::FLAT, Some(flat.right_center()));
    assert!(near(right, most, 0.0), "{right:?}");
    let corner = settled(right, Some(flat.left_top()));
    assert!(near(corner, -most, -most), "{corner:?}");
    let middle = settled(corner, Some(flat.center()));
    assert!(near(middle, 0.0, 0.0), "{middle:?}");
    for gone in [None, Some(flat.right_center() + Vec2::new(40.0, 0.0))] {
        let left = settled(right, gone);
        assert!(near(left, 0.0, 0.0), "{left:?}");
    }
}

#[test]
fn a_card_pane_takes_the_same_room_empty_and_full() {
    let ctx = context();
    let pane = CardPane {
        size: Vec2::new(320.0, 300.0),
        foot: controls::CONTROL_HEIGHT,
    };
    let mut rooms = Vec::new();
    pass(&ctx, |ui| {
        rooms.push(pane.show(ui, |_| {}, |_| {}).rect.size());
        let full = pane.show(
            ui,
            |ui| {
                Card::new(CardKind::Equipment, "Equipment", "Rusty Sword").show(ui, |ui, _| {
                    for _ in 0..40 {
                        card::fact(ui, "A long line of facts about the sword.");
                    }
                });
            },
            |ui| {
                controls::primary(ui, "Sell 2 for 4 Gold");
                controls::button(ui, "All 4");
            },
        );
        rooms.push(full.rect.size());
    });
    assert_eq!(rooms, vec![pane.size, pane.size]);
}

#[test]
fn a_card_cuts_a_long_name_and_tells_the_plate_so() {
    let ctx = context();
    let mut cut = Vec::new();
    pass(&ctx, |ui| {
        Card::new(CardKind::Resource, "Resource", "Bundle")
            .show(ui, |_, plate| cut.push(plate.name_cut));
        Card::new(
            CardKind::Resource,
            "Resource",
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
        let mut closable = Card::new(CardKind::Active, "Active skill", "Cleave");
        closable.close = Some("Close");
        crosses.push(closable.show(ui, |_, _| {}).close.is_some());
        crosses.push(
            Card::new(CardKind::Passive, "Passive skill", "Thick Skin")
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
fn a_row_under_the_pointer_is_lit_unless_its_list_lights_the_selection_alone() {
    let lit_rows = |hover: RowHover| {
        let ctx = context();
        let mut lit = 0;
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 720.0))),
                events: vec![egui::Event::PointerMoved(Pos2::new(100.0, 12.0))],
                ..egui::RawInput::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                for selected in [false, true] {
                    let look = RowLook {
                        selected,
                        hover,
                        ..RowLook::default()
                    };
                    controls::row(ui, look, |ui| ui.label("A choice"));
                }
            });
            output.textures_delta.clear();
            lit = washes(&output.shapes);
        }
        lit
    };
    assert_eq!(
        lit_rows(RowHover::Lit),
        2,
        "the row under the pointer is lit beside the selected one"
    );
    assert_eq!(
        lit_rows(RowHover::Unlit),
        1,
        "only the selected row is lit, whatever the pointer rests on"
    );
}

fn washes(shapes: &[egui::epaint::ClippedShape]) -> usize {
    fn count(shape: &egui::Shape) -> usize {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().map(count).sum(),
            egui::Shape::Rect(rect) => usize::from(rect.fill == ink::gold_wash(0.2)),
            _ => 0,
        }
    }
    shapes.iter().map(|clipped| count(&clipped.shape)).sum()
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
    let mut clock = SheetClock::CLOSED;
    for _ in 0..12 {
        clock.run(true, motion::SHEET_OPEN_SECS / 12.0);
        let posed = clock.pose().transform(&ctx, rect, pin) * pin;
        let drift = within(posed) - within(pin);
        assert!(drift.length() < 1e-3, "drifted {drift:?} within its pixel");
    }
}

#[test]
fn any_colour_reads_as_text_over_the_dark_and_one_that_already_does_is_left_as_it_is() {
    let dim = [
        egui::Color32::BLACK,
        egui::Color32::from_rgb(128, 128, 128),
        ink::TYRIAN,
        ink::AZURE,
        ink::RUBRIC,
    ];
    for color in dim {
        let read = ink::contrast(ink::legible(color, ink::VEIL), ink::VEIL);
        assert!(read >= ink::READABLE, "{color:?} reads at {read}");
    }
    assert_eq!(ink::legible(ink::SPRING, ink::VEIL), ink::SPRING);
}

#[test]
fn labels_laid_apart_never_overlap_and_only_those_that_would_move() {
    let label = |x: f32, y: f32| Rect::from_min_size(Pos2::new(x, y), Vec2::new(120.0, 20.0));
    let cases = [
        vec![label(100.0, 300.0), label(100.0, 300.0)],
        vec![
            label(100.0, 300.0),
            label(100.0, 300.0),
            label(100.0, 300.0),
        ],
        vec![label(100.0, 300.0), label(160.0, 310.0), label(40.0, 290.0)],
        vec![label(100.0, 300.5), label(100.3, 279.7), label(99.9, 300.2)],
        vec![label(100.0, 300.0), label(400.0, 300.0)],
    ];
    let apart = 4.0;
    for case in cases {
        let mut laid = case.clone();
        ui::float::lay_apart(&mut laid, apart);
        assert_eq!(laid[0], case[0], "the first keeps its place");
        for (i, rect) in laid.iter().enumerate() {
            assert_eq!(rect.size(), case[i].size());
            assert_eq!(rect.left(), case[i].left(), "a label only rises");
            assert!(rect.top() <= case[i].top(), "a label only rises");
            let clear = |other: &Rect| {
                rect.right() + apart <= other.left() + 1e-3
                    || other.right() + apart <= rect.left() + 1e-3
                    || rect.bottom() + apart <= other.top() + 1e-3
                    || other.bottom() + apart <= rect.top() + 1e-3
            };
            for other in &laid[..i] {
                assert!(clear(other), "{rect:?} comes near {other:?} in {case:?}");
            }
            let near_none_before = case[..i].is_empty()
                || laid[..i].iter().all(|other| {
                    let start = case[i];
                    start.right() + apart <= other.left()
                        || other.right() + apart <= start.left()
                        || start.bottom() + apart <= other.top()
                        || other.bottom() + apart <= start.top()
                });
            if near_none_before {
                assert_eq!(
                    *rect, case[i],
                    "a label clear of the others keeps its place"
                );
            }
        }
    }
}

/// A card set down in a scene stands with its centre as near the middle of the screen as it can, at
/// least its margin clear of the dialogue and of everyone talking, and wholly on the screen; where
/// no spot on the screen keeps them clear, the screen pushes it in from the clear spot it pushes
/// least.
#[test]
fn a_card_set_down_in_a_scene_stands_nearest_the_middle_clear_of_the_dialogue_and_the_people() {
    let screen = Rect::from_min_max(Pos2::ZERO, Pos2::new(1280.0, 720.0));
    let card = Vec2::new(300.0, 415.0);
    let margin = 20.0;
    let body =
        |x: f32, width: f32| Rect::from_min_max(Pos2::new(x, 230.0), Pos2::new(x + width, 720.0));
    let dialogue = |top: f32| Rect::from_min_max(Pos2::new(228.0, top), Pos2::new(1040.0, 634.0));
    let cases: [(&str, Vec<Rect>, bool); 7] = [
        ("with nothing in its way", vec![], true),
        ("with someone in the middle", vec![body(560.0, 160.0)], true),
        (
            "with a short dialogue and people on both sides of the middle",
            vec![dialogue(580.0), body(380.0, 140.0), body(760.0, 140.0)],
            true,
        ),
        (
            "with a narrow dialogue too tall to stand above",
            vec![Rect::from_min_max(
                Pos2::new(400.0, 430.0),
                Pos2::new(880.0, 634.0),
            )],
            true,
        ),
        (
            "with people across the screen above a short dialogue",
            vec![dialogue(580.0), body(0.0, 620.0), body(660.0, 620.0)],
            false,
        ),
        (
            "with the dialogue a little too tall to stand above, someone in the middle and someone \
             near at hand",
            vec![
                dialogue(428.0),
                Rect::from_min_max(Pos2::new(300.0, 150.0), Pos2::new(800.0, 720.0)),
                Rect::from_min_max(Pos2::new(880.0, 300.0), Pos2::new(1280.0, 720.0)),
            ],
            false,
        ),
        (
            "with the dialogue too tall to stand above, and people on both sides of it",
            vec![dialogue(430.0), body(0.0, 360.0), body(900.0, 380.0)],
            false,
        ),
    ];
    let middle = screen.center();
    let clear = |at: Pos2, kept_clear: &[Rect]| {
        let placed = Rect::from_min_size(at, card);
        kept_clear
            .iter()
            .all(|kept| !placed.intersects(kept.expand(margin - 1e-3)))
    };
    for (case, kept_clear, fits) in cases {
        let at = ui::float::nearest_clear_spot(card, screen, &kept_clear, margin, middle);
        let placed = Rect::from_min_size(at, card);
        assert!(
            screen.expand(1e-3).contains_rect(placed),
            "{case}, the card stands at {placed:?}, off the screen"
        );
        let free = screen.size() - card;
        let nearest = (0..=640)
            .flat_map(|i| (0..=152).map(move |j| (i, j)))
            .map(|(i, j)| Pos2::new(free.x * i as f32 / 640.0, free.y * j as f32 / 152.0))
            .filter(|spot| clear(*spot, &kept_clear))
            .map(|spot| Rect::from_min_size(spot, card).center().distance(middle))
            .fold(None, |nearest: Option<f32>, distance| {
                Some(nearest.map_or(distance, |nearest| nearest.min(distance)))
            });
        assert_eq!(
            nearest.is_some(),
            fits,
            "{case}, the screen {} a spot clear of them all",
            if fits {
                "should have"
            } else {
                "should not have"
            }
        );
        if nearest.is_none() {
            let step = 4.0;
            let reach = 600.0;
            let pushed = |spot: Pos2| {
                spot.distance(spot.clamp(Pos2::ZERO, (screen.size() - card).to_pos2()))
            };
            let clear_spots: Vec<Pos2> = (0..=((free_reach(screen, card, reach).x) / step)
                as usize)
                .flat_map(|i| {
                    (0..=((free_reach(screen, card, reach).y) / step) as usize)
                        .map(move |j| Pos2::new(i as f32 * step - reach, j as f32 * step - reach))
                })
                .filter(|spot| clear(*spot, &kept_clear))
                .collect();
            let least = clear_spots
                .iter()
                .map(|spot| pushed(*spot))
                .fold(f32::INFINITY, f32::min);
            let from_clear = clear_spots
                .iter()
                .map(|spot| spot.distance(at))
                .fold(f32::INFINITY, f32::min);
            assert!(
                from_clear <= least + step * 2.0,
                "{case}, the card stands {from_clear} from the nearest clear spot where the \
                 screen need push it only {least}"
            );
        }
        if let Some(nearest) = nearest {
            assert!(
                clear(at, &kept_clear),
                "{case}, the card stands at {placed:?}, nearer than {margin} to what it keeps clear of"
            );
            let distance = placed.center().distance(middle);
            assert!(
                distance <= nearest + 1e-3,
                "{case}, the card stands {distance} from the middle where it could stand {nearest}"
            );
        }
    }
}

/// How far the top left of a card of `size` reaches over the screen and `reach` past it each way.
fn free_reach(screen: Rect, size: Vec2, reach: f32) -> Vec2 {
    screen.size() - size + Vec2::splat(reach * 2.0)
}
