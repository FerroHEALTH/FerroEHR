// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The manufacturer FerroEHR names in the running system, pinned as
//! Regulation (EU) 2025/327 (`docs/law/eu/ehds/text.html` Art. 30(1)(g)) asks
//! for it: the name, the postal address, and the digital contact with its
//! single point. The name is held to the Licensor the licence files name, and
//! the container images' manufacturer label to this one constant.

#![expect(
    clippy::panic_in_result_fn,
    reason = "the blessed test shape (the Rust Book ch11-01): `?` propagates \
              plumbing failures — here the file reads — while the assertion \
              carries the behaviour under test and is meant to panic"
)]

use std::error::Error;
use std::path::Path;

use ferroehr::manufacturer::MANUFACTURER;

/// A repository file, read from this crate's position in the tree.
fn repository_file(relative: &str) -> Result<String, std::io::Error> {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(relative),
    )
}

#[test]
fn the_manufacturer_is_the_licensor_with_its_address_and_contact() {
    assert_eq!("Cadasto B.V.", MANUFACTURER.name);
    assert_eq!(
        "Comeniusstraat 2d, 1817 MS Alkmaar, The Netherlands",
        MANUFACTURER.postal_address
    );
    assert_eq!("info@cadasto.com", MANUFACTURER.email);
    assert_eq!("https://www.cadasto.com/contact/", MANUFACTURER.website);
}

#[test]
fn the_one_line_form_names_the_address_and_the_single_point_of_contact() {
    assert_eq!(
        "Cadasto B.V., Comeniusstraat 2d, 1817 MS Alkmaar, The Netherlands, info@cadasto.com",
        MANUFACTURER.line()
    );
}

#[test]
fn the_version_text_follows_the_version_with_the_manufacturer() {
    assert_eq!(
        "9.9.9\nManufactured by Cadasto B.V., Comeniusstraat 2d, 1817 MS Alkmaar, The Netherlands, \
         info@cadasto.com\nhttps://www.cadasto.com/contact/",
        MANUFACTURER.version_text("9.9.9")
    );
}

/// The manufacturer is the Licensor both licence files name.
#[test]
fn the_manufacturer_is_the_licensor_the_licence_names() -> Result<(), Box<dyn Error>> {
    for file in ["LICENSE", "LICENSES/BUSL-1.1.txt"] {
        let licence = repository_file(file)?;
        let licensor = licence
            .lines()
            .find_map(|line| line.strip_prefix("Licensor:"))
            .map(str::trim);
        assert_eq!(Some(MANUFACTURER.name), licensor, "{file}");
    }
    Ok(())
}

#[test]
fn the_manufacturer_is_written_with_its_field_names() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        r#"{"name":"Cadasto B.V.","postal_address":"Comeniusstraat 2d, 1817 MS Alkmaar, The Netherlands","email":"info@cadasto.com","website":"https://www.cadasto.com/contact/"}"#,
        serde_json::to_string(&MANUFACTURER)?
    );
    Ok(())
}

/// Every image the release ships carries the one-line form in its
/// `eu.ferroehr.image.manufacturer` label.
#[test]
fn every_image_label_names_the_manufacturer() -> Result<(), Box<dyn Error>> {
    const KEY: &str = "eu.ferroehr.image.manufacturer=\"";
    for file in [
        "docker/Dockerfile",
        "docker/viewer/Dockerfile",
        "docker/postgres/Dockerfile",
    ] {
        let dockerfile = repository_file(file)?;
        let label = dockerfile
            .lines()
            .find_map(|line| line.trim().strip_prefix(KEY))
            .and_then(|rest| rest.split_once('"'))
            .map(|(value, _)| value);
        assert_eq!(Some(MANUFACTURER.line().as_str()), label, "{file}");
    }
    Ok(())
}
