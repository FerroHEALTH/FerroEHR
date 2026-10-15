# specifications-ITS-JSON

JSON schemas for use with openEHR systems and tools. See [release baseline](https://specifications.openehr.org/release_baseline) of current specifications.

Schema format is based on [draft-07 of JSON-Schema specifications](https://json-schema.org/specification.html).

## Releases and IM Versions

> [!NOTE]
> These schemas are in **DEVELOPMENT** state and subject to change.

The repository is structured as follows:

```
/examples                # JSON examples
/components
    /AM                  # schemas for AM component
        /Release-1.4     # schemas for Release-1.4 of AM
        /Release-2.1.0   # schemas for Release-2.1.0 of AM
        /Release-2.2.0   # schemas for Release-2.2.0 of AM
    /BASE                # schemas for BASE component
        /Release-1.1.0   # schemas for Release-1.1.0 of BASE
    /RM                  # schemas for RM component
        /Release-1.0.3   # schemas for Release-1.0.3 of RM
        /Release-1.0.4   # schemas for Release-1.0.4 of RM
        /Release-1.1.0   # schemas for Release-1.1.0 of RM
```

## Structure

Schemas are organized in directories considering openEHR components (e.g. `AM`, `RM`, etc.), release name and package (e.g. `/components/RM/Release-1.1.0/Data_Structures`). Each directory contains several json schema files named as `[type].json`, while packages contains also a `main.json` file. On the component level there is also few files containing all types of that package, i.e. `openehr_rm_1.1.0_all.json`.

## Known issues

The followings are some known issues of openEHR JSON-Schema. These will be addressed in the near future and will cause the schemas to be changed before it reaches maturity.

A longer discussion on this subject can be found at [openEHR forum](https://discourse.openehr.org/t/json-schema-and-openapi-current-state-and-how-to-progress/1385).

### Available components, packages and versions

There is only one BASE version currently available, Release-1.1.0. The BASE Release-1.0.0 is also possible to generate, but there is no BASE Release-1.2.0 BMM file available to generate the JSON schema from.

### Design choices

- no inheritance in definitions, so definitions contain all their fields directly. This is according to json schema recommendations.
- no abstract classes are generated, as they cannot appear in json schema - so no LOCATABLE.json!
- if .... construction for polymorphism, which makes it easier to read validation messages, and makes validation very fast
- if ... construction is in the reference
- one _all file per RM version, which contains the entire schema and works with many validators. Recommended for automated use!
- A split in classes generated one, with one main.json per RM version. The main.json defines which json can appear at the root level. This one will have some problems with some validators, and will work fine with others.

### OpenAPI

There is currently an experiment (demo) of this [on a project to show how OpenAPI can work with openEHR](https://github.com/nedap/openehr-openapi).

## Acknowledgements

The initial JSON schema was authored by [Code24](https://www.code24.nl), generated from openEHR UMLs.
The current version is generated from openEHR BMM files with [Archie](https://github.com/openEHR/archie) by [Nedap](https://nedap-healthcare.com).

