// FerroEHR docs — version picker + API-reference link.
// Additive JS (additional-js), NOT an index.hbs fork — survives mdBook theme churn.
// Fetches versions.json, builds a <select> pre-selected from the current path, and
// injects it plus an "API Reference ↗" link into the mdBook top menu bar.
(function () {
  "use strict";

  // The site base is DERIVED from the page's own location, never hardcoded:
  // every book page lives under `<base>/docs/<version>/`, so whatever precedes
  // `/docs/` IS the base. That is "" at a domain root (ferroehr.eu) and
  // "/<repo>" under a GitHub Pages project path — a hardcoded value goes stale
  // the moment the site moves, which silently 404s the manifest and leaves the
  // picker with nothing to offer.
  var SITE_BASE = (function () {
    var marker = window.location.pathname.indexOf("/docs/");
    return marker === -1 ? "" : window.location.pathname.slice(0, marker);
  })();
  var MANIFEST = SITE_BASE + "/versions.json";
  var API_HREF = SITE_BASE + "/api/";

  function currentVersionId(versions) {
    var path = window.location.pathname;
    // Longest matching path wins (so /docs/latest/ isn't shadowed by /docs/).
    var best = null;
    versions.forEach(function (v) {
      if (v.path && path.indexOf(v.path) === 0) {
        if (!best || v.path.length > best.path.length) best = v;
      }
    });
    return best ? best.id : null;
  }

  function inject(manifest) {
    var bar =
      document.querySelector(".right-buttons") ||
      document.querySelector(".menu-bar");
    if (!bar) return;

    var versions = (manifest && manifest.versions) || [];
    if (!versions.length) {
      // Before the first tag is cut, only "dev" exists; latest resolves to dev.
      versions = [{ id: "dev", label: "dev (main)", path: SITE_BASE + "/docs/dev/" }];
    }

    var select = document.createElement("select");
    select.id = "rs-version-picker";
    select.setAttribute("aria-label", "Documentation version");
    var current = currentVersionId(versions);
    versions.forEach(function (v) {
      var opt = document.createElement("option");
      opt.value = v.path;
      opt.textContent = v.label || v.id;
      if (v.id === current) opt.selected = true;
      select.appendChild(opt);
    });
    select.addEventListener("change", function () {
      if (select.value) window.location.href = select.value;
    });

    var api = document.createElement("a");
    api.id = "rs-api-link";
    api.href = API_HREF;
    api.textContent = "API Reference ↗";
    api.title = "OpenAPI endpoint reference";

    bar.insertBefore(select, bar.firstChild);
    bar.insertBefore(api, bar.firstChild);
  }

  function start() {
    fetch(MANIFEST)
      .then(function (r) { return r.ok ? r.json() : null; })
      .then(inject)
      .catch(function () { inject(null); });
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start);
  } else {
    start();
  }
})();
