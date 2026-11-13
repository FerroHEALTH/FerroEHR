// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access-delay register: whether, and for which categories, the EHDS
//! Art. 3(3) delay of the person's own access applies.
//!
//! Art. 3(3) is a Member State option: a Member State "may restrict" the
//! person's right of access "by delaying access … for a limited period of
//! time" (`docs/law/eu/ehds/text.html`). The register is therefore off by
//! default, and a deployment turns it on only where its Member State's rule
//! provides for the delay. No openEHR spec governs the delay or the register:
//! our own design.

use ferrohealth_config::units::Duration;
use serde::{Deserialize, Serialize};

use crate::system_log::categories::{Classifier, NONE};

/// The longest hold a deployment may configure: one year.
///
/// Art. 3(3) allows the delay only "for a limited period of time" and names no
/// figure; the ceiling keeps a typo from becoming a hold of years. No
/// specification fixes the number: our own design.
pub const MAX_HOURS: u32 = 24 * 366;

/// The access-delay register document. Off by default: Art. 3(1) immediate
/// access holds until a deployment says its Member State provides otherwise.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AccessDelayRegister {
    /// Whether the delay applies on this deployment.
    pub enabled: bool,
    /// The categories whose new compositions are held at creation, named as
    /// the audit category register names one: a priority category of Art.
    /// 14(1) by its code (`Laboratory-Reports`, `Medical-Imaging`, …) or as
    /// `<system>|<code>`, or a national category the map declares. Empty holds
    /// nothing at creation, and a professional still holds an object by hand.
    pub categories: Vec<String>,
    /// How long a hold lasts at most, a whole number of hours from `"1h"` to
    /// [`MAX_HOURS`] hours: the length of a hold placed at creation and the
    /// latest end a professional may give one. Required while enabled.
    pub max_delay: Option<Duration>,
}

impl AccessDelayRegister {
    /// The limit in whole hours, when it is one from 1 to [`MAX_HOURS`].
    #[must_use]
    pub fn hours(&self) -> Option<u32> {
        let millis = self.max_delay?.as_millis();
        if millis.checked_rem(3_600_000) != Some(0) {
            return None;
        }
        let hours = u32::try_from(millis.checked_div(3_600_000)?).ok()?;
        (1..=MAX_HOURS).contains(&hours).then_some(hours)
    }

    /// The problems a write is refused on while the register is enabled: a
    /// missing, fractional or out-of-range limit, and a category the audit
    /// category map in force does not name.
    #[must_use]
    pub fn errors(&self, classifier: &Classifier) -> Vec<String> {
        let mut errors = Vec::new();
        if !self.enabled {
            return errors;
        }
        if self.hours().is_none() {
            let got = self
                .max_delay
                .map_or_else(|| "nothing".to_owned(), |delay| format!("\"{delay}\""));
            errors.push(format!(
                "access-delay max_delay must be a whole number of hours from \"1h\" to \
                 \"{MAX_HOURS}h\", got {got}: EHDS Art. 3(3) allows a delay for a limited period \
                 only, and the period is the Member State's to set"
            ));
        }
        errors.extend(
            self.categories
                .iter()
                .filter(|reference| token(classifier, reference).is_none())
                .map(|reference| {
                    format!(
                        "access-delay categories names {reference:?}, which is not an EHDS \
                         category of the audit category register"
                    )
                }),
        );
        errors
    }
}

/// What the commit path and the admin routes need of an enabled register: the
/// category tokens and the limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessDelayPolicy {
    categories: Vec<String>,
    hours: i32,
}

impl AccessDelayPolicy {
    /// The policy of `register` with its categories resolved through
    /// `classifier`, or `None` when it is off or its limit is unusable.
    ///
    /// A category the classifier does not name is left out, and an unusable
    /// limit leaves no policy; the write
    /// ([`AccessDelayRegister::errors`]) has refused both already.
    #[must_use]
    pub fn compile(register: &AccessDelayRegister, classifier: &Classifier) -> Option<Self> {
        if !register.enabled {
            return None;
        }
        let config = register;
        let hours = config.hours()?;
        Some(Self {
            categories: config
                .categories
                .iter()
                .filter_map(|reference| token(classifier, reference))
                .collect(),
            hours: i32::try_from(hours).ok()?,
        })
    }

    /// The `<system>|<code>` tokens of the held categories.
    #[must_use]
    pub fn categories(&self) -> &[String] {
        &self.categories
    }

    /// The limit of a hold, in hours.
    #[must_use]
    pub fn hours(&self) -> i32 {
        self.hours
    }
}

/// The token of a category reference, never the word for no category.
fn token(classifier: &Classifier, reference: &str) -> Option<String> {
    if reference.trim() == NONE {
        return None;
    }
    classifier.category_token(reference.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_log::categories::CategoryMapConfig;

    fn enabled(categories: &[&str], hours: u64) -> AccessDelayRegister {
        AccessDelayRegister {
            enabled: true,
            categories: categories.iter().map(|c| (*c).to_owned()).collect(),
            max_delay: Some(Duration::from_hours(hours)),
        }
    }

    fn shipped() -> Classifier {
        CategoryMapConfig::default()
            .compile()
            .expect("the default map compiles")
    }

    #[test]
    fn the_register_is_off_by_default() {
        let register = AccessDelayRegister::default();
        assert!(!register.enabled);
        assert_eq!(register.errors(&shipped()), Vec::<String>::new());
        assert_eq!(AccessDelayPolicy::compile(&register, &shipped()), None);
    }

    #[test]
    fn an_enabled_register_needs_a_whole_hour_limit_within_the_ceiling() {
        let map = shipped();
        let no_limit = AccessDelayRegister {
            enabled: true,
            ..AccessDelayRegister::default()
        };
        assert_eq!(no_limit.errors(&map).len(), 1);
        assert_eq!(enabled(&[], 0).errors(&map).len(), 1);
        assert_eq!(enabled(&[], u64::from(MAX_HOURS) + 1).errors(&map).len(), 1);
        let fractional = AccessDelayRegister {
            max_delay: Some(Duration::from_mins(90)),
            ..no_limit
        };
        assert_eq!(fractional.errors(&map).len(), 1);
        assert_eq!(enabled(&[], 72).errors(&map), Vec::<String>::new());
    }

    #[test]
    fn a_category_is_named_as_the_category_map_names_one() {
        let classifier = shipped();
        let register = enabled(&["Laboratory-Reports", "none", "no-such-category"], 72);
        let errors = register.errors(&classifier);
        assert_eq!(errors.len(), 2, "{errors:?}");
        let policy = AccessDelayPolicy::compile(&register, &classifier).expect("enabled");
        assert_eq!(policy.categories().len(), 1);
        assert!(policy.categories()[0].ends_with("|Laboratory-Reports"));
        assert_eq!(policy.hours(), 72);
    }

    #[test]
    fn the_document_reads_from_its_json_form() {
        let register: AccessDelayRegister = serde_json::from_str(
            r#"{"enabled": true, "categories": ["Laboratory-Reports", "Medical-Imaging"],
                "max_delay": "7d"}"#,
        )
        .expect("the document parses");
        assert_eq!(
            register,
            enabled(&["Laboratory-Reports", "Medical-Imaging"], 168)
        );
        assert!(serde_json::from_str::<AccessDelayRegister>(r#"{"hours": 3}"#).is_err());
    }
}
