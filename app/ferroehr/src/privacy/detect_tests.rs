// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Each rule's checksum is exercised in both directions, because a detector
//! that only ever says "no" passes a length check just as well as the real
//! arithmetic. Every positive case is paired with the same value one digit
//! changed, which is the mutation that proves the checksum runs.
//!
//! Every number here is synthetic: each was constructed by running the
//! published algorithm forward over a chosen prefix. None is issued to
//! anyone.

use super::{CustomPattern, IdentifierRule, built_in_rules, rule};

/// The rule under test, by key.
fn detector(key: &str) -> &'static IdentifierRule {
    rule(key).unwrap_or_else(|| panic!("this build ships no `{key}` rule"))
}

/// Asserts a rule accepts `valid` and refuses `mutated`.
fn accepts_and_refuses(key: &str, valid: &str, mutated: &str) {
    let rule = detector(key);
    assert!(rule.matches(valid), "{key} should accept {valid}");
    assert!(
        !rule.matches(mutated),
        "{key} should refuse {mutated}: the checksum is not being evaluated"
    );
}

#[test]
fn the_registry_is_ordered_by_key_and_every_rule_names_its_source() {
    let keys: Vec<&str> = built_in_rules().iter().map(|r| r.key).collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(
        keys, sorted,
        "the registry is read in order in the boot log"
    );
    for rule in built_in_rules() {
        assert!(
            rule.source.contains("https://"),
            "{}: a checksum with no citable source is not shipped",
            rule.key
        );
        assert!(
            !rule.collision.is_empty(),
            "{}: state the collision",
            rule.key
        );
        assert_eq!(
            rule.jurisdiction.len(),
            2,
            "{}: the jurisdiction is ISO 3166-1 alpha-2",
            rule.key
        );
        assert!(
            rule.key
                .starts_with(&rule.jurisdiction.to_ascii_lowercase()),
            "{}: the key is keyed by jurisdiction",
            rule.key
        );
    }
}

#[test]
fn no_jurisdiction_is_the_default() {
    // The registry is a set of peers. A build shipping exactly one rule
    // would make that jurisdiction the implicit norm, which is the shape
    // this ruleset exists to avoid.
    assert!(
        built_in_rules().len() >= 2,
        "a single-rule registry reads as a default jurisdiction"
    );
    let jurisdictions: std::collections::BTreeSet<&str> =
        built_in_rules().iter().map(|r| r.jurisdiction).collect();
    assert!(jurisdictions.len() >= 2, "{jurisdictions:?}");
}

#[test]
fn an_unknown_rule_key_resolves_to_nothing() {
    assert!(rule("xx-invented").is_none());
}

#[test]
fn nl_bsn_runs_the_elfproef() {
    // privacy-allow: synthetic, constructed to satisfy the published arithmetic
    accepts_and_refuses("nl-bsn", "111222333", "111222334"); // privacy-allow: synthetic
    let rule = detector("nl-bsn");
    // privacy-allow: the same digits, undelimited and over-length
    for miss in ["1112223334", "a111222333b", "field_111222333", "11122233"] {
        assert!(
            !rule.matches(miss),
            "{miss} is not a delimited nine-digit run"
        );
    }
    assert!(
        !rule.matches("000000000"),
        "no register issues an all-zero number"
    );
}

#[test]
fn no_fodselsnummer_checks_both_control_digits() {
    accepts_and_refuses("no-fodselsnummer", "15038545660", "15038545661");
    let rule = detector("no-fodselsnummer");
    assert!(rule.matches("fnr 29127600196 recorded"));
    // The first control digit right and the second wrong: a one-digit
    // checker would accept this.
    assert!(!rule.matches("15038545661"));
}

#[test]
fn se_personnummer_checks_luhn_and_the_embedded_date() {
    accepts_and_refuses("se-personnummer", "9001011239", "9001011238");
    let rule = detector("se-personnummer");
    // The coordination-number day offset (+60) is accepted, so the date
    // narrowing cannot miss a real number.
    assert!(rule.matches("9912310019"));
    // Luhn-valid, but month 91 is no date.
    assert!(!rule.matches("9991011232"));
}

#[test]
fn gb_nhs_number_runs_the_mandatory_modulus_11() {
    accepts_and_refuses("gb-nhs-number", "9434767016", "9434767015");
    let rule = detector("gb-nhs-number");
    assert!(rule.matches("NHS no. 4010231335"));
    // A remainder of one gives a check digit of ten, which the Data
    // Dictionary says makes the number invalid outright.
    assert!(!rule.matches("1234567890"));
}

