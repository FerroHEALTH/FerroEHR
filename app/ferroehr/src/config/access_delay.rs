// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[access_delay]` section: whether, and for which categories, the EHDS
//! Art. 3(3) delay of the person's own access applies.
//!
//! Art. 3(3) is a Member State option: a Member State "may restrict" the
//! person's right of access "by delaying access … for a limited period of
//! time" (`docs/law/eu/ehds/text.html`). The section is therefore off by
//! default, and a deployment turns it on only where its Member State's rule
//! provides for the delay. No openEHR spec governs configuration or the delay:
//! our own design.

use serde::{Deserialize, Serialize};

use crate::system_log::categories::{CategoryMapConfig, CategoryMapError, Classifier, NONE};

/// The longest hold a deployment may configure: one year.
///
/// Art. 3(3) allows the delay only "for a limited period of time" and names no
/// figure; the ceiling keeps a typo from becoming a hold of years. No
/// specification fixes the number: our own design.
pub const MAX_HOURS: u32 = 24 * 366;

/// The `[access_delay]` section; its default is off, with no category and no
/// limit.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AccessDelayConfig {
    /// Whether the delay applies (`FERROEHR__ACCESS_DELAY__ENABLED`). Off by
    /// default: Art. 3(1) immediate access holds until a deployment says its
    /// Member State provides otherwise.
    pub enabled: bool,
    /// The categories whose new compositions are held at creation, named as
    /// `[audit.categories]` names one: a priority category of Art. 14(1) by
    /// its code (`Laboratory-Reports`, `Medical-Imaging`, …) or as
    /// `<system>|<code>`, or a national category the map declares
    /// (`FERROEHR__ACCESS_DELAY__CATEGORIES`, comma-separated). Empty holds
    /// nothing at creation, and a professional still holds an object by hand.
    pub categories: Vec<String>,
    /// How long a hold lasts at most, in hours, 1 to [`MAX_HOURS`]
    /// (`FERROEHR__ACCESS_DELAY__HOURS`): the length of a hold placed at
    /// creation and the latest end a professional may give one.
    pub hours: u32,
}

impl AccessDelayConfig {
    /// The problems boot refuses: a missing or out-of-range limit, and a
    /// category the `[audit.categories]` map does not name. Nothing is checked
    /// while the section is off.
    #[must_use]
    pub fn errors(&self, map: &CategoryMapConfig) -> Vec<String> {
        if !self.enabled {
            return Vec::new();
        }
        let mut errors = Vec::new();
        if !(1..=MAX_HOURS).contains(&self.hours) {
            errors.push(format!(
                "access_delay.hours must be 1 to {MAX_HOURS} while access_delay.enabled is \
                 true, got {}: EHDS Art. 3(3) allows a delay for a limited period only, and \
                 the period is the Member State's to set",
                self.hours
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
                            "access_delay.categories names {reference:?}, which is not an \
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
    /// `classifier`, or `None` when the section is off.
    ///
    /// A category the classifier does not name is left out; boot validation
    /// ([`AccessDelayConfig::errors`]) has refused one already.
    #[must_use]
    pub fn compile(config: &AccessDelayConfig, classifier: &Classifier) -> Option<Self> {
        if !config.enabled {
            return None;
        }
        Some(Self {
            categories: config
                .categories
                .iter()
                .filter_map(|reference| token(classifier, reference))
                .collect(),
            hours: i32::try_from(config.hours.clamp(1, MAX_HOURS)).unwrap_or(i32::MAX),
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
        if !config.access_delay.enabled {
            return Ok(None);
        }
        let classifier = config.audit.categories.compile()?;
        Ok(Self::compile(&config.access_delay, &classifier))
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

    fn enabled(categories: &[&str], hours: u32) -> AccessDelayConfig {
        AccessDelayConfig {
            enabled: true,
            categories: categories.iter().map(|c| (*c).to_owned()).collect(),
            hours,
        }
    }

    #[test]
    fn the_section_is_off_by_default_and_then_checks_nothing() {
        let config = AccessDelayConfig::default();
        assert!(!config.enabled);
        let off = AccessDelayConfig {
            hours: 0,
            categories: vec!["no-such-category".to_owned()],
            ..config
        };
        assert_eq!(
            off.errors(&CategoryMapConfig::default()),
            Vec::<String>::new()
        );
        assert_eq!(
            AccessDelayPolicy::compile(&off, &Classifier::default()),
            None
        );
    }

    #[test]
    fn an_enabled_section_needs_a_limit_within_the_ceiling() {
        let map = CategoryMapConfig::default();
        assert_eq!(enabled(&[], 0).errors(&map).len(), 1);
        assert_eq!(enabled(&[], MAX_HOURS + 1).errors(&map).len(), 1);
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
            ("FERROEHR__ACCESS_DELAY__ENABLED", "true"),
            (
                "FERROEHR__ACCESS_DELAY__CATEGORIES",
                "Laboratory-Reports,Medical-Imaging",
            ),
            ("FERROEHR__ACCESS_DELAY__HOURS", "168"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let config = crate::config::assemble(None, &env, &[]).expect("the config assembles");
        assert_eq!(
            config.access_delay,
            enabled(&["Laboratory-Reports", "Medical-Imaging"], 168)
        );
    }
}
