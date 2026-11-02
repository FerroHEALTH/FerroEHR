- Five archive dump and load failures still answer `500`, and their bodies are now fixed
  text that echoes no value from the archive:
  - an unreadable data segment no longer names the segment, though `manifest.json` and
    `demographic-commons.json` are still named;
  - an unknown logical format in the manifest no longer echoes the declared value;
  - a version whose id carries a path separator, and a version whose commit audit the
    export lacks, no longer name the version.