#[test]
fn fi_hetu_reads_the_control_character_table() {
    let rule = detector("fi-hetu");
    for valid in ["010100A123D", "290277+456W"] {
        assert!(rule.matches(valid), "{valid}");
    }
    for mutated in ["010100A123E", "290277+456X"] {
        assert!(
            !rule.matches(mutated),
            "{mutated}: the control character is not checked"
        );
    }
    assert!(rule.matches("hetu 010100a123d in a lowercase note"));
    // Without the century sign it is not the shape at all.
    assert!(!rule.matches("010100123D"));
}

#[test]
fn no_rule_matches_a_uuid() {
    // A UUID's hyphen groups are 8, 4, 4, 4 and 12 characters, so it
    // carries no delimited run of 9, 10 or 11 digits — which is what lets
    // the opaque subject pseudonym the boundary mandates pass every rule.
    let uuids = [
        "018f3c2a-7b41-7c2e-9a55-6d1e4f80b2c3",
        "00000000-0000-0000-0000-000000000000",
        "11122233-3111-2223-3311-122233311122",
        "15038545-6601-5038-5456-601503854566",
    ];
    for (nth, uuid) in uuids.into_iter().enumerate() {
        for rule in built_in_rules() {
            assert!(!rule.matches(uuid), "{} matched UUID {nth}", rule.key);
        }
    }
}

#[test]
fn no_rule_matches_an_archetype_or_version_identifier() {
    for technical in [
        "openEHR-EHR-COMPOSITION.encounter.v1",
        "018f3c2a-7b41-7c2e-9a55-6d1e4f80b2c3::ferroehr.local::1",
        "at0002",
        "IDCR - Adverse Reaction List.v1",
    ] {
        for rule in built_in_rules() {
            assert!(!rule.matches(technical), "{} matched {technical}", rule.key);
        }
    }
}

#[test]
fn multibyte_text_never_panics_and_still_delimits() {
    for rule in built_in_rules() {
        // privacy-allow: synthetic values inside multibyte text
        let _ = rule.matches("patiënt 111222333 café — 15038545660 · 9001011239"); // privacy-allow: synthetic
    }
    // privacy-allow: synthetic, and the multibyte characters are delimiters
    assert!(detector("nl-bsn").matches("ø111222333ø")); // privacy-allow: synthetic
}

#[test]
fn a_custom_pattern_reports_its_own_spelling() {
    let pattern = CustomPattern::new(r"\bMRN-[0-9]{6}\b").expect("compiles");
    assert!(pattern.is_match("admitted under MRN-004221 today"));
    assert!(!pattern.is_match("admitted under MRN-42 today"));
    assert_eq!(pattern.source(), r"\bMRN-[0-9]{6}\b");
}

#[test]
fn a_custom_pattern_covers_a_local_address_form() {
    // The shape a deployment adds when its build ships no rule for it —
    // here a Dutch postcode paired with a house number, which is what
    // identifies a household. No jurisdiction gets an address rule built
    // in: address formats are local facts with no issuing register.
    let pattern = CustomPattern::new(
        r"(?m)(^|[^0-9A-Za-z])[1-9][0-9]{3} ?[A-Z]{2}[^0-9A-Za-z]{1,3}[0-9]{1,4}([^0-9A-Za-z]|$)",
    )
    .expect("compiles");
    assert!(pattern.is_match("address 1012 AB 5")); // privacy-allow: synthetic address
    assert!(!pattern.is_match("area 1012 AB")); // privacy-allow: synthetic, no house number
}

#[test]
fn an_uncompilable_pattern_is_an_error_not_a_panic() {
    assert!(CustomPattern::new("[unclosed").is_err());
}

/// A reproducible stream of pseudorandom digits, so the measurement below
/// gives the same answer on every machine and every run.
///
/// The multiplier and increment are the ones Knuth tabulates for a 64-bit
/// linear congruential generator (`The Art of Computer Programming`
/// vol. 2, §3.3.4, table 1, line 26); the high bits are the ones with a
/// full period, so the low 33 are discarded.
struct Digits(u64);

impl Digits {
    fn next_index(&mut self, modulus: u64) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        usize::try_from((self.0 >> 33) % modulus).expect("a value below 31 fits in usize")
    }

    fn run(&mut self, length: usize) -> String {
        (0..length)
            .map(|_| {
                let digit = u32::try_from(self.next_index(10)).expect("one digit fits in u32");
                char::from_digit(digit, 10).expect("a value below ten is a decimal digit")
            })
            .collect()
    }
}

/// One in how many of `samples` the rule claimed.
fn one_in(samples: usize, matched: usize) -> f64 {
    let samples = u32::try_from(samples).expect("the sample count fits in u32");
    let matched = u32::try_from(matched.max(1)).expect("the match count fits in u32");
    f64::from(samples) / f64::from(matched)
}

