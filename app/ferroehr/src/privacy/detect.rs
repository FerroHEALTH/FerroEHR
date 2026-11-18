// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The identifier ruleset: one named rule per national personal-identifier
//! kind, plus the patterns a deployment supplies for its own local ones.
//!
//! **No openEHR spec governs any of this — our own design/extension.** openEHR
//! is deployed across many jurisdictions, so the scanner is keyed by
//! jurisdiction rather than built around any one country: a rule is a named
//! kind carrying its own shape and, where the issuing register defines one, its
//! own checksum. Every shipped checksum is transcribed from the register that
//! issues the number, cited on the rule; a kind whose current algorithm could
//! not be established from its own register is NOT shipped, because a rule that
//! guesses is worse than an absent one — it tells an operator their data was
//! scanned.
//!
//! Collisions are a property of each rule, stated on each rule. A checksum over
//! a digit run of a fixed length accepts a fraction of random runs by chance,
//! and clinical content is full of numbers, so a rule narrows itself with
//! whatever structure it actually has (two control digits, an embedded date, a
//! non-numeric token shape) and the caller skips the RM slots whose value space
//! is machine codes (the `CODE_PHRASE.code_string` slot).

use std::sync::LazyLock;

use regex::Regex;

/// A national personal-identifier kind the scanner can recognise.
///
/// Rules are addressed by [`Self::key`] in
/// [`super::config::IdentifierScanConfig::rules`]; the key is
/// `<iso-3166-1-alpha-2 lowercased>-<local name>` so no jurisdiction reads as
/// the default.
#[derive(Debug, Clone, Copy)]
pub struct IdentifierRule {
    /// The configuration key, e.g. `nl-bsn`.
    pub key: &'static str,
    /// The issuing jurisdiction, ISO 3166-1 alpha-2.
    pub jurisdiction: &'static str,
    /// The identifier's own name in its register.
    pub label: &'static str,
    /// The register or authority whose published definition this rule
    /// transcribes.
    pub source: &'static str,
    /// How often this rule accepts a value that is not one of its identifiers,
    /// and what narrows it.
    pub collision: &'static str,
    detect: fn(&str) -> bool,
}

impl IdentifierRule {
    /// Whether `text` carries a value this rule claims.
    #[must_use]
    pub fn matches(&self, text: &str) -> bool {
        (self.detect)(text)
    }
}

/// Every identifier rule this build ships, ordered by key.
///
/// Deliberately NOT shipped, each for a reason that would otherwise be
/// invisible:
///
/// - **DK CPR-nummer.** CPR-kontoret has issued numbers without the modulus-11
///   control since 2007 and states they are fully valid personal numbers
///   (<https://cpr.dk/cpr-systemet/personnumre-uden-kontrolciffer-modulus-11-kontrol>),
///   so a checksum rule would pass most recent numbers straight through while
///   reporting that Denmark was covered.
/// - **BE rijksregisternummer.** Its modulo-97 check is widely reproduced, but
///   no definition published by the Rijksregister itself could be retrieved, so
///   it is left out rather than transcribed from secondary sources.
///
/// Postcodes and medical-record numbers are not here either: their formats are
/// local facts with no single issuing register, so a deployment declares them
/// as patterns ([`CustomPattern`]) instead of the build guessing.
#[must_use]
pub fn built_in_rules() -> &'static [IdentifierRule] {
    &BUILT_IN_RULES
}

