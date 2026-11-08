//! Small source-entry cases for bounded entity delimiter inspection.

use super::*;

#[test]
fn entity_delimiter_scan_has_linear_small_input_work() {
    for count in [24, 48, 96] {
        let input = format!("{}unterminated suffix", "&".repeat(count));
        let mut visits = 0;
        let output = decode_entities_with_probe(&input, || visits += 1);
        assert_eq!(output, input);
        assert!(visits <= count * 13);
        assert!(visits >= count);
    }
    let input = format!("{}{};", "&".repeat(48), "x".repeat(32));
    let mut visits = 0;
    assert_eq!(decode_entities_with_probe(&input, || visits += 1), input);
    assert_eq!(visits, 48 * 13);
}

#[test]
fn entity_delimiter_window_preserves_boundary_and_unicode() {
    let accepted = format!("&#{}65;", "0".repeat(9));
    let rejected = format!("&#{}65;", "0".repeat(10));
    assert_eq!(decode_entities(&accepted), "A");
    assert_eq!(decode_entities(&rejected), rejected);
    assert_eq!(decode_entities("&#x1F642;"), "\u{1f642}");
    assert_eq!(decode_entities("&#x110000;"), "&#x110000;");
    assert_eq!(decode_entities("&unknown; &; &#;"), "&unknown; &; &#;");

    // The thirteenth inspected byte lies inside the fourth Unicode scalar.
    // Byte inspection must not create a string slice at that boundary.
    let input = "&\u{1f980}\u{1f980}\u{1f980}\u{1f980};";
    let mut visits = 0;
    assert_eq!(decode_entities_with_probe(input, || visits += 1), input);
    assert_eq!(visits, 13);
}

#[test]
fn html_to_text_preserves_entities_and_active_content_under_bounded_scan() {
    let input = concat!(
        "<head>&amp; hidden</head><p>\u{732b} &amp; \u{72d7}&nbsp;",
        "&lt;x&gt; &#65; &#x1F642; &unknown; &#x110000; &unfinished</p>",
        "<script>ignored &&&&&&&&&&&&&&&&</script><p>After &quot;end&quot;</p>"
    );
    assert_eq!(
        html_to_text(input),
        concat!(
            "\u{732b} & \u{72d7} <x> A \u{1f642} &unknown; &#x110000; &unfinished\n",
            "After \"end\""
        )
    );
}
