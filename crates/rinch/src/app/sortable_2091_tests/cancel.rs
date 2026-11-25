use super::*;

fn hovering_after_c(f: &mut Fixture) -> (f32, f32) {
    start_card(f, "a");
    let at = point(f, item(f, "c"), 0.5, 0.75);
    move_to(f, at);
    assert!(last_offer(f).is_some());
    at
}

#[test]
fn escape_cancels_a_pointer_drag() {
    let mut f = fixture();
    let at = hovering_after_c(&mut f);
    send(
        &mut f,
        PlatformEvent::KeyDown {
            key: KeyCode::Escape,
            logical_key: None,
            text: None,
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Unknown,
        },
    );
    assert!(f.app.active_dnd.is_none());
    assert_eq!(OFFERS.with(|o| o.borrow().last().cloned()), Some(None));
    assert_clean(&f);
    release_at(&mut f, at);
    assert!(drops().is_empty());
}

#[test]
fn a_missed_release_heals_without_dropping() {
    let mut f = fixture();
    let at = hovering_after_c(&mut f);
    // The release went missing; the next left press proves it.
    press_at(&mut f, at);
    assert!(f.app.active_dnd.is_none());
    assert!(drops().is_empty(), "a healed drag drops nothing");
    assert_clean(&f);
    release_at(&mut f, at);
    assert!(drops().is_empty());
}
