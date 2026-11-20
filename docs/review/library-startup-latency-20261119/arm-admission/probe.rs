//! In-memory comparison of the two product outline-admission paths.
#![allow(dead_code)]

mod deadline;
#[path = "../../../../src/main_font_budget.rs"]
mod font_budget;
#[path = "../../../../src/main_glyph_budget.rs"]
mod glyph_budget;
#[path = "../../../../src/main_glyph_cff.rs"]
mod glyph_cff;

use font_budget::{CffLoopCharge, FontBudget, FontLimits, FontUsage};
use std::{hint::black_box, time::Instant};

include!(concat!(env!("OUT_DIR"), "/bundled_fonts.rs"));
include!(env!("WEREAD_PROBE_PRODUCT_LOOKUP"));

const FACES: [(&str, &[u8]); 7] = [
    ("sans-regular", SANS_REGULAR),
    ("serif-regular", SERIF_REGULAR),
    ("sans-medium", SANS_MEDIUM),
    ("sans-bold", SANS_BOLD),
    ("serif-light", SERIF_LIGHT),
    ("serif-medium", SERIF_MEDIUM),
    ("serif-bold", SERIF_BOLD),
];

#[derive(Clone, Copy)]
enum Mode {
    Full,
    Receipt,
}

fn admit(
    bytes: &[u8],
    mode: Mode,
    budget: &mut FontBudget,
) -> Result<(), font_budget::FontLimitError> {
    match mode {
        Mode::Full => glyph_budget::admit_font_glyphs(bytes, 0, budget),
        Mode::Receipt => {
            let charge = bundled_cff_charge(bytes).expect("Exact static face must have a receipt");
            glyph_budget::admit_font_glyphs_with_cff(bytes, 0, budget, Some(charge)).map(|_| ())
        }
    }
}

fn verify() {
    let mut cumulative_full = FontBudget::default();
    let mut cumulative_fast = FontBudget::default();
    let mut first_usage = None;
    for (name, bytes) in FACES {
        assert!(bundled_cff_charge(&bytes[..bytes.len() - 1]).is_none());
        assert!(bundled_cff_charge(&bytes.to_vec()).is_none());
        let charge = bundled_cff_charge(bytes).unwrap();
        let mut fresh = FontBudget::default();
        // Independently regenerate the loop charge on this ABI, then compare it
        // with the host-generated receipt; allocator layout is intentionally target-local.
        assert_eq!(
            glyph_budget::admit_font_glyphs_with_cff(bytes, 0, &mut fresh, None),
            Ok(Some(charge))
        );
        let mut fresh_fast = FontBudget::default();
        admit(bytes, Mode::Receipt, &mut fresh_fast).unwrap();
        assert_eq!(fresh.usage(), fresh_fast.usage());
        first_usage.get_or_insert(fresh.usage());
        admit(bytes, Mode::Full, &mut cumulative_full).unwrap();
        admit(bytes, Mode::Receipt, &mut cumulative_fast).unwrap();
        assert_eq!(cumulative_full.usage(), cumulative_fast.usage());
        println!(
            "face={name} loop_work={} target_state_bytes={}",
            charge.work,
            fresh.usage().glyph_state_bytes
        );
    }

    let u = first_usage.unwrap();
    let d = FontLimits::default();
    for (name, limits) in [
        (
            "work",
            FontLimits {
                glyph_work: u.glyph_work,
                ..d
            },
        ),
        (
            "visits",
            FontLimits {
                glyph_visits: u.glyph_visits - 1,
                ..d
            },
        ),
        (
            "points",
            FontLimits {
                glyph_points: u.glyph_points - 1,
                ..d
            },
        ),
        (
            "target-state",
            FontLimits {
                glyph_state_bytes: u.glyph_state_bytes - 1,
                ..d
            },
        ),
        (
            "per-glyph-memory",
            FontLimits {
                per_glyph_memory: u.glyph_memory - 1,
                ..d
            },
        ),
        (
            "per-glyph-visits",
            FontLimits {
                per_glyph_visits: 1,
                ..d
            },
        ),
        (
            "per-glyph-points",
            FontLimits {
                per_glyph_points: 4,
                ..d
            },
        ),
    ] {
        let mut full = FontBudget::new(limits);
        let mut fast = FontBudget::new(limits);
        for b in [&mut full, &mut fast] {
            b.reserve_glyph_work(1).unwrap();
        }
        let error = admit(SANS_REGULAR, Mode::Full, &mut full).unwrap_err();
        assert_eq!(admit(SANS_REGULAR, Mode::Receipt, &mut fast), Err(error));
        assert_eq!(full.usage(), fast.usage(), "{name}");
        println!("edge={name} error={error:?} same_partial_budget=true");
    }
    println!(
        "PASS pointer_bits={} aggregate={:?}",
        usize::BITS,
        cumulative_full.usage()
    );
}

fn timed(mode: Mode) -> (u128, FontUsage) {
    let mut budget = FontBudget::default();
    let start = Instant::now();
    for (_, bytes) in FACES {
        admit(black_box(bytes), mode, &mut budget).unwrap();
    }
    let elapsed = start.elapsed().as_nanos();
    (elapsed, black_box(budget.usage()))
}

fn bench() {
    for round in 0..=6 {
        let (full, fast) = if round == 0 || round % 2 == 1 {
            (timed(Mode::Full), timed(Mode::Receipt))
        } else {
            let fast = timed(Mode::Receipt);
            (timed(Mode::Full), fast)
        };
        assert_eq!(full.1, fast.1);
        println!("round={round} full_ns={} receipt_ns={}", full.0, fast.0);
    }
}

fn main() {
    assert_eq!(usize::BITS, 32, "This probe is for the 32-bit ARM target");
    let mut args = std::env::args_os().skip(1);
    match (
        args.next().as_deref().and_then(|s| s.to_str()),
        args.next(),
        args.next(),
    ) {
        (Some("--verify"), None, None) => verify(),
        (Some("--bench"), None, None) => bench(),
        (Some("--bench-guarded"), Some(pid), None) => {
            let parent = pid.to_str().and_then(|s| s.parse().ok()).unwrap_or(0);
            if let Err(e) = deadline::guard_benchmark(parent) {
                eprintln!("Cannot arm benchmark guards: {e}");
                std::process::exit(2);
            }
            bench();
        }
        _ => {
            eprintln!("Expected --verify, --bench or --bench-guarded PARENT_PID");
            std::process::exit(2);
        }
    }
}