/// The registry behind [`built_in_rules`], key-sorted.
static BUILT_IN_RULES: [IdentifierRule; 8] = [
    IdentifierRule {
        key: "ch-ahvn13",
        jurisdiction: "CH",
        label: "AHV-Nummer (AHVN13)",
        source: "Structure: AHVV Art. 133 (SR 831.101), vendored at \
                     docs/law/ch/ahvv/; check digit: Bundesamt für Sozialversicherungen, \
                     Wegleitung über Versicherungsausweis und individuelles Konto (WL VA/IK) \
                     318.106.02 d, Anhang 7 B, \
                     https://sozialversicherungen.admin.ch/de/d/6938/download",
        collision: "The fixed 756 country code and the mod-10 check digit together accept \
                        one in ten thousand thirteen-digit runs by chance. The official display \
                        form 756.3047.5009.62 (WL VA/IK Rz 1201) is matched as well as the plain \
                        run, because that dotted shape is how the number appears on the insurance \
                        card and in correspondence.",
        detect: ch_ahvn13,
    },
    IdentifierRule {
        key: "ch-epd-pid",
        jurisdiction: "CH",
        label: "Patientenidentifikationsnummer (EPD-PID)",
        source: "EPDV Art. 5 Abs. 2 (SR 816.11) and EPDV-EDI Art. 1 with Anhang 1 \
                     (SR 816.111), vendored at docs/law/ch/epdv/ and docs/law/ch/epdv-edi/, \
                     https://www.fedlex.admin.ch/eli/cc/2017/205/de",
        collision: "The first eight digits are the ordinance's own literal constant \
                        76133761 (country code 76, the BAG participant 13376, the application \
                        area 1) and the mod-10 check digit adds a tenth, so one in a billion \
                        eighteen-digit runs passes by chance; the prefix alone is the detector, \
                        the check digit only rejects transcription errors.",
        detect: ch_epd_pid,
    },
    IdentifierRule {
        key: "de-kvnr",
        jurisdiction: "DE",
        label: "Krankenversichertennummer (unveränderbarer Teil)",
        source: "GKV-Spitzenverband, Richtlinie zum Aufbau und zur Vergabe einer \
                     Krankenversichertennummer nach § 290 SGB V, Version 3.4.1 (Stand \
                     21.10.2025) Rz 175-188 with Anlage 1 (Stand 02.01.2023), \
                     https://www.gkv-datenaustausch.de/kvnr/kvnr.jsp; the letter-to-digit \
                     convention (A=01 ... Z=26) is stated in prose only in the same Richtlinie's \
                     Anlage 2 and fixed for A and C by the KVNR's own worked examples",
        collision: "One in ten tokens of the shape (a capital letter immediately followed \
                        by nine digits) passes by chance; the register's exclusions (no digit \
                        more than three times in a row, no 666) narrow that to one in 10.1 and \
                        are not claimed. The shape itself is the narrowing, as for fi-hetu: a \
                        letter glued to nine digits occurs in clinical prose almost never.",
        detect: de_kvnr,
    },
    IdentifierRule {
        key: "fi-hetu",
        jurisdiction: "FI",
        label: "henkilötunnus (personal identity code)",
        source: "Valtioneuvoston asetus väestötietojärjestelmästä 128/2010, 2 §, \
                     https://www.finlex.fi/fi/laki/ajantasa/2010/20100128",
        collision: "The token shape itself is the narrowing: six digits, a century sign, \
                        three digits and a control character. Among tokens of exactly that \
                        shape one in 31 matches by chance; free clinical text produces the \
                        shape almost never.",
        detect: fi_hetu,
    },
    IdentifierRule {
        key: "gb-nhs-number",
        jurisdiction: "GB",
        label: "NHS Number",
        source: "NHS Data Model and Dictionary, \
                     https://www.datadictionary.nhs.uk/attributes/nhs_number.html",
        collision: "One in eleven delimited ten-digit runs passes by chance, less the \
                        tenth of candidates the algorithm declares invalid outright. A Unix \
                        epoch second is ten digits, so a raw timestamp written into free text \
                        is this rule's most likely false positive.",
        detect: gb_nhs_number,
    },
    IdentifierRule {
        key: "nl-bsn",
        jurisdiction: "NL",
        label: "burgerservicenummer (BSN)",
        source: "Rijksdienst voor Identiteitsgegevens, Logisch Ontwerp BSN, \
                     https://www.rvig.nl/logisch-ontwerp-bsn",
        collision: "One in eleven delimited nine-digit runs passes by chance. This is the \
                        loosest rule shipped, and the reason the caller skips CODE_PHRASE \
                        code strings: SNOMED CT concept identifiers are nine digits often \
                        enough to matter.",
        detect: nl_bsn,
    },
    IdentifierRule {
        key: "no-fodselsnummer",
        jurisdiction: "NO",
        label: "fødselsnummer (national identity number)",
        source: "Skatteetaten, \
                     https://skatteetaten.github.io/folkeregisteret-api-dokumentasjon/nytt-fodselsnummer-fra-2032/",
        collision: "Two independent modulus-11 control digits, so about one in 121 \
                        delimited eleven-digit runs passes by chance.",
        detect: no_fodselsnummer,
    },
    IdentifierRule {
        key: "se-personnummer",
        jurisdiction: "SE",
        label: "personnummer",
        source: "Skatteverket, Rättslig vägledning §Uppbyggnad, \
                     https://www4.skatteverket.se/rattsligvagledning/edition/2020.2/330245.html",
        collision: "The Luhn check alone accepts one in ten ten-digit runs, so the rule \
                        also requires the first six digits to read as a date. That narrowing \
                        is ours, not Skatteverket's; it accepts the coordination-number day \
                        offset so no real number is missed, and takes the measured combined \
                        rate to one in 139.",
        detect: se_personnummer,
    },
];

