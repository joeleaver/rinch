//! Review of #1687: rinch's answers for the Chrome-probe table (UTF-16 units).
const X: &[(&str, &str)] = &[
    ("vs_after_mark", "\u{65}\u{301}\u{FE0F}"),
    ("deva_zwj", "\u{915}\u{94D}\u{200D}\u{937}"),
    ("deva_conjunct", "\u{915}\u{94D}\u{937}\u{93F}"),
    ("hebrew_niqqud", "\u{5E9}\u{5B8}\u{5C1}"),
    ("arabic_harakat", "\u{628}\u{64E}\u{651}"),
    ("lone_ri", "\u{1F1EB}"),
    ("emoji_zwj_end", "\u{1F468}\u{200D}"),
    ("emoji_accent", "\u{1F600}\u{301}"),
    ("hash_keycap", "\u{23}\u{FE0F}\u{20E3}"),
    (
        "couple_tones",
        "\u{1F9D1}\u{1F3FD}\u{200D}\u{1F91D}\u{200D}\u{1F9D1}\u{1F3FB}",
    ),
    ("thai_sara_am", "\u{E01}\u{E33}"),
    ("heart_tone", "\u{2764}\u{1F3FB}"),
    ("deva_ki", "\u{915}\u{93F}"),
    ("rainbow", "\u{1F3F3}\u{FE0F}\u{200D}\u{1F308}"),
    ("copyright_vs", "\u{A9}\u{FE0F}"),
    ("e_zwj", "\u{65}\u{200D}"),
    ("eye_bubble", "\u{1F441}\u{200D}\u{1F5E8}"),
    ("health_worker", "\u{1F468}\u{200D}\u{2695}\u{FE0F}"),
    ("jamo_lv", "\u{1100}\u{1161}"),
    ("precomposed_plus_t", "\u{AC00}\u{11A8}"),
    ("cjk_ivs", "\u{845B}\u{E0100}"),
    ("tamil_ksha", "\u{B95}\u{BCD}\u{BB7}"),
    ("zwj_cjk", "\u{1F600}\u{200D}\u{4E00}"),
];
fn u16(s: &str) -> usize {
    s.encode_utf16().count()
}
#[test]
fn review_1687_table() {
    for &(k, x) in X {
        let t = format!("a{x}b");
        let before_b = t.len() - 1;
        let bs = rinch_words::backspace_start(&t, before_b);
        let after_bs: String = format!("{}{}", &t[..bs], &t[before_b..]);
        let del = rinch_words::next_grapheme_boundary(&t, 1);
        let after_del: String = format!("a{}", &t[del..]);
        let left = rinch_words::prev_grapheme_boundary(&t, before_b);
        let cps = |s: &str| {
            s.chars()
                .map(|c| format!("{:04X}", c as u32))
                .collect::<Vec<_>>()
                .join(" ")
        };
        println!(
            "RINCH {k:16} BS->{} | {} ; DEL | {} ; RIGHT 1->{} ; LEFT {}->{}",
            u16(&t[..bs]),
            cps(&after_bs),
            cps(&after_del),
            u16(&t[..del]),
            u16(&t[..before_b]),
            u16(&t[..left])
        );
    }
}
