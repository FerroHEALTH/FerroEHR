// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The manufacturer of FerroEHR, named in the running system.
//!
//! Regulation (EU) 2025/327 (`docs/law/eu/ehds/text.html` Art. 30(1)(g)) has
//! the manufacturer of an EHR system "indicate the name, registered trade name
//! or registered trade mark, the postal address, and the website, email
//! address or other digital contact details through which they can be
//! contacted, in the EHR system", with a single point at which it can be
//! contacted. The manufacturer of each tagged release is the Licensor
//! `LICENSE` names.
//!
//! This file is the one place those details are written. The startup banner,
//! `ferroehr --version`, `GET /management/info`, the default `vendor` of the
//! `OPTIONS` System-Options manifest and `ferroehr report` read [`MANUFACTURER`]
//! from here, and the viewer compiles this same file through a `#[path]`
//! module because it links no application crate. The container images repeat
//! the one-line form in their `eu.ferroehr.image.manufacturer` label, which a
//! test holds to this file. The file depends on nothing but `serde`, so both
//! crates and both viewer targets compile it unchanged.

/// The manufacturer of an EHR system, as Art 30(1)(g) asks it to be named.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Manufacturer {
    /// The registered name.
    pub name: &'static str,
    /// The postal address, on one line.
    pub postal_address: &'static str,
    /// The email address, the single point of contact.
    pub email: &'static str,
    /// The website through which a person contacts the manufacturer.
    pub website: &'static str,
}

/// The manufacturer of FerroEHR: Cadasto B.V., the Licensor.
pub const MANUFACTURER: Manufacturer = Manufacturer {
    name: "Cadasto B.V.",
    postal_address: "Comeniusstraat 2d, 1817 MS Alkmaar, The Netherlands",
    email: "info@cadasto.com",
    website: "https://www.cadasto.com/contact/",
};

impl Manufacturer {
    /// Returns the manufacturer on one line: the name, the postal address and
    /// the single point of contact.
    #[must_use]
    pub fn line(&self) -> String {
        format!("{}, {}, {}", self.name, self.postal_address, self.email)
    }

    /// Returns what a FerroEHR binary's `--version` prints after its name:
    /// `version`, then the manufacturer on one line and its website.
    #[must_use]
    pub fn version_text(&self, version: &str) -> String {
        format!(
            "{version}\nManufactured by {}\n{}",
            self.line(),
            self.website
        )
    }
}