/// The rule with this configuration key, if this build ships one.
#[must_use]
pub fn rule(key: &str) -> Option<&'static IdentifierRule> {
    built_in_rules()
        .iter()
        .find(|candidate| candidate.key == key)
}

/// A pattern a deployment adds for an identifier its build ships no rule for —
/// a local medical-record number, a national postcode, a payer reference.
///
/// The source spelling is kept because a caller cannot act on "some pattern
/// matched".
#[derive(Debug, Clone)]
pub struct CustomPattern {
    source: String,
    regex: Regex,
}

impl CustomPattern {
    /// Compile one configured pattern.
    ///
    /// # Errors
    /// [`regex::Error`] when the pattern does not compile; the boot path turns
    /// that into a configuration error naming the pattern.
    pub fn new(source: &str) -> Result<Self, regex::Error> {
        Ok(Self {
            source: source.to_owned(),
            regex: Regex::new(source)?,
        })
    }

    /// The pattern as the operator wrote it.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Whether `text` carries a value this pattern claims.
    #[must_use]
    pub fn is_match(&self, text: &str) -> bool {
        self.regex.is_match(text)
    }
}

// ── shared scanning ───────────────────────────────────────────────────────────

/// Calls `accept` on every maximal digit run of exactly `len` digits that is
/// delimited by something other than a letter, a digit or an underscore, until
/// one returns `true`.
///
/// The delimiter rule is what keeps the checksums off hex digests, base64
/// blobs and longer numeric identifiers: a checksum over a fixed-length run
/// accepts a fixed fraction of random runs, so without a boundary every
/// document produces hits at that rate. It also means a UUID can never match
/// any rule here — its hyphen-separated groups are 8, 4, 4, 4 and 12
/// characters, so no delimited run of 9, 10 or 11 digits occurs in one.
fn any_delimited_run(text: &str, len: usize, accept: impl Fn(&str) -> bool) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
            continue;
        }
        let start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        let end = index;
        let delimited = !start
            .checked_sub(1)
            .is_some_and(|before| is_word_byte(bytes.get(before)))
            && !is_word_byte(bytes.get(end));
        if delimited
            && end.saturating_sub(start) == len
            && let Some(run) = text.get(start..end)
            && run.bytes().any(|b| b != b'0')
            && accept(run)
        {
            return true;
        }
    }
    false
}

/// Whether `text` holds at least `count` ASCII digits anywhere.
///
/// The token patterns each demand a fixed number of `[0-9]`, so a text with
/// fewer cannot match one, and most strings of a record (names, codes, node
/// ids) are told apart by this one pass with no pattern run over them.
fn holds_digits(text: &str, count: usize) -> bool {
    text.bytes()
        .filter(u8::is_ascii_digit)
        .nth(count.saturating_sub(1))
        .is_some()
}

/// Whether a byte continues a word. `None` (past the end) is a delimiter.
fn is_word_byte(byte: Option<&u8>) -> bool {
    byte.is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
}

