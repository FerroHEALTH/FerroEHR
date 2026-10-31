# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1

# The English catalog of the definitions group: the reference catalog and
# the one every lookup falls back to. Each message is one `DefinitionsMsg`
# of the crate's `locale::definitions` module; a catalog of another language
# holds the same messages in a directory named by its BCP 47 tag beside this
# one.

template-deleted = Template deleted
template-source = Source
template-uploaded = Template uploaded
candidate-code-2 = candidate code
value-set-id-2 = value set id
terminology-id = terminology id
reference-code-2 = reference code
term-code-2 = term code
operationaltemplatev2-parse = OperationalTemplateV2 parse: { $e }
webtemplate-build = WebTemplate build: { $value0 }
terminology-json = terminology JSON: { $e }
a-is-required = a { $what } is required

## Numbers: the marks FerroUI's formatter groups a number with

number-group = ,
number-decimal = .

templates-subtitle = The operational templates the CDR validates compositions against.
templates-upload = Upload a template
templates-find = Id, concept or archetype
templates-families = Template families
templates-family = Family
templates-empty = No template is uploaded yet.
templates-no-match = No template matches.
templates-refused = The templates could not be read.
chip-contains = Contains: { $value }
column-template = Template
column-root-archetype = Root archetype
column-uploaded = Uploaded
template-build-query = Build a query
template-delete = Delete
template-delete-title = Delete the template { $name }?
template-delete-message = The CDR refuses while compositions use the template; delete or migrate them first.
template-delete-confirm = Delete the template
template-delete-refused = The CDR refused to delete the template.
template-more = More actions for the template
template-download = Download
template-copy-id = Copy id
template-views = Views of this template
template-refused = The template could not be read.
template-incomplete = { $count ->
    [one] This template cannot be used yet: it is built on an archetype this CDR does not hold.
   *[other] This template cannot be used yet: it is built on { $count } archetypes this CDR does not hold.
}
template-incomplete-upload = Upload the missing archetypes
template-incomplete-tab = This view needs the missing archetypes. Upload them, then open it again.
template-adl-14-line = ADL 1.4
template-adl-14-subtitle = { $concept } · ADL 1.4
template-adl-2-subtitle = { $hrid } · ADL 2
template-paths = Paths
template-json = JSON
template-example = Example
template-paths-of = The paths of { $name }
template-node = Node
template-pick-node = Pick a node in the tree to read its path, type and codes.
template-adl-2-paths = The viewer builds these paths itself: the CDR has no web template for ADL 2.
template-no-catalog = The CDR sent no operational template to build paths from.
template-no-source = The CDR holds no source for this release.
template-source-title = Source
template-json-title = JSON · release { $release }
template-example-type = Type
template-example-detail = Detail level
template-show-example = Show the example
template-example-title = Example · { $format }
template-no-example = The CDR built no example for these options.
template-no-example-for-root = A template rooted at { $root } has no example: the CDR builds examples for COMPOSITION and party templates.
template-example-refused = The CDR could not build an example for these options.
template-identity = Identity
template-use = Use
template-use-line = Query the compositions committed under it
template-concept = Concept
template-version = Template version
template-default-language = Default language
template-languages = Languages
template-uid = UID
template-release = Release
template-releases = Releases
template-release-count = { $count ->
    [one] 1 release
   *[other] { $count } releases
}
template-reading-release = You are reading release { $release }.
template-read-latest = Read { $release }, the latest
template-upload-title = { $mode ->
    [new] Upload a template
   *[edit] Edit
}
template-upload-subtitle = Choose a file or paste the source; the CDR validates it on upload.
template-opt-source = The operational template
template-adl-2-source = The ADL 2 source
template-choose-file = Choose a file
template-opt-placeholder = <?xml version="1.0" encoding="UTF-8"?>
template-adl-2-placeholder = template (adl_version=2.4.0; rm_release=1.1.0)
template-not-yet = Not uploaded yet.
template-not-uploaded = Not uploaded.
template-upload = Upload this template
template-upload-refused = The CDR refused the template, so nothing was uploaded.

term-subtitle = What the CDR can answer about the code sets it serves.
term-operations = Terminology operations
term-terminologies = Terminologies
term-lookup = Look up
term-value-sets = Value sets
term-hierarchy = Hierarchy
term-off = The CDR's terminology API is off.
term-off-line = Set in ferroehr.toml and read at start; the Configuration page shows the running value.
term-refused = The terminologies could not be read.
term-terminology = Terminology
term-publisher = Publisher
term-served = Terminologies the CDR serves
term-id = Id
term-uri = Identifying URI
term-fields = Fields you can ask for
term-descriptor = Descriptor
term-lookup-in = Look up a code in it
term-expand-group = Expand one of its groups
term-code = Code
term-effective-date = Effective date (optional)
term-look-up = Look up
term-defined = Defined:
term-undefined = The terminology { $terminology } does not define { $code }.
term-language = Language
term-preferred = Preferred term
term-lookup-refused = The CDR could not look the code up.
term-value-set = Value set
term-expand = Expand
term-members = Members
term-no-value-set = The terminology has no value set { $set }.
term-value-set-refused = The CDR could not expand the value set.
term-validate-title = Validate a code
term-validate = Validate
term-member = { $code } is a member of { $set }.
term-not-member = { $code } is not a member of { $set }.
term-validate-refused = The CDR could not validate the code.
term-broader = Broader code
term-narrower = Narrower code
term-test = Test
term-subsumes = { $broader } is broader than { $narrower }.
term-not-subsumes = { $broader } is not broader than { $narrower }.
term-flat = The openEHR vocabulary has no broader or narrower codes, so it always answers no; a terminology server bound to the CDR can answer otherwise.
term-subsumption-refused = The CDR could not test the codes.

template-delete-row = Delete…
