# specifications-AM - openEHR Archetype Model Specifications

This repository contains the document and model sources (Asciidoctor and MagicDraw) for the openEHR Archetype Model (AM) specifications. The published form may be accessed from the openEHR website [AM component index page](http://www.openehr.org/releases/AM/development/docs/index).

## Contents

```
manifest.json                                       # Index of spec titles & other meta-data (used by PHP site & Asciidcotor)

/computable
    +--- /UML
           +--- openEHR-UML-AM.mdzip                # MagicDraw files
/docs
    +--- /<spec 1>                                  # Specification #1 e.g. ADL2
    |      +--- /diagrams                           # .svg, .png and .drawio diagram files
    |      +--- /images                             # .png screen captures & other images
    |      +--- manifest_vars.adoc                  # .adoc vars file extracted from manifest.json file above
    |      +--- master.adoc                         # specification master (root) .adoc file
    |      +--- master00-amendment_record.adoc      # specification amendment record file
    |      +--- master01-preface.adoc               # specification preface .adoc file
    |      +--- master02-xxx.adoc                   # specification main content .adoc file
    |      +--- master03-xxx.adoc                   # specification main content .adoc file
    |      +       ...
    |      +--- masterNN-xxx.adoc                   # specification main content .adoc file
    |
    +--- /<spec N>    
    |      +--- etc
    |
    +--- /UML                                       # Asciidoctor .adoc files extracted from UML file above
            +--- /classes                           # extracted .adoc files for each class
            +--- /diagrams                          # extracted .svg files for each class diagram
```
