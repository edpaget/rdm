// Progressive enhancement for review-comment anchor highlights: hovering or
// focusing a comment's quote preview (blockquote[data-rdm-anchor-ref]) lights
// up the matching inline <mark class="rdm-anchor" data-rdm-anchor="..."> in
// the rendered body. Without this script the page stays fully readable —
// marks render visually inert and the quote preview carries the anchor.
(function () {
  "use strict";

  function cssEscape(s) {
    return window.CSS && CSS.escape
      ? CSS.escape(s)
      : s.replace(/[^a-zA-Z0-9_-]/g, "\\$&");
  }

  function setActive(ref, on) {
    document
      .querySelectorAll('mark.rdm-anchor[data-rdm-anchor="' + cssEscape(ref) + '"]')
      .forEach(function (mark) {
        mark.classList.toggle("is-active", on);
      });
  }

  document.addEventListener("DOMContentLoaded", function () {
    document.querySelectorAll("[data-rdm-anchor-ref]").forEach(function (el) {
      var ref = el.getAttribute("data-rdm-anchor-ref");
      ["mouseenter", "focus"].forEach(function (ev) {
        el.addEventListener(ev, function () { setActive(ref, true); });
      });
      ["mouseleave", "blur"].forEach(function (ev) {
        el.addEventListener(ev, function () { setActive(ref, false); });
      });
    });
  });
})();