/// The decimal value of one ASCII digit.
fn digit(run: &str, index: usize) -> i64 {
    run.as_bytes()
        .get(index)
        .map_or(0, |b| i64::from(b.saturating_sub(b'0')))
}

/// Whether `yy`, `mm`, `dd` read as a calendar date, accepting a day offset a
/// register adds to mark a non-resident or coordination number.
///
/// Deliberately loose on the year (every two-digit value is a possible birth
/// year) and on month length (a 31st of February costs nothing to accept and
/// missing a real identifier costs everything).
fn plausible_date(month: i64, day: i64, day_offsets: &[i64]) -> bool {
    (1..=12).contains(&month)
        && day_offsets
            .iter()
            .any(|offset| (1 + offset..=31 + offset).contains(&day))
}

// ── the rules ─────────────────────────────────────────────────────────────────

/// NL — the elfproef the Logisch Ontwerp BSN defines: for the digits
/// `s0`…`s8`, `9·s0 + 8·s1 + 7·s2 + 6·s3 + 5·s4 + 4·s5 + 3·s6 + 2·s7 − s8` is
/// divisible by eleven.
fn nl_bsn(text: &str) -> bool {
    any_delimited_run(text, 9, |run| {
        let weighted: i64 = (0..8)
            .map(|i| digit(run, i) * (9 - i64::from(u8::try_from(i).unwrap_or(0))))
            .sum();
        (weighted - digit(run, 8)) % 11 == 0
    })
}

/// NO — the two modulus-11 control digits Skatteetaten publishes: `k1` over the
/// first nine digits with weights 3, 7, 6, 1, 8, 9, 4, 5, 2 and `k2` over the
/// first ten with weights 5, 4, 3, 2, 7, 6, 5, 4, 3, 2, each
/// `11 − (sum mod 11)`, with 11 written as 0 and 10 rejecting the number.
fn no_fodselsnummer(text: &str) -> bool {
    const W1: [i64; 9] = [3, 7, 6, 1, 8, 9, 4, 5, 2];
    const W2: [i64; 10] = [5, 4, 3, 2, 7, 6, 5, 4, 3, 2];
    any_delimited_run(text, 11, |run| {
        let control = |weights: &[i64]| -> Option<i64> {
            let sum: i64 = weights
                .iter()
                .enumerate()
                .map(|(i, w)| w * digit(run, i))
                .sum();
            match 11 - (sum % 11) {
                11 => Some(0),
                10 => None,
                other => Some(other),
            }
        };
        control(&W1) == Some(digit(run, 9)) && control(&W2) == Some(digit(run, 10))
    })
}

/// SE — the modulus-10 check Skatteverket describes (the digits multiplied
/// alternately by 2 and 1, the products' own digits summed, a remainder of ten
/// written as zero), plus our own narrowing that the first six digits read as
/// `YYMMDD`. The day accepts the +60 offset a coordination number carries, so
/// the narrowing cannot miss a real number.
fn se_personnummer(text: &str) -> bool {
    any_delimited_run(text, 10, |run| {
        if !plausible_date(
            digit(run, 2) * 10 + digit(run, 3),
            digit(run, 4) * 10 + digit(run, 5),
            &[0, 60],
        ) {
            return false;
        }
        let sum: i64 = (0..9)
            .map(|i| {
                let doubled = digit(run, i) * if i % 2 == 0 { 2 } else { 1 };
                if doubled > 9 { doubled - 9 } else { doubled }
            })
            .sum();
        (10 - (sum % 10)) % 10 == digit(run, 9)
    })
}

/// GB — the Modulus 11 check the NHS Data Model and Dictionary makes mandatory:
/// the first nine digits weighted 10 down to 2, the sum's remainder modulo
/// eleven subtracted from eleven, a result of eleven written as zero and a
/// result of ten meaning the number is not a valid NHS Number at all.
fn gb_nhs_number(text: &str) -> bool {
    any_delimited_run(text, 10, |run| {
        let sum: i64 = (0..9)
            .map(|i| digit(run, i) * (10 - i64::from(u8::try_from(i).unwrap_or(0))))
            .sum();
        match 11 - (sum % 11) {
            11 => 0 == digit(run, 9),
            10 => false,
            check => check == digit(run, 9),
        }
    })
}

