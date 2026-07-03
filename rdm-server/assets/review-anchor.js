// Progressive enhancement for select-to-anchor review comments and the
// draft panel. Two behaviors, both requiring an open draft:
//
// 1. Selection gesture: highlighting text inside an annotated body
//    (.body-content[data-rdm-annotated], whose text runs carry
//    span.rdm-src[data-so][data-se] source-offset annotations) pops an
//    "Add review comment" affordance. Submitting it POSTs the mapped
//    source byte range plus the selected text to the draft's
//    /form/comments/anchor endpoint; the server re-validates the mapping
//    and either stores a text-quote anchor or degrades to a general
//    comment — a wrong anchor is never stored.
// 2. Panel updates without reloads: the draft panel's add/edit/remove
//    forms are intercepted and POSTed with Accept: application/json; the
//    response carries the re-rendered panel fragment, which is swapped in
//    place. Start/submit/delete keep full navigation.
//
// Without JavaScript everything degrades to the plain-form flow.
//
// Testing note: the selection->offset byte arithmetic below is
// intentionally not unit-tested client-side (this repo has no JS test
// harness). The server's guard ladder (rdm-server/src/selection.rs) fails
// closed: a wrong mapping produces a rendered-text mismatch and degrades
// to a general comment -- it can never store a wrong anchor. The mapping
// contract itself is pinned by the server-side round-trip matrix.
(function () {
  "use strict";

  var encoder = typeof TextEncoder !== "undefined" ? new TextEncoder() : null;

  function byteLen(s) {
    return encoder ? encoder.encode(s).length : null;
  }

  // ---- panel swap ----

  function panel() {
    return document.getElementById("review-draft");
  }

  function swapPanel(html) {
    var current = panel();
    if (!current) return;
    var tpl = document.createElement("template");
    tpl.innerHTML = html.trim();
    var next = tpl.content.getElementById("review-draft");
    if (next) current.replaceWith(next);
  }

  function showPanelError(form, msg) {
    var out = form.querySelector("[data-rdm-error]");
    if (!out) {
      out = document.createElement("output");
      out.setAttribute("role", "alert");
      out.setAttribute("data-rdm-error", "");
      form.appendChild(out);
    }
    out.textContent = msg;
  }

  function postForm(action, params, onOk, onErr) {
    fetch(action, {
      method: "POST",
      headers: {
        "Content-Type": "application/x-www-form-urlencoded",
        "Accept": "application/json",
      },
      body: params.toString(),
    }).then(function (res) {
      return res.json().then(
        function (body) {
          if (res.ok && body.panel_html) onOk(body);
          else onErr(body.error || "Error " + res.status);
        },
        function () { onErr("Error " + res.status); }
      );
    }).catch(function (err) {
      onErr((err && err.message) || "Network error");
    });
  }

  // Delegated so handlers survive panel swaps. Only the comment
  // add/edit/remove forms are enhanced; start, submit, and delete keep
  // full navigation (they change page state materially).
  document.addEventListener("submit", function (event) {
    var form = event.target;
    var root = panel();
    if (!root || !root.contains(form)) return;
    var enhanced =
      form.classList.contains("draft-add-comment") ||
      form.classList.contains("draft-comment-edit") ||
      form.classList.contains("draft-comment-remove");
    if (!enhanced) return;
    event.preventDefault();
    // In-flight latch: a double-submit before the response lands would
    // create a duplicate comment. On success the panel swap replaces the
    // form (latch and all); on error the form is re-enabled.
    if (form.dataset.rdmBusy) return;
    form.dataset.rdmBusy = "1";
    setSubmitDisabled(form, true);
    var params = new URLSearchParams(new FormData(form));
    postForm(form.action, params, function (body) {
      swapPanel(body.panel_html);
    }, function (msg) {
      delete form.dataset.rdmBusy;
      setSubmitDisabled(form, false);
      showPanelError(form, msg);
    });
  });

  function setSubmitDisabled(form, disabled) {
    form.querySelectorAll("button").forEach(function (b) {
      if (b.type !== "button") b.disabled = disabled;
    });
  }

  // ---- selection mapping ----

  // The nearest annotated body container fully containing `node`.
  function containerOf(node) {
    var el = node.nodeType === 1 ? node : node.parentElement;
    return el ? el.closest(".body-content[data-rdm-annotated]") : null;
  }

  // The nearest run span (span.rdm-src[data-so]) containing `node`.
  function runOf(node) {
    var el = node.nodeType === 1 ? node : node.parentElement;
    return el ? el.closest("span.rdm-src[data-so][data-se]") : null;
  }

  // Maps one selection endpoint to a source byte offset. `edge` is
  // "start" or "end". Opaque runs (rendered text byte length differs from
  // the source range — inline code, entities) snap to the run boundary.
  function mapEndpoint(node, offset, edge) {
    var run = runOf(node);
    if (!run) return null;
    var so = parseInt(run.getAttribute("data-so"), 10);
    var se = parseInt(run.getAttribute("data-se"), 10);
    if (isNaN(so) || isNaN(se)) return null;
    var text = run.textContent;
    var clean = byteLen(text) === se - so;
    if (!clean) return edge === "start" ? so : se;
    if (node.nodeType !== 3) return edge === "start" ? so : se;
    // Byte offset of the caret within this run's (possibly multi-node)
    // text content: bytes of preceding sibling text plus the clipped node.
    var before = 0;
    var walker = document.createTreeWalker(run, NodeFilter.SHOW_TEXT);
    var t;
    while ((t = walker.nextNode())) {
      if (t === node) {
        before += byteLen(t.textContent.slice(0, offset));
        return so + before;
      }
      before += byteLen(t.textContent);
    }
    return null;
  }

  // Maps the current selection to {container, start, end, text}, or null.
  function mapSelection() {
    var sel = window.getSelection();
    if (!sel || sel.isCollapsed || sel.rangeCount === 0 || !encoder) return null;
    var range = sel.getRangeAt(0);
    var startContainer = containerOf(range.startContainer);
    var endContainer = containerOf(range.endContainer);
    // Both endpoints must live in the SAME annotated body (offsets are
    // per-document); anything else suppresses the affordance.
    if (!startContainer || startContainer !== endContainer) return null;
    var start = mapEndpoint(range.startContainer, range.startOffset, "start");
    var end = mapEndpoint(range.endContainer, range.endOffset, "end");
    if (start === null || end === null || start >= end) return null;
    // The visible text of the mapped range: clipped run text in document
    // order (matches the server's reconstruction).
    var text = collectText(startContainer, range);
    if (!text) return null;
    return { container: startContainer, start: start, end: end, text: text, rect: range.getBoundingClientRect() };
  }

  function collectText(container, range) {
    var out = "";
    var seenOpaque = [];
    var walker = document.createTreeWalker(container, NodeFilter.SHOW_TEXT);
    var t;
    while ((t = walker.nextNode())) {
      var run = runOf(t);
      if (!run) continue; // only annotated runs count
      if (!range.intersectsNode(t)) continue;
      var so = parseInt(run.getAttribute("data-so"), 10);
      var se = parseInt(run.getAttribute("data-se"), 10);
      if (byteLen(run.textContent) !== se - so) {
        // Opaque run (inline code, entities): the offsets snap to the
        // WHOLE run, so the text must be the run's full content too -- a
        // clipped slice would fail the server's cross-check and degrade
        // the anchor needlessly.
        if (seenOpaque.indexOf(run) === -1) {
          seenOpaque.push(run);
          out += run.textContent;
        }
        continue;
      }
      var s = 0;
      var e = t.textContent.length;
      if (t === range.startContainer) s = range.startOffset;
      if (t === range.endContainer) e = range.endOffset;
      if (s < e) out += t.textContent.slice(s, e);
    }
    return out;
  }

  // ---- affordance UI ----

  var affordance = null;
  var composer = null;
  var pending = null;

  function hideAffordance() {
    if (affordance) affordance.remove();
    affordance = null;
  }

  function hideComposer() {
    if (composer) composer.remove();
    composer = null;
    pending = null;
  }

  function anchorAction() {
    var root = panel();
    return root ? root.getAttribute("data-rdm-anchor-action") : null;
  }

  function showAffordance(mapped) {
    hideAffordance();
    var btn = document.createElement("button");
    btn.type = "button";
    btn.className = "rdm-anchor-affordance";
    btn.textContent = "Add review comment";
    btn.setAttribute("aria-label", "Add review comment on the selected text");
    btn.style.top = window.scrollY + mapped.rect.bottom + 6 + "px";
    btn.style.left = window.scrollX + mapped.rect.left + "px";
    btn.addEventListener("mousedown", function (event) {
      // Fires before the click collapses the selection.
      event.preventDefault();
      showComposer(mapped);
    });
    document.body.appendChild(btn);
    affordance = btn;
  }

  function showComposer(mapped) {
    hideAffordance();
    hideComposer();
    pending = mapped;
    var box = document.createElement("form");
    box.className = "rdm-anchor-form";
    box.style.top = window.scrollY + mapped.rect.bottom + 6 + "px";
    box.style.left = window.scrollX + mapped.rect.left + "px";
    var quote = document.createElement("blockquote");
    quote.className = "rdm-anchor-form-quote";
    quote.textContent = mapped.text;
    var label = document.createElement("label");
    label.className = "visually-hidden";
    label.setAttribute("for", "rdm-anchor-comment");
    label.textContent = "Review comment (markdown)";
    var ta = document.createElement("textarea");
    ta.id = "rdm-anchor-comment";
    ta.rows = 3;
    var add = document.createElement("button");
    add.type = "submit";
    add.textContent = "Add comment";
    var cancel = document.createElement("button");
    cancel.type = "button";
    cancel.textContent = "Cancel";
    cancel.addEventListener("click", hideComposer);
    box.appendChild(quote);
    box.appendChild(label);
    box.appendChild(ta);
    box.appendChild(add);
    box.appendChild(cancel);
    box.addEventListener("submit", function (event) {
      event.preventDefault();
      submitAnchor(box, ta.value);
    });
    document.body.appendChild(box);
    composer = box;
    ta.focus();
  }

  function submitAnchor(box, comment) {
    var action = anchorAction();
    if (!action || !pending) return;
    // Same in-flight latch as the panel forms: the composer survives
    // until the response, so a double-submit would duplicate the comment.
    if (box.dataset.rdmBusy) return;
    box.dataset.rdmBusy = "1";
    setSubmitDisabled(box, true);
    var params = new URLSearchParams();
    params.set("body", comment);
    params.set("doc_stem", pending.container.getAttribute("data-rdm-doc") || "");
    params.set("sel_start", String(pending.start));
    params.set("sel_end", String(pending.end));
    params.set("rendered_text", pending.text);
    postForm(action, params, function (body) {
      hideComposer();
      swapPanel(body.panel_html);
      if (body.outcome === "fallback") {
        var root = panel();
        if (root) {
          var note = document.createElement("p");
          note.className = "rdm-no-anchor-note";
          note.setAttribute("role", "status");
          note.textContent =
            "No anchor attached — the selection could not be mapped, so it was added as a general comment.";
          root.insertBefore(note, root.firstChild);
        }
      }
    }, function (msg) {
      delete box.dataset.rdmBusy;
      setSubmitDisabled(box, false);
      showPanelError(box, msg);
    });
  }

  document.addEventListener("mouseup", function (event) {
    if (affordance && affordance.contains(event.target)) return;
    if (composer && composer.contains(event.target)) return;
    // Let the selection settle before reading it.
    window.setTimeout(function () {
      if (!anchorAction()) return;
      var mapped = mapSelection();
      if (mapped) showAffordance(mapped);
      else hideAffordance();
    }, 0);
  });

  document.addEventListener("keydown", function (event) {
    if (event.key === "Escape") {
      hideAffordance();
      hideComposer();
    }
  });
})();