/// The measured false-positive rate of every rule against the rate the
/// rule itself publishes.
///
/// Each rule's `collision` text tells a deployment how often random digits
/// satisfy its arithmetic, and that number decides whether the rule is
/// safe to run over free clinical text. An unchecked claim in a doc string
/// is worth nothing, so this measures it: 100 000 random tokens of the
/// rule's own shape, and the observed rate must land within a factor of
/// two of the published one. A rule that stops discriminating — the
/// mutation that makes a checksum return `true` — matches everything and
/// fails here even though no fixture and no corpus document changes.
#[test]
fn every_rule_matches_random_tokens_at_the_rate_it_publishes() {
    const SAMPLES: usize = 100_000;
    // key, digits in the token, the published one-in-N rate.
    const PUBLISHED: &[(&str, usize, f64)] = &[
        ("gb-nhs-number", 10, 11.0),
        ("nl-bsn", 9, 11.0),
        ("no-fodselsnummer", 11, 121.0),
        ("se-personnummer", 10, 139.0),
    ];

    let mut digits = Digits(0x_5eed_1234_5eed_1234);
    for &(key, length, published) in PUBLISHED {
        let detector = detector(key);
        let matched = (0..SAMPLES)
            .filter(|_| detector.matches(&digits.run(length)))
            .count();
        let measured = one_in(SAMPLES, matched);
        assert!(
            measured >= published / 2.0 && measured <= published * 2.0,
            "{key} matched one in {measured:.1} random {length}-digit tokens, and publishes \
             one in {published:.0}"
        );
    }

    // The Finnish rule reads a token shape rather than a bare digit run,
    // so its population is that shape: six digits, a century sign, three
    // digits and one character from the 31-entry control table.
    let hetu = detector("fi-hetu");
    let matched = (0..SAMPLES)
        .filter(|_| {
            let index = digits.next_index(31);
            let control = super::FI_HETU_CONTROL
                .get(index)
                .map(|byte| char::from(*byte))
                .expect("an index below 31 is in the control table");
            let token = format!("{}-{}{control}", digits.run(6), digits.run(3));
            hetu.matches(&token)
        })
        .count();
    let measured = one_in(SAMPLES, matched);
    assert!(
        (15.5..=62.0).contains(&measured),
        "fi-hetu matched one in {measured:.1} tokens of its own shape, and publishes one in 31"
    );
}

#[test]
fn ch_ahvn13_runs_the_gs1_check_in_both_forms() {
    // The Wegleitung's own example (Anhang 7 C: Zwischensumme 123 → 7) and
    // its display example (Rz 1201), then one digit changed in each.
    accepts_and_refuses("ch-ahvn13", "7561234567897", "7561234567898"); // privacy-allow: the publisher's worked example
    accepts_and_refuses("ch-ahvn13", "756.3047.5009.62", "756.3047.5009.63"); // privacy-allow: the publisher's display example
    let rule = detector("ch-ahvn13");
    // The country code is part of the rule: the same arithmetic over another
    // prefix is not an AHV number.
    assert!(!rule.matches("1234567890128"));
    // A run glued to a word is not a delimited identifier.
    assert!(!rule.matches("id7561234567897x"));
}

#[test]
fn ch_epd_pid_requires_the_ordinance_prefix_and_the_check_digit() {
    // EPDV-EDI Anhang 1 Ziff. 3, the annex's own illustration
    // (76133761123456789 → 7), then one digit changed.
    accepts_and_refuses("ch-epd-pid", "761337611234567897", "761337611234567898"); // privacy-allow: the ordinance's worked example
    let rule = detector("ch-epd-pid");
    // Eighteen digits with a valid check but another prefix: not an EPD-PID.
    assert!(!rule.matches("761337621234567894"));
}

#[test]
fn de_kvnr_reads_the_letter_as_its_alphabet_position() {
    // The Richtlinie's own worked examples (Tabelle 2 and 3), then one digit
    // changed; the C example is what fixes the letter-to-digit convention.
    accepts_and_refuses("de-kvnr", "A000500015", "A000500016"); // privacy-allow: the publisher's worked example
    accepts_and_refuses("de-kvnr", "C000500021", "C000500022"); // privacy-allow: the publisher's worked example
    let rule = detector("de-kvnr");
    // Computed forward from the algorithm over chosen prefixes.
    assert!(rule.matches("K543219876")); // privacy-allow: synthetic
    assert!(rule.matches("M123456785")); // privacy-allow: synthetic
    // Nine digits without the letter are not this identifier.
    assert!(!rule.matches("000500015"));
    // A lowercase letter is not the register's spelling.
    assert!(!rule.matches("a000500015"));
}
