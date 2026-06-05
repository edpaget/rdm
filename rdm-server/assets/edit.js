(function () {
  "use strict";

  function getOrCreateError(form) {
    var out = form.querySelector("[data-rdm-error]");
    if (!out) {
      out = document.createElement("output");
      out.setAttribute("role", "alert");
      out.setAttribute("data-rdm-error", "");
      form.appendChild(out);
    }
    return out;
  }

  function showError(form, msg) { getOrCreateError(form).textContent = msg; }
  function clearError(form) {
    var out = form.querySelector("[data-rdm-error]");
    if (out) out.textContent = "";
  }

  function isClearTagsSubmitter(el) {
    return !!(el && el.name === "clear_tags" && el.value === "true");
  }

  function buildPayload(form, submitter) {
    var payload = {};
    var clearTags = isClearTagsSubmitter(submitter);
    var statusEl = form.elements.namedItem("status");
    if (statusEl && statusEl.value !== "") payload.status = statusEl.value;
    var bodyEl = form.elements.namedItem("body");
    if (bodyEl) {
      if (bodyEl.value === "") {
        // Server refuses `body:""` against a non-empty body; translate an
        // empty textarea into an explicit clear so the web UI can still
        // empty a body in one save.
        payload.clear_body = true;
      } else {
        payload.body = bodyEl.value;
      }
    }
    if (clearTags) {
      payload.clear_tags = true;
    } else {
      var tagsEl = form.elements.namedItem("tags");
      if (tagsEl && tagsEl.value !== "") {
        var arr = tagsEl.value.split(",")
          .map(function (t) { return t.trim(); })
          .filter(function (t) { return t.length > 0; });
        var seen = {};
        payload.tags = arr.filter(function (t) {
          if (Object.prototype.hasOwnProperty.call(seen, t)) return false;
          seen[t] = true;
          return true;
        });
      }
    }
    return payload;
  }

  function handleSubmit(event) {
    event.preventDefault();
    var form = event.currentTarget;
    var submitter = event.submitter || document.activeElement;
    var payload = buildPayload(form, submitter);
    // Skip the network round-trip when the user submitted with every field blank.
    if (Object.keys(payload).length === 0) return;
    clearError(form);
    fetch(form.action, {
      method: form.dataset.rdmMethod || "PATCH",
      headers: {
        "Content-Type": "application/json",
        "Accept": "application/hal+json",
      },
      body: JSON.stringify(payload),
    }).then(function (res) {
      if (res.ok) { window.location.reload(); return; }
      return res.json().then(
        function (b) { showError(form, b.detail || b.title || "Error " + res.status); },
        function () { showError(form, "Error " + res.status); }
      );
    }).catch(function (err) {
      showError(form, (err && err.message) || "Network error");
    });
  }

  document.addEventListener("DOMContentLoaded", function () {
    document.querySelectorAll("form[data-rdm-edit]").forEach(function (form) {
      form.addEventListener("submit", handleSubmit);
    });
  });
})();
