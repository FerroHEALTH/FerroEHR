// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[privacy.access_delay]` section: whether, and for which categories, the EHDS
//! Art. 3(3) delay of the person's own access applies.
//!
//! Art. 3(3) is a Member State option: a Member State "may restrict" the
//! person's right of access "by delaying access … for a limited period of
//! time" (`docs/law/eu/ehds/text.html`). The section is therefore off by
//! default, and a deployment turns it on, by writing the table, only where its
//! Member State's rule provides for the delay. No openEHR spec governs configuration or the delay:
//! our own design.

use ferrohealth_config::units::Duration;
use serde::{Deserialize, Serialize};

use crate::system_log::categories::{CategoryMapConfig, CategoryMapError, Classifier, NONE};

/// The longest hold a deployment may configure: one year.
///
/// Art. 3(3) allows the delay only "for a limited period of time" and names no
/// figure; the ceiling keeps a typo from becoming a hold of years. No
/// specification fixes the number: our own design.
pub const MAX_HOURS: u32 = 24 * 366;

/// The `[privacy.access_delay]` section. Absent, the delay is off: Art. 3(1)
/// immediate access holds until a deployment says its Member State provides
/// otherwise.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
pub struct AccessDelayConfig {
    /// The categories whose new compositions are held at creation, named as
    /// `[audit.categories]` names one: a priority category of Art. 14(1) by
    /// its code (`Laboratory-Reports`, `Medical-Imaging`, …) or as
    /// `<system>|<code>`, or a national category the map declares
    /// (`FERROEHR__PRIVACY__ACCESS_DELAY__CATEGORIES`, comma-separated). Empty holds
    /// nothing at creation, and a professional still holds an object by hand.
    pub categories: Vec<String>,
    /// How long a hold lasts at most, a whole number of hours from `"1h"` to
    /// [`MAX_HOURS`] hours (`FERROEHR__PRIVACY__ACCESS_DELAY__MAX_DELAY`):
    /// the length of a hold placed at creation and the latest end a
    /// professional may give one. Required.
    pub max_delay: Option<Duration>,
}

impl AccessDelayConfig {
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

    /// The problems boot refuses: a missing, fractional or out-of-range
    /// limit, and a category the `[audit.categories]` map does not name.
    #[must_use]
    pub fn errors(&self, map: &CategoryMapConfig) -> Vec<String> {
        let mut errors = Vec::new();
        if self.hours().is_none() {
            let got = self
                .max_delay
                .map_or_else(|| "nothing".to_owned(), |delay| format!("\"{delay}\""));
            errors.push(format!(
                "privacy.access_delay.max_delay must be a whole number of hours from \"1h\" to \
                 \"{MAX_HOURS}h\", got {got}: EHDS Art. 3(3) allows a delay for a limited period \
                 only, and the period is the Member State's to set"
            ));
        }
        // NOTE: an uncompilable map is reported by the `[audit.categories]` check itself.
        if let Ok(classifier) = map.compile() {
            errors.extend(
                self.categories
                    .iter()
                    .filter(|reference| token(&classifier, reference).is_none())
                    .map(|reference| {
                        format!(
                            "privacy.access_delay.categories names {reference:?}, which is not an \
                             EHDS category of the [audit.categories] map"
                        )
                    }),
            );
        }
        errors
    }
}

/// What the commit path and the admin routes need of an enabled section: the
/// category tokens and the limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessDelayPolicy {
    categories: Vec<String>,
    hours: i32,
}

impl AccessDelayPolicy {
    /// The policy of `config` with its categories resolved through
    /// `classifier`, or `None` when its limit is unusable.
    ///
    /// A category the classifier does not name is left out, and an unusable
    /// limit leaves no policy; boot validation
    /// ([`AccessDelayConfig::errors`]) has refused both already.
    #[must_use]
    pub fn compile(config: &AccessDelayConfig, classifier: &Classifier) -> Option<Self> {
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

    /// The policy of the loaded configuration, its categories resolved
    /// through the `[audit.categories]` map in force; `Ok(None)` when the
    /// section is off.
    ///
    /// # Errors
    /// The map's compile errors, which boot validation reports first.
    pub fn from_config(
        config: &crate::config::FerroEhrConfig,
    ) -> Result<Option<Self>, Vec<CategoryMapError>> {
        let Some(section) = &config.privacy.access_delay else {
            return Ok(None);
        };
        let classifier = config.audit.categories.compile()?;
        Ok(Self::compile(section, &classifier))
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
    use std::collections::HashMap;

    use super::*;

    fn enabled(categories: &[&str], hours: u64) -> AccessDelayConfig {
        AccessDelayConfig {
            categories: categories.iter().map(|c| (*c).to_owned()).collect(),
            max_delay: Some(Duration::from_hours(hours)),
        }
    }

    #[test]
    fn the_section_is_off_by_default() {
        let config = crate::config::FerroEhrConfig::default();
        assert_eq!(config.privacy.access_delay, None);
        assert_eq!(
            AccessDelayPolicy::from_config(&config).expect("the default map compiles"),
            None
        );
    }

    #[test]
    fn a_present_section_needs_a_whole_hour_limit_within_the_ceiling() {
        let map = CategoryMapConfig::default();
        assert_eq!(AccessDelayConfig::default().errors(&map).len(), 1);
        assert_eq!(enabled(&[], 0).errors(&map).len(), 1);
        assert_eq!(enabled(&[], u64::from(MAX_HOURS) + 1).errors(&map).len(), 1);
        let fractional = AccessDelayConfig {
            max_delay: Some(Duration::from_mins(90)),
            ..AccessDelayConfig::default()
        };
        assert_eq!(fractional.errors(&map).len(), 1);
        assert_eq!(enabled(&[], 72).errors(&map), Vec::<String>::new());
    }

    #[test]
    fn a_category_is_named_as_the_category_map_names_one() {
        let map = CategoryMapConfig::default();
        let config = enabled(&["Laboratory-Reports", "none", "no-such-category"], 72);
        let errors = config.errors(&map);
        assert_eq!(errors.len(), 2, "{errors:?}");
        let classifier = map.compile().expect("the default map compiles");
        let policy = AccessDelayPolicy::compile(&config, &classifier).expect("enabled");
        assert_eq!(policy.categories().len(), 1);
        assert!(policy.categories()[0].ends_with("|Laboratory-Reports"));
        assert_eq!(policy.hours(), 72);
    }

    #[test]
    fn the_section_maps_from_the_environment() {
        let env: HashMap<String, String> = [
            (
                "FERROEHR__PRIVACY__ACCESS_DELAY__CATEGORIES",
                "Laboratory-Reports,Medical-Imaging",
            ),
            ("FERROEHR__PRIVACY__ACCESS_DELAY__MAX_DELAY", "7d"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let config = crate::config::assemble(None, &env, &[]).expect("the config assembles");
        assert_eq!(
            config.privacy.access_delay,
            Some(enabled(&["Laboratory-Reports", "Medical-Imaging"], 168))
        );
    }
}