/// The Finnish personal identity code's token shape: `ddmmyy`, a century sign,
/// a three-digit individual number and a control character.
///
/// The century signs named on the cited page are `+` (1800s), `-` and `Y`
/// (1900s) and `A` (2000s); the same page says further separators can be
/// introduced, so the whole letter range each century draws from is accepted —
/// widening acceptance can only cost a false positive, while narrowing it would
/// miss real codes.
static FI_HETU_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    // A fixed literal pattern this module's own tests exercise; the only
    // alternative to a panic at first use is a silently disabled rule.
    #[expect(
        clippy::expect_used,
        reason = "a fixed literal pattern the module's tests exercise; the alternative to a \
                  panic at first use is a silently disabled privacy rule"
    )]
    Regex::new(r"(?i)(^|[^0-9A-Za-z_])([0-9]{6})([+\-A-FU-Y])([0-9]{3})([0-9A-Y])([^0-9A-Za-z_]|$)")
        .expect("the Finnish personal-identity-code pattern should compile")
});

/// The control characters, indexed by the remainder modulo 31 — the table of
/// Valtioneuvoston asetus väestötietojärjestelmästä 128/2010, 2 §.
const FI_HETU_CONTROL: &[u8; 31] = b"0123456789ABCDEFHJKLMNPRSTUVWXY";

/// The digits every match of [`FI_HETU_TOKEN`] holds: the date's six and the
/// individual number's three.
const FI_HETU_DIGITS: usize = 9;

/// FI — the control character is the nine digits of the date and individual
/// number, read as one number, divided by 31, with the remainder indexed into
/// [`FI_HETU_CONTROL`].
fn fi_hetu(text: &str) -> bool {
    holds_digits(text, FI_HETU_DIGITS)
        && FI_HETU_TOKEN.captures_iter(text).any(|captured| {
            let Some((date, individual, control)) = captured
                .get(2)
                .zip(captured.get(4))
                .zip(captured.get(5))
                .map(|((date, individual), control)| (date, individual, control))
            else {
                return false;
            };
            let Ok(value) = format!("{}{}", date.as_str(), individual.as_str()).parse::<u32>()
            else {
                return false;
            };
            let expected = FI_HETU_CONTROL
                .get(usize::try_from(value % 31).unwrap_or(usize::MAX))
                .copied();
            expected.is_some_and(|expected| {
                control
                    .as_str()
                    .bytes()
                    .next()
                    .is_some_and(|found| found.eq_ignore_ascii_case(&expected))
            })
        })
}

/// The GS1 modulo-10 check the Swiss registers use for both the AHVN13 and the
/// EPD patient identification number: the digits before the last are weighted
/// from the right alternately 3 and 1, the products are added whole, and the
/// check digit completes the sum to the next multiple of ten (a multiple of
/// ten gives 0) — WL VA/IK 318.106.02 d Anhang 7 B and EPDV-EDI Anhang 1 Ziff. 2.
fn gs1_check_digit_holds(run: &str) -> bool {
    let bytes = run.as_bytes();
    let Some((&last, body)) = bytes.split_last() else {
        return false;
    };
    if !bytes.iter().all(u8::is_ascii_digit) {
        return false;
    }
    let sum: u32 = body
        .iter()
        .rev()
        .enumerate()
        .map(|(i, b)| u32::from(b - b'0') * if i % 2 == 0 { 3 } else { 1 })
        .sum();
    let expected = (10 - sum % 10) % 10;
    u32::from(last - b'0') == expected
}

/// The official display form of the AHV number, `756.3047.5009.62`
/// (WL VA/IK Rz 1201): the plain thirteen-digit run split 3.4.4.2.
static CH_AHVN13_DOTTED: LazyLock<Regex> = LazyLock::new(|| {
    #[expect(
        clippy::expect_used,
        reason = "a fixed literal pattern the module's tests exercise; the alternative to a \
                  panic at first use is a silently disabled privacy rule"
    )]
    Regex::new(r"(^|[^0-9A-Za-z_.])(756\.[0-9]{4}\.[0-9]{4}\.[0-9]{2})([^0-9A-Za-z_.]|$)")
        .expect("the AHV-number display pattern should compile")
});

