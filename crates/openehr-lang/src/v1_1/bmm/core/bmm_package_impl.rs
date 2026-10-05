// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written spec functions of `BMM_PACKAGE` and of the
//! `BMM_PACKAGE_CONTAINER` surface it shares with `BMM_MODEL`.
//!
//! Spec: `LANG/docs/UML/classes/org.openehr.lang.bmm.bmm_package.adoc`
//! §Functions (`root_classes`, `path`) and
//! `…bmm.bmm_package_container.adoc` §Attributes + §Functions
//! (`packages`: "Child packages; keys all in upper case for guaranteed
//! matching"; `package_at_path`, `do_recursive_packages`, `has_package_path`:
//! "paths are delimited with Package_name_delimiter"), with the delimiter
//! itself from `…bmm.bmm_definitions.adoc` §Constants
//! (`Package_name_delimiter` `"."`).
//!
//! The three container functions are implemented once here, as `pub(crate)`
//! free functions over a `packages` map, and called from
//! [`BmmPackage`], [`BmmModel`](crate::v1_1::bmm::core::bmm_model::BmmModel) —
//! the two `BMM_PACKAGE_CONTAINER` descendants
//! (`…bmm.bmm_model.adoc` §Inherit lists `BMM_PACKAGE_CONTAINER`) — and the
//! [`BmmPackageContainer`] slot itself, which also carries the class's own
//! least-rich form.

use std::collections::BTreeMap;

use crate::v1_1::bmm::core::bmm_class::BmmClass;
use crate::v1_1::bmm::core::bmm_definitions::BmmDefinitionsData;
use crate::v1_1::bmm::core::bmm_package::BmmPackage;
use crate::v1_1::bmm::core::bmm_package_container::BmmPackageContainer;

/// Splits a package path into its non-empty segments, delimited by
/// `BMM_DEFINITIONS.Package_name_delimiter`
/// (`org.openehr.lang.bmm.bmm_definitions.adoc` §Constants).
fn path_segments(path: &str) -> Vec<&str> {
    path.split(BmmDefinitionsData::PACKAGE_NAME_DELIMITER)
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect()
}

/// `BMM_PACKAGE_CONTAINER.package_at_path` over a `packages` map: "Package at
/// the path `a_path`" (`org.openehr.lang.bmm.bmm_package_container.adoc`
/// §Functions).
///
/// Keys are matched case-insensitively, because the container's `packages` map
/// is specified as having its "keys all in upper case for guaranteed matching"
/// (class doc §Attributes) while a package's own `name` keeps its source case.
/// At each level the LONGEST matching key prefix is tried first, since a
/// top-level package name "may be qualified"
/// (`org.openehr.lang.bmm.bmm_package.adoc` §Attributes), i.e. one key can span
/// several path segments (`org.openehr.rm`).
pub(crate) fn package_at_path_in<'a>(
    packages: Option<&'a BTreeMap<String, BmmPackage>>,
    path: &str,
) -> Option<&'a BmmPackage> {
    resolve_segments(packages, &path_segments(path))
}

