#![allow(clippy::unwrap_used)]
use forkstr::capture::{Normalizer, without_ansi};

#[test]
fn transcript_handles_split_utf8_color_and_progress_safely() {
    let mut normalizer = Normalizer::new();
    normalizer.process(b"\x1b[31mRED\x1b[0m\nprogress 1\rprogress 2\x1b[K\n\xe2");
    normalizer.process(b"\x98\x95\n\x1b]52;c;clipboard\x07tail");
    normalizer.finish();
    let output = normalizer.take_output();
    assert!(output.windows(5).any(|s| s == b"\x1b[31m"));
    assert_eq!(without_ansi(&output), "RED\nprogress 2\n☕\ntail\n");
    assert!(!output.windows(4).any(|s| s == b"]52;"));
}

#[test]
fn long_lines_are_bounded_and_not_lost() {
    let mut normalizer = Normalizer::new();
    for _ in 0..64 {
        normalizer.process(&[b'x'; 8192]);
    }
    normalizer.finish();
    let output = normalizer.take_output();
    assert_eq!(output.iter().filter(|b| **b == b'x').count(), 64 * 8192);
    assert!(output.split(|b| *b == b'\n').all(|line| line.len() <= 8192));
}

#[test]
fn empty_and_newline_terminated_streams_gain_no_extra_lines() {
    let mut normalizer = Normalizer::new();
    normalizer.finish();
    assert!(normalizer.take_output().is_empty());
    let mut normalizer = Normalizer::new();
    normalizer.process(b"line\n");
    normalizer.finish();
    assert_eq!(normalizer.take_output(), b"line\n");
}

#[test]
fn unterminated_control_strings_do_not_accumulate_in_terminal_parsers() {
    use forkstr::capture::OutputFilter;
    let mut filter = OutputFilter::default();
    let mut normalizer = Normalizer::new();
    let start = b"before\x1b]52;c;";
    assert!(filter.filter(start).len() <= start.len());
    normalizer.process(start);
    for _ in 0..4096 {
        let bytes = [b'x'; 8192];
        assert!(filter.filter(&bytes).is_empty());
        normalizer.process(&bytes);
    }
    assert_eq!(filter.filter(b"\x1b"), b"");
    assert_eq!(filter.filter(b"\\after"), b"after");
    normalizer.process(b"\x07after\n");
    normalizer.finish();
    assert_eq!(normalizer.take_output(), b"beforeafter\n");
}