/// The digits every match of [`CH_AHVN13_DOTTED`] holds.
const CH_AHVN13_DIGITS: usize = 13;

/// CH — the AHVN13: the country code 756, nine digits and a GS1 modulo-10
/// check digit (AHVV Art. 133; WL VA/IK Anhang 7 B), matched as a plain
/// thirteen-digit run and in the dotted display form.
fn ch_ahvn13(text: &str) -> bool {
    any_delimited_run(text, 13, |run| {
        run.starts_with("756") && gs1_check_digit_holds(run)
    }) || holds_digits(text, CH_AHVN13_DIGITS)
        && CH_AHVN13_DOTTED.captures_iter(text).any(|captured| {
            captured.get(2).is_some_and(|dotted| {
                let run: String = dotted
                    .as_str()
                    .chars()
                    .filter(char::is_ascii_digit)
                    .collect();
                gs1_check_digit_holds(&run)
            })
        })
}

/// The first eight digits of every EPD patient identification number, fixed
/// by EPDV-EDI Anhang 1 Ziff. 1 (its `Wert` row): the country code 76, the
/// registered participant «Bundesamt für Gesundheit» 13376, the application
/// area «Elektronisches Patientendossier» 1.
const CH_EPD_PID_PREFIX: &str = "76133761";

/// CH — the EPD patient identification number: the fixed prefix, nine
/// identification digits and a GS1 modulo-10 check digit (EPDV-EDI Anhang 1).
fn ch_epd_pid(text: &str) -> bool {
    any_delimited_run(text, 18, |run| {
        run.starts_with(CH_EPD_PID_PREFIX) && gs1_check_digit_holds(run)
    })
}

/// The unchangeable part of a Krankenversichertennummer: a capital letter,
/// eight digits and a check digit, glued together (Richtlinie 3.4.1 Rz 175-179).
static DE_KVNR_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    #[expect(
        clippy::expect_used,
        reason = "a fixed literal pattern the module's tests exercise; the alternative to a \
                  panic at first use is a silently disabled privacy rule"
    )]
    Regex::new(r"(^|[^0-9A-Za-z_])([A-Z])([0-9]{8})([0-9])([^0-9A-Za-z_]|$)")
        .expect("the Krankenversichertennummer pattern should compile")
});

/// The digits every match of [`DE_KVNR_TOKEN`] holds: eight and the check digit.
const DE_KVNR_DIGITS: usize = 9;

/// DE — the check digit of the unchangeable part: the letter becomes its
/// two-digit alphabet position, the ten digits are weighted from the left
/// alternately 1 and 2, each product is reduced to its cross-sum, and the check
/// digit is the units digit of the total (Richtlinie 3.4.1 Rz 185-188, the
/// worked tables of Anlage 1).
fn de_kvnr(text: &str) -> bool {
    holds_digits(text, DE_KVNR_DIGITS)
        && DE_KVNR_TOKEN.captures_iter(text).any(|captured| {
            let Some(((letter, digits), check)) =
                captured.get(2).zip(captured.get(3)).zip(captured.get(4))
            else {
                return false;
            };
            let Some(position) = letter
                .as_str()
                .bytes()
                .next()
                .map(|b| u32::from(b - b'A') + 1)
            else {
                return false;
            };
            let ten: Vec<u32> = [position.div_euclid(10), position % 10]
                .into_iter()
                .chain(digits.as_str().bytes().map(|b| u32::from(b - b'0')))
                .collect();
            let total: u32 = ten
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    let product = d * if i % 2 == 0 { 1 } else { 2 };
                    if product >= 10 { product - 9 } else { product }
                })
                .sum();
            check
                .as_str()
                .bytes()
                .next()
                .is_some_and(|b| u32::from(b - b'0') == total % 10)
        })
}

#[cfg(test)]
#[path = "detect_tests.rs"]
mod tests;