/// Resolves `segments` against a `packages` map, longest key prefix first.
fn resolve_segments<'a>(
    packages: Option<&'a BTreeMap<String, BmmPackage>>,
    segments: &[&str],
) -> Option<&'a BmmPackage> {
    let packages = packages?;
    if segments.is_empty() {
        return None;
    }
    for taken in (1..=segments.len()).rev() {
        let Some(prefix) = segments.get(..taken) else {
            continue;
        };
        let key = prefix.join(BmmDefinitionsData::PACKAGE_NAME_DELIMITER);
        let hit = packages
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(&key));
        let Some((_, package)) = hit else {
            continue;
        };
        match segments.get(taken..) {
            None | Some([]) => return Some(package),
            Some(rest) => {
                if let Some(found) = resolve_segments(package.packages.as_ref(), rest) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// `BMM_PACKAGE_CONTAINER.do_recursive_packages` over a `packages` map:
/// "Recursively execute `action`, which is a procedure taking a BMM_PACKAGE
/// argument, on all members of packages"
/// (`org.openehr.lang.bmm.bmm_package_container.adoc` §Functions). Each package
/// is visited before its children, in key order.
pub(crate) fn do_recursive_packages_in(
    packages: Option<&BTreeMap<String, BmmPackage>>,
    action: &mut dyn FnMut(&BmmPackage),
) {
    for package in packages.into_iter().flat_map(BTreeMap::values) {
        action(package);
        do_recursive_packages_in(package.packages.as_ref(), action);
    }
}

impl BmmPackage {
    /// `BMM_PACKAGE.path`: "Full path of this package back to root package"
    /// (class doc §Functions).
    ///
    /// NOTE: the generated `BMM_PACKAGE` is a standalone tree node — it carries
    /// its child `packages` but no back-reference to its parent (class doc
    /// §Attributes lists `name` and `classes` only), so the path can only be
    /// this package's own `name`, which the class doc states "may be qualified
    /// if it is a top-level package" and is therefore already the full path for
    /// every package a schema declares at top level. A nested package reached
    /// through [`Self::package_at_path`] returns its unqualified segment; the
    /// caller holds the prefix it navigated with.
    #[must_use]
    pub fn path(&self) -> &str {
        self.name.as_str()
    }

    /// `BMM_PACKAGE.root_classes`: "Obtain the set of top-level classes in this
    /// package, either from this package itself or by recursing into the
    /// structure until classes are obtained from child packages. Recurse into
    /// each child only far enough to find the first level of classes" (class doc
    /// §Functions).
    #[must_use]
    pub fn root_classes(&self) -> Vec<&BmmClass> {
        if let Some(classes) = &self.classes
            && !classes.is_empty()
        {
            return classes.iter().collect();
        }
        let mut out = Vec::new();
        for child in self.packages.iter().flat_map(BTreeMap::values) {
            out.extend(child.root_classes());
        }
        out
    }

    /// `BMM_PACKAGE_CONTAINER.package_at_path`: "Package at the path `a_path`"
    /// (`org.openehr.lang.bmm.bmm_package_container.adoc` §Functions).
    ///
    /// Keys are matched case-insensitively and the longest matching key prefix
    /// wins at each level (module docs).
    #[must_use]
    pub fn package_at_path(&self, a_path: &str) -> Option<&Self> {
        package_at_path_in(self.packages.as_ref(), a_path)
    }

    /// `BMM_PACKAGE_CONTAINER.has_package_path`: "True if there is a package at
    /// the path `a_path`; paths are delimited with Package_name_delimiter"
    /// (`org.openehr.lang.bmm.bmm_package_container.adoc` §Functions).
    #[must_use]
    pub fn has_package_path(&self, a_path: &str) -> bool {
        self.package_at_path(a_path).is_some()
    }

    /// `BMM_PACKAGE_CONTAINER.do_recursive_packages`: "Recursively execute
    /// `action`, which is a procedure taking a BMM_PACKAGE argument, on all
    /// members of packages"
    /// (`org.openehr.lang.bmm.bmm_package_container.adoc` §Functions).
    pub fn do_recursive_packages(&self, action: &mut dyn FnMut(&Self)) {
        do_recursive_packages_in(self.packages.as_ref(), action);
    }
}

impl BmmPackageContainer {
    /// `BMM_PACKAGE_CONTAINER.packages`: "Child packages; keys all in upper case
    /// for guaranteed matching" (class doc §Attributes), read through whichever
    /// descendant this slot carries.
    #[must_use]
    pub fn packages(&self) -> Option<&BTreeMap<String, BmmPackage>> {
        match self {
            Self::BmmModel(model) => model.packages.as_ref(),
            Self::BmmPackage(package) => package.packages.as_ref(),
            Self::BmmPackageContainer(container) => container.packages.as_ref(),
        }
    }

    /// `BMM_PACKAGE_CONTAINER.package_at_path`: "Package at the path `a_path`"
    /// (`org.openehr.lang.bmm.bmm_package_container.adoc` §Functions).
    ///
    /// Keys are matched case-insensitively and the longest matching key prefix
    /// wins at each level (module docs).
    #[must_use]
    pub fn package_at_path(&self, a_path: &str) -> Option<&BmmPackage> {
        package_at_path_in(self.packages(), a_path)
    }

    /// `BMM_PACKAGE_CONTAINER.has_package_path`: "True if there is a package at
    /// the path `a_path`; paths are delimited with Package_name_delimiter"
    /// (`org.openehr.lang.bmm.bmm_package_container.adoc` §Functions).
    #[must_use]
    pub fn has_package_path(&self, a_path: &str) -> bool {
        self.package_at_path(a_path).is_some()
    }

    /// `BMM_PACKAGE_CONTAINER.do_recursive_packages`: "Recursively execute
    /// `action`, which is a procedure taking a BMM_PACKAGE argument, on all
    /// members of packages"
    /// (`org.openehr.lang.bmm.bmm_package_container.adoc` §Functions).
    pub fn do_recursive_packages(&self, action: &mut dyn FnMut(&BmmPackage)) {
        do_recursive_packages_in(self.packages(), action);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::v1_1::bmm::core::bmm_class::BmmClass;
    use crate::v1_1::bmm::core::bmm_class::BmmClassData;
    use crate::v1_1::bmm::core::bmm_package::BmmPackage;

    /// A package with the given name, child packages and class names.
    fn package(name: &str, children: Vec<BmmPackage>, classes: &[&str]) -> BmmPackage {
        let mut package = BmmPackage {
            documentation: None,
            packages: None,
            name: name.to_owned(),
            classes: openehr_base::containers::present(
                classes.iter().map(|c| simple_class(c)).collect(),
            ),
        };
        if !children.is_empty() {
            let map: BTreeMap<String, BmmPackage> = children
                .into_iter()
                .map(|child| (child.name.to_uppercase(), child))
                .collect();
            package.packages = Some(map);
        }
        package
    }

    /// A simple class named `name`.
    fn simple_class(name: &str) -> BmmClass {
        BmmClass::BmmClass(BmmClassData {
            documentation: None,
            name: name.to_owned(),
            ancestors: None,
            package: BmmPackage {
                documentation: None,
                packages: None,
                name: "org.openehr.rm".to_owned(),
                classes: openehr_base::containers::present(Vec::new()),
            },
            properties: None,
            source_schema_id: "openehr_test_1.0.0".to_owned(),
            immediate_descendants: openehr_base::containers::present(Vec::new()),
            is_abstract: false,
            is_primitive_type: false,
            is_override: false,
        })
    }

    #[test]
    fn package_at_path_matches_keys_case_insensitively() {
        let root = package(
            "org.openehr.rm",
            vec![package("composition", Vec::new(), &["COMPOSITION"])],
            &[],
        );
        assert!(root.has_package_path("composition"));
        assert!(root.has_package_path("COMPOSITION"));
        assert!(root.has_package_path("Composition"));
        assert_eq!(
            root.package_at_path("composition").map(BmmPackage::path),
            Some("composition")
        );
        assert_eq!(root.package_at_path("ehr"), None);
    }

    #[test]
    fn package_at_path_walks_nested_segments() {
        let root = package(
            "org.openehr.rm",
            vec![package(
                "data_structures",
                vec![package("item_structure", Vec::new(), &["ITEM_TREE"])],
                &[],
            )],
            &[],
        );
        let leaf = root
            .package_at_path("data_structures.item_structure")
            .expect("the nested package resolves");
        assert_eq!(leaf.path(), "item_structure");
        assert!(!root.has_package_path("data_structures.missing"));
    }

    #[test]
    fn root_classes_recurse_only_to_the_first_level_of_classes() {
        let deep = package("deeper", Vec::new(), &["ELEMENT"]);
        let child = package("data_structures", vec![deep], &[]);
        let root = package("org.openehr.rm", vec![child], &[]);
        assert_eq!(
            root.root_classes()
                .into_iter()
                .map(BmmClass::name)
                .collect::<Vec<_>>(),
            ["ELEMENT"]
        );

        let with_own = package(
            "org.openehr.rm",
            vec![package("child", Vec::new(), &["IGNORED"])],
            &["COMPOSITION"],
        );
        assert_eq!(
            with_own
                .root_classes()
                .into_iter()
                .map(BmmClass::name)
                .collect::<Vec<_>>(),
            ["COMPOSITION"]
        );
    }

    /// The `BMM_PACKAGE_CONTAINER` slot answers the three container functions
    /// for every descendant it can carry, INCLUDING the class's own least-rich
    /// form — the boundary case a dispatcher over the two named descendants
    /// alone would miss.
    #[test]
    fn the_package_container_slot_dispatches_over_every_form() {
        use crate::v1_1::bmm::core::bmm_package_container::BmmPackageContainer;
        use crate::v1_1::bmm::core::bmm_package_container::BmmPackageContainerData;

        let leaf = package("composition", Vec::new(), &["COMPOSITION"]);
        let root = package("org.openehr.rm", vec![leaf], &[]);
        let packages: BTreeMap<String, BmmPackage> =
            [("ORG.OPENEHR.RM".to_owned(), root.clone())].into();

        let own = BmmPackageContainer::BmmPackageContainer(BmmPackageContainerData {
            documentation: None,
            packages: Some(packages),
        });
        assert!(own.has_package_path("org.openehr.rm.composition"));
        assert_eq!(
            own.package_at_path("org.openehr.rm").map(BmmPackage::path),
            Some("org.openehr.rm")
        );
        let mut visited: Vec<String> = Vec::new();
        own.do_recursive_packages(&mut |package| visited.push(package.name.clone()));
        assert_eq!(
            visited,
            ["org.openehr.rm".to_owned(), "composition".to_owned()]
        );

        let as_package = BmmPackageContainer::BmmPackage(root);
        assert!(as_package.has_package_path("composition"));
        assert!(!as_package.has_package_path("ehr"));

        // An empty container answers every function without a package.
        let empty = BmmPackageContainer::BmmPackageContainer(BmmPackageContainerData {
            documentation: None,
            packages: None,
        });
        assert!(!empty.has_package_path("composition"));
        assert!(empty.package_at_path("composition").is_none());
        let mut none_visited = 0usize;
        empty.do_recursive_packages(&mut |_| none_visited += 1);
        assert_eq!(none_visited, 0);
    }

    #[test]
    fn do_recursive_packages_visits_every_descendant() {
        let root = package(
            "org.openehr.rm",
            vec![
                package("composition", Vec::new(), &[]),
                package(
                    "data_structures",
                    vec![package("item_structure", Vec::new(), &[])],
                    &[],
                ),
            ],
            &[],
        );
        let mut visited: Vec<String> = Vec::new();
        root.do_recursive_packages(&mut |package| visited.push(package.name.clone()));
        assert_eq!(
            visited,
            [
                "composition".to_owned(),
                "data_structures".to_owned(),
                "item_structure".to_owned()
            ]
        );
    }
}
