- **The EHDS system documents live in the FerroHEALTH book, and each release
  attaches them at a pinned revision** (#3705). The EHR system Cadasto B.V.
  declares is FerroEHR with FerroBRIDGE, so its information sheet (EHDS
  Art. 38), instructions for use (Art. 30(1)(d)), readiness record and
  technical documentation (Art. 37, Annex III) move to
  <https://ferrohealth.eu/docs/ehds/>, and the old addresses in this book
  redirect there. `docs/ehds/system-documents.env` pins the FerroHEALTH commit
  and the revision of each page. The release lane fetches the two attached
  pages at that commit, refuses a page whose `Revised` line differs from the
  pin, and appends this release's facts under "This copy": the system version
  (this FerroEHR release with the FerroBRIDGE release its chart runs beside),
  the release date and the versions of the standards, read from
  `docs/VERSIONS.md` (Art. 38(2)(b) and (e)). The release notes name the
  pinned revisions, and `release-notes.sh` refuses notes that do not.
