;;; rdm-review.el --- Review overlays, the requests queue, resolution and authoring  -*- lexical-binding: t; -*-

;; This file is not part of GNU Emacs.

;;; Commentary:

;; rdm's document reviews, layered over the detail buffers of rdm-view.el:
;; the reviewee side (overlays, requests, resolution) and the reviewer
;; side (comments anchored to selections, verdicts).
;;
;; - After every render of a roadmap, phase or task, `rdm review list
;;   --on <target>' is read and each shown review's comments are laid over
;;   the body (`rdm-view-body-region'): an anchored comment becomes an
;;   overlay with a fringe marker (a margin marker in a terminal), faced
;;   `rdm-review-resolved' or `rdm-review-drifted', plus
;;   `rdm-review-closed' once addressed or won't-fixed.  A trailing
;;   Reviews section lists every shown review and every comment that is
;;   not overlaid: whole-document comments, a roadmap review's comments
;;   on one of its phases, and comments whose anchor could not be placed.
;;   `rdm-review-shown-states' picks the review states shown.
;; - Placement follows each comment's `resolution' (see
;;   `rdm-review-place'): a `current' range indexes the displayed body; an
;;   `original' range indexes the body at the review's `created_commit',
;;   read once per commit with `show --at'.  When that body is the one
;;   displayed the range applies as is, otherwise the quote is searched
;;   for in the displayed body.  Ranges are UTF-8 byte offsets
;;   (`rdm-review-byte-to-char').
;; - `rdm-review-comment-at-point' describes the comment under point,
;;   `rdm-review-next-comment' / `rdm-review-previous-comment' move between
;;   overlaid comments, `rdm-review-resolve' marks the comment at point
;;   addressed or won't-fix (the applied commit defaults to the plan
;;   repo's HEAD), `rdm-review-close' / `rdm-review-dismiss' close a
;;   review and `rdm-review-show' displays one.
;; - `rdm-review-requests' picks from `rdm review requests' and opens the
;;   target with its overlays.
;; - `rdm-review-revise' hands a review to the revision loop by calling
;;   `rdm-review-revise-function' (default: copy the command); the
;;   package never starts an agent.
;;
;; - Authoring: `rdm-review-comment-region' turns the selected body text
;;   into an anchored comment (`--quote' is the text; when it occurs more
;;   than once in the body, `--occurrence' is its index among the
;;   non-overlapping occurrences, as `rdm review comment --help'
;;   documents), `rdm-review-comment-document' files a whole-document
;;   comment, and `rdm-review-submit' submits with a verdict read from
;;   `rdm describe review'.  Comments go into your draft review -- the
;;   newest draft rdm lists as yours (`rdm review list --mine', or
;;   `--author' when `rdm-review-author' is set) -- which
;;   `rdm-review-start' or the first comment starts.  A detail buffer of an item with your draft is shown as of
;;   the draft's `created_commit', the text rdm locates quotes in, and its
;;   header line says so; a selection made on other text is discarded.
;;
;; Keys in a detail buffer: `K' describes the comment at point, `M-n' /
;; `M-p' move between comments, `A' resolves one, `c' comments on the
;; region, `C' on the whole document and `S' submits the draft (evil
;; bindings are in rdm-evil.el); the *Review* group of `rdm-dispatch'
;; offers show, requests, close, dismiss, revise, start, comment and
;; submit.

;;; Code:

(require 'cl-lib)
(require 'seq)
(require 'subr-x)
(require 'rdm-core)
(require 'rdm-view)
(require 'rdm-transient)
(require 'rdm-compose)
(require 'rdm-pick)

(defgroup rdm-review nil
  "Document reviews in rdm detail buffers."
  :group 'rdm
  :prefix "rdm-review-")

(defcustom rdm-review-in-views t
  "Whether rdm detail buffers show the item's reviews."
  :type 'boolean)

(defcustom rdm-review-shown-states '("draft" "submitted")
  "The review states whose reviews a detail buffer shows.
Reviews in other states (addressed, dismissed) are neither overlaid
nor listed."
  :type '(repeat string))

(defcustom rdm-review-revise-command-format "/rdm:revise %s"
  "Format of the revision command `rdm-review-revise' produces.
It receives the review id.  The plugin form is \"/rdm:revise %s\"; in
the rdm source repo the dogfood skill is \"/rdm-revise %s\"."
  :type 'string)

(defcustom rdm-review-revise-function #'rdm-review-revise-copy
  "Function `rdm-review-revise' calls with one plist argument.
The plist has `:review-id', `:target' (the target reference, such as
task/SLUG), `:project' and `:command' (see
`rdm-review-revise-command-format').  The default copies the command
to the kill ring.  Set it to hand the command to an agent session."
  :type 'function)

(defcustom rdm-review-author nil
  "The author the package reviews as, or nil for rdm's own resolution.
nil leaves the author to rdm: a draft is started without `--author'
and your drafts are listed with `rdm review list --mine', both of which
resolve it the way `rdm review start --help' documents.  A string
overrides both: it is passed as `--author' when a draft is started and
when the item's drafts are listed (in place of `--mine')."
  :type '(choice (const :tag "rdm's own resolution" nil) string))

(defface rdm-review-resolved
  '((((background dark)) :background "#3a3a18" :extend nil)
    (t :background "#fff3b0" :extend nil))
  "Face for a comment whose anchor resolved in the displayed body."
  :group 'rdm-review)

(defface rdm-review-drifted
  '((t :inherit warning :underline (:style wave)))
  "Face for a comment whose anchored text changed since the review."
  :group 'rdm-review)

(defface rdm-review-closed
  '((t :inherit shadow :strike-through nil :slant italic))
  "Face added to a comment marked addressed or won't-fix."
  :group 'rdm-review)

(defface rdm-review-unplaced
  '((t :inherit error))
  "Face for the label of a comment that could not be placed."
  :group 'rdm-review)

(defface rdm-review-heading
  '((t :inherit font-lock-keyword-face))
  "Face for the Reviews section heading."
  :group 'rdm-review)

;;;; Targets

(defun rdm-review-item-target (item)
  "The review target reference of ITEM, as `rdm review --on' takes it."
  (pcase (plist-get item :kind)
    ('roadmap (concat "roadmap/" (plist-get item :roadmap)))
    ('phase (format "phase/%s/%s" (plist-get item :roadmap)
                    (plist-get item :stem)))
    ('task (concat "task/" (plist-get item :slug)))
    (kind (user-error "rdm: a %s has no reviews" kind))))

(defun rdm-review-target-item (target)
  "The item plist for TARGET, a review's target JSON, or nil.
Only roadmap, phase and task targets have an item; plan and change
targets do not."
  (pcase (plist-get target :kind)
    ("roadmap" (list :kind 'roadmap :roadmap (plist-get target :roadmap)))
    ("phase" (list :kind 'phase :roadmap (plist-get target :roadmap)
                   :stem (plist-get target :stem)))
    ("task" (list :kind 'task :slug (plist-get target :slug)))))

(defun rdm-review--ref (review)
  "REVIEW's target reference, as rdm reports it (`target.ref')."
  (plist-get (plist-get review :target) :ref))

;;;; Offsets and placement

(defun rdm-review-byte-to-char (string byte)
  "The character offset in STRING of the UTF-8 byte offset BYTE, or nil.
STRING is encoded to UTF-8 and its first BYTE bytes decoded, so the
result does not depend on the buffer's internal representation.  nil
when BYTE is not a natural number, lies past STRING's end or falls
inside a multibyte character."
  (let ((bytes (encode-coding-string string 'utf-8 t)))
    (when (and (natnump byte)
               (<= byte (length bytes))
               (or (= byte (length bytes))
                   (/= (logand (aref bytes byte) #xC0) #x80)))
      (length (decode-coding-string (substring bytes 0 byte) 'utf-8 t)))))

(defun rdm-review--char-range (body start end)
  "The byte range START..END of BODY as a non-empty (START . END) in chars."
  (let ((s (rdm-review-byte-to-char body start))
        (e (rdm-review-byte-to-char body end)))
    (and s e (< s e) (cons s e))))

(defun rdm-review--occurrences (body quoted)
  "The character offsets of every occurrence of QUOTED in BODY."
  (let ((start 0) found pos)
    (unless (string-empty-p quoted)
      (while (setq pos (string-search quoted body start))
        (push pos found)
        (setq start (1+ pos))))
    (nreverse found)))

(defun rdm-review--choose (body quoted anchor near occurrences)
  "Choose one of OCCURRENCES of QUOTED in BODY for ANCHOR.
Occurrences whose surrounding text matches ANCHOR's `:prefix' and
`:suffix' are preferred; among the candidates the one nearest NEAR (the
original character offset, when known) wins, the earliest on a tie."
  (let* ((prefix (plist-get anchor :prefix))
         (suffix (plist-get anchor :suffix))
         (context
          (seq-filter
           (lambda (pos)
             (and (or (not (rdm--nonempty prefix))
                      (string-suffix-p prefix (substring body 0 pos)))
                  (or (not (rdm--nonempty suffix))
                      (string-prefix-p suffix
                                       (substring body (+ pos (length quoted)))))))
           occurrences))
         (pool (or context occurrences))
         best)
    (dolist (pos pool)
      (when (or (null best)
                (and near (< (abs (- pos near)) (abs (- best near)))))
        (setq best pos)))
    best))

(defun rdm-review-place (comment displayed original)
  "Where COMMENT's anchor lies in DISPLAYED, the body the buffer shows.
ORIGINAL is the body at the review's `created_commit', or nil when it
is unknown.  Returns a plist: `:placement' is `resolved' or `drifted'
with `:start' and `:end' (character offsets into DISPLAYED) for a
placed comment, else `whole' (no anchor), `doc' (a roadmap review's
comment on one of its phases, whose range indexes the phase body) or
`lost' (the anchor could not be placed).

A `current' range indexes DISPLAYED.  An `original' range (resolved or
drifted) indexes ORIGINAL: when ORIGINAL equals DISPLAYED it applies
directly, otherwise the quote is searched for in DISPLAYED and, when it
occurs more than once, the occurrence is chosen by the anchor's
prefix/suffix context, then by nearness to the original offset (see
`rdm-review--choose').  A malformed range degrades to `lost'."
  (let* ((anchor (plist-get comment :anchor))
         (res (plist-get comment :resolution))
         (state (plist-get res :state))
         (rs (plist-get res :range_start))
         (re (plist-get res :range_end)))
    (cond
     ((null anchor) (list :placement 'whole))
     ((plist-get comment :doc) (list :placement 'doc))
     ((not (member state '("resolved" "drifted"))) (list :placement 'lost))
     (t
      (let ((range
             (if (or (equal (plist-get res :body) "current")
                     (and original (equal original displayed)))
                 (rdm-review--char-range displayed rs re)
               (let* ((quoted (or (plist-get res :quote)
                                  (plist-get anchor :quote) ""))
                      (near (car (and original
                                      (rdm-review--char-range original rs re))))
                      (occ (rdm-review--occurrences displayed quoted))
                      (pos (and occ (rdm-review--choose displayed quoted anchor
                                                        near occ))))
                 (and pos (cons pos (+ pos (length quoted))))))))
        (if range
            (list :placement (if (equal state "drifted") 'drifted 'resolved)
                  :start (car range) :end (cdr range))
          (list :placement 'lost)))))))

;;;; The layer

(defvar-local rdm-review--reviews nil
  "The reviews this detail buffer shows, as `rdm review list' JSON.")

(defvar-local rdm-review--entries nil
  "One plist per shown comment.
Its keys are `:review', `:comment', `:placement' and, when overlaid,
`:overlay'.")

(defvar-local rdm-review--section-start nil
  "Marker at the start of the Reviews section, or nil.")

(defun rdm-review--closed-p (comment)
  "Non-nil when rdm reports COMMENT closed (its `closed' is true)."
  (eq (plist-get comment :closed) t))

(defun rdm-review--open-count (review)
  "The number of REVIEW's comments still open."
  (seq-count (lambda (c) (not (rdm-review--closed-p c)))
             (plist-get review :comments)))

(defun rdm-review--needs-original-p (review)
  "Non-nil when some comment of REVIEW indexes the `created_commit' body."
  (seq-some (lambda (c)
              (let ((res (plist-get c :resolution)))
                (and (plist-get c :anchor)
                     (not (plist-get c :doc))
                     (member (plist-get res :state) '("resolved" "drifted"))
                     (not (equal (plist-get res :body) "current")))))
            (plist-get review :comments)))

(defun rdm-review--original-bodies (item reviews)
  "A table from `created_commit' to ITEM's body then, for REVIEWS.
Each commit is read once; a failed read maps to nil."
  (let ((bodies (make-hash-table :test #'equal)))
    (dolist (review reviews)
      (let ((commit (plist-get review :created_commit)))
        (when (and (rdm--nonempty commit)
                   (eq (gethash commit bodies 'unset) 'unset)
                   (rdm-review--needs-original-p review))
          (puthash commit
                   (condition-case nil
                       (or (plist-get (apply #'rdm-call-sync
                                             (rdm-item-show-args item commit))
                                      :body)
                           "")
                     (error nil))
                   bodies))))
    bodies))

(defun rdm-review--marker-spec (face)
  "A `before-string' marking a comment in FACE.
The marker goes in the fringe, or the margin in a terminal."
  (if (display-graphic-p)
      (propertize " " 'display `(left-fringe vertical-bar ,face))
    (setq-local left-margin-width 1)
    (propertize " " 'display `((margin left-margin)
                               ,(propertize "▌" 'face face)))))

(defun rdm-review--face (entry)
  "The overlay face of the placed ENTRY."
  (let ((state (if (eq (plist-get entry :placement) 'drifted)
                   'rdm-review-drifted
                 'rdm-review-resolved)))
    (if (rdm-review--closed-p (plist-get entry :comment))
        (list 'rdm-review-closed state)
      state)))

(defun rdm-review--overlay (entry start end)
  "Overlay ENTRY on START..END; return the overlay."
  (let ((ov (make-overlay start end nil t nil))
        (face (rdm-review--face entry)))
    (overlay-put ov 'rdm-review t)
    (overlay-put ov 'rdm-review-entry entry)
    (overlay-put ov 'face face)
    (overlay-put ov 'priority 10)
    (overlay-put ov 'help-echo (plist-get (plist-get entry :comment) :body))
    (overlay-put ov 'before-string
                 (rdm-review--marker-spec (if (consp face) (cadr face) face)))
    ov))

(defun rdm-review--first-line (text)
  "The first line of TEXT, or the empty string."
  (car (split-string (or text "") "\n")))

(defun rdm-review--label (entry)
  "The Reviews-section label saying why ENTRY is not overlaid."
  (let ((comment (plist-get entry :comment)))
    (pcase (plist-get entry :placement)
      ('whole "whole document")
      ('doc (format "on phase %s"
                    (or (plist-get (plist-get comment :doc) :stem) "?")))
      (_ (propertize
          (format "un-placed (%s) \"%s\""
                  (or (plist-get (plist-get comment :resolution) :state) "?")
                  (or (plist-get (plist-get comment :anchor) :quote) ""))
          'font-lock-face 'rdm-review-unplaced)))))

(defun rdm-review--review-line (review)
  "The Reviews-section summary line of REVIEW."
  (format "%s  %s  %s%s  %d/%d open"
          (plist-get review :id)
          (or (plist-get review :author) "?")
          (or (plist-get review :state) "?")
          (if (rdm--nonempty (plist-get review :verdict))
              (concat " " (plist-get review :verdict))
            "")
          (rdm-review--open-count review)
          (length (plist-get review :comments))))

(defun rdm-review--insert-section ()
  "Insert the Reviews section at the end of the buffer."
  (goto-char (point-max))
  (setq rdm-review--section-start (copy-marker (point)))
  (insert "\n" (propertize "Reviews" 'font-lock-face 'rdm-review-heading) "\n")
  (dolist (review rdm-review--reviews)
    (insert (propertize (concat "  " (rdm-review--review-line review))
                        'rdm-review review)
            "\n")
    (dolist (entry rdm-review--entries)
      (when (and (eq (plist-get entry :review) review)
                 (not (plist-get entry :overlay)))
        (let ((comment (plist-get entry :comment)))
          (insert (propertize
                   (format "    #%s %s  %s  %s"
                           (plist-get comment :id)
                           (plist-get comment :status)
                           (rdm-review--label entry)
                           (rdm-review--first-line (plist-get comment :body)))
                   'rdm-review review
                   'rdm-review-entry entry)
                  "\n"))))))

(defun rdm-review--clear (&optional section)
  "Remove this buffer's review overlays; with SECTION, its Reviews section."
  (remove-overlays (point-min) (point-max) 'rdm-review t)
  (when (and section (markerp rdm-review--section-start)
             (marker-buffer rdm-review--section-start))
    (let ((inhibit-read-only t))
      (delete-region rdm-review--section-start (point-max))))
  (setq rdm-review--section-start nil
        rdm-review--entries nil
        rdm-review--reviews nil))

(defun rdm-review--fetch (item)
  "The reviews of ITEM in `rdm-review-shown-states', or nil on error."
  (condition-case err
      (seq-filter (lambda (r) (member (plist-get r :state) rdm-review-shown-states))
                  (append (rdm-call-sync 'review-list
                                         :on (rdm-review-item-target item))
                          nil))
    (error (message "rdm: reviews of %s: %s" (rdm-item-identifier item)
                    (error-message-string err))
           nil)))

(defun rdm-review--apply ()
  "Read, place and draw the reviews of this detail buffer's item."
  (let ((item rdm-view--item)
        (region (rdm-view-body-region)))
    (when (and rdm-review-in-views region
               (memq (plist-get item :kind) '(roadmap phase task)))
      (setq rdm-review--reviews (rdm-review--fetch item))
      (when rdm-review--reviews
        (let* ((displayed (or (rdm-view-body) ""))
               (originals (rdm-review--original-bodies item rdm-review--reviews))
               entries)
          (dolist (review rdm-review--reviews)
            (let ((original (gethash (plist-get review :created_commit) originals)))
              (seq-doseq (comment (plist-get review :comments))
                (let* ((where (rdm-review-place comment displayed original))
                       (entry (list :review review :comment comment
                                    :placement (plist-get where :placement))))
                  (when (plist-get where :start)
                    (plist-put entry :overlay
                               (rdm-review--overlay
                                entry
                                (+ (car region) (plist-get where :start))
                                (+ (car region) (plist-get where :end)))))
                  (push entry entries)))))
          (setq rdm-review--entries (nreverse entries))
          (let ((inhibit-read-only t))
            (save-excursion (rdm-review--insert-section))))))))

(defun rdm-review--after-render ()
  "Lay this detail buffer's reviews over its fresh render.
A current-revision render of an item with an own draft review is
re-rendered at the draft's `created_commit' first (see
`rdm-review--follow-draft')."
  (rdm-review--clear)
  (unless (rdm-review--follow-draft)
    (rdm-review--show-draft)
    (rdm-review--apply)))

(defun rdm-review--setup ()
  "Attach the review layer to this detail buffer's renders."
  (add-hook 'rdm-view-after-render-hook #'rdm-review--after-render nil t))

;; Attached per buffer from the mode hook, so reloading rdm-view.el (which
;; re-initializes `rdm-view-after-render-hook') does not detach it.
(add-hook 'rdm-view-mode-hook #'rdm-review--setup)

(defun rdm-review-refresh ()
  "Re-read and redraw the review layer of this detail buffer.
The item itself is not re-read; `revert-buffer' does both."
  (interactive)
  (unless (derived-mode-p 'rdm-view-mode)
    (user-error "rdm: Not in an rdm detail buffer"))
  (rdm-review--clear t)
  (rdm-review--apply))

;;;; Comments at point

(defun rdm-review-entry-at-point (&optional pos)
  "The review comment at POS (default point): an overlay or a section row."
  (get-char-property (or pos (point)) 'rdm-review-entry))

(defun rdm-review-at-point (&optional pos)
  "The review at POS (default point): its comment's, or a section row's."
  (let ((pos (or pos (point))))
    (or (plist-get (rdm-review-entry-at-point pos) :review)
        (get-text-property pos 'rdm-review))))

(defun rdm-review--require-entry ()
  "The comment at point, or a `user-error'."
  (or (rdm-review-entry-at-point)
      (user-error "rdm: No review comment at point")))

(defun rdm-review-describe (entry)
  "A description of ENTRY's comment: review, author, status, state, reply."
  (let* ((review (plist-get entry :review))
         (comment (plist-get entry :comment))
         (reply (plist-get comment :reply))
         (commit (plist-get comment :applied_commit)))
    (concat
     (format "Review %s #%s by %s — %s, %s\n"
             (plist-get review :id) (plist-get comment :id)
             (or (plist-get review :author) "?")
             (plist-get comment :status)
             (pcase (plist-get entry :placement)
               ('resolved "resolved")
               ('drifted "drifted")
               ('whole "whole document")
               ('doc "on a phase")
               (_ (format "un-placed (%s)"
                          (or (plist-get (plist-get comment :resolution) :state)
                              "?")))))
     (or (plist-get comment :body) "")
     (if (rdm--nonempty reply) (concat "\nReply: " reply) "")
     (if (rdm--nonempty commit) (concat "\nApplied: " commit) ""))))

(defun rdm-review-comment-at-point ()
  "Describe the review comment at point in the echo area; return the text."
  (interactive)
  (let ((text (rdm-review-describe (rdm-review--require-entry))))
    (message "%s" text)
    text))

(defun rdm-review--overlay-starts ()
  "The start positions of this buffer's comment overlays, in order."
  (sort (delq nil (mapcar (lambda (e)
                            (let ((ov (plist-get e :overlay)))
                              (and ov (overlay-buffer ov) (overlay-start ov))))
                          rdm-review--entries))
        #'<))

(defun rdm-review-next-comment ()
  "Move to the next overlaid review comment."
  (interactive)
  (goto-char (or (seq-find (lambda (p) (> p (point))) (rdm-review--overlay-starts))
                 (user-error "rdm: No next review comment"))))

(defun rdm-review-previous-comment ()
  "Move to the previous overlaid review comment."
  (interactive)
  (goto-char (or (car (last (seq-filter (lambda (p) (< p (point)))
                                        (rdm-review--overlay-starts))))
                 (user-error "rdm: No previous review comment"))))

;;;; Writes

(defun rdm-review--after-write (review what)
  "Refresh the views of REVIEW's target, report WHAT, run the write hook.
REVIEW is the review JSON rdm returned from the write."
  (let ((item (rdm-review-target-item (plist-get review :target))))
    (if item
        (rdm--after-write item what)
      (message "rdm: review %s %s staged" (plist-get review :id) what)
      (run-hooks 'rdm-after-write-hook))
    (when (and (derived-mode-p 'rdm-view-mode) rdm-view--at)
      (rdm-review-refresh))))

(defun rdm-review--read-resolution ()
  "Read the arguments of `rdm-review-resolve' for the comment at point."
  (let* ((entry (rdm-review--require-entry))
         (comment (plist-get entry :comment))
         (status (completing-read
                  (format "Mark comment #%s: " (plist-get comment :id))
                  (or (rdm-enum-values 'review "comments.status")
                      (user-error "rdm: `rdm describe review' declares no comment statuses"))
                  nil t))
         (reply (read-string "Reply (empty for none): "))
         (commit (and (equal status "addressed")
                      (read-string "Applied commit (empty for none): "
                                   "HEAD"))))
    (list entry status reply commit)))

(defun rdm-review-resolve (entry status &optional reply commit)
  "Mark ENTRY's comment STATUS, one of the comment statuses rdm declares.
REPLY and COMMIT, when non-empty, are recorded as the reply and the
applied commit.  Interactively, ENTRY is the comment at point, STATUS
is read from `rdm describe review' and the applied commit is pre-filled
with HEAD, which rdm resolves in the target's repository: the source
repo for a `change' review, the plan repo otherwise.  COMMIT is passed
through unchanged.  Returns the updated review's JSON; a refusal
signals rdm's error."
  (interactive (rdm-review--read-resolution))
  (let* ((comment (plist-get entry :comment))
         (updated (rdm-call-sync 'review-update
                                 (plist-get (plist-get entry :review) :id)
                                 :comment (plist-get comment :id)
                                 :status status
                                 :reply (rdm--nonempty reply)
                                 :applied-commit (rdm--nonempty commit)
                                 :no-edit t)))
    (rdm-review--after-write
     updated (format "comment #%s → %s" (plist-get comment :id) status))
    updated))

(defun rdm-review--shows-p (item)
  "Non-nil when this buffer is a detail buffer showing ITEM."
  (and (derived-mode-p 'rdm-view-mode)
       rdm-view--item
       (eq (plist-get rdm-view--item :kind) (plist-get item :kind))
       (equal (rdm-item-identifier rdm-view--item) (rdm-item-identifier item))))

(defun rdm-review--reviews-of (item)
  "The shown reviews of ITEM: this buffer's when it shows ITEM, else read."
  (if (rdm-review--shows-p item)
      rdm-review--reviews
    (rdm-review--fetch item)))

(defun rdm-review-read-review (&optional item)
  "The review at point, else one of ITEM's reviews, picked if several.
ITEM defaults to this detail buffer's item; the review at point counts
only when this buffer shows ITEM."
  (let ((item (or item (and (derived-mode-p 'rdm-view-mode) rdm-view--item)
                  (user-error "rdm: No item to pick a review of"))))
    (or (and (rdm-review--shows-p item) (rdm-review-at-point))
        (let* ((reviews (or (rdm-review--reviews-of item)
                            (user-error "rdm: %s has no open reviews"
                                        (rdm-item-identifier item))))
               (table (mapcar (lambda (r) (cons (rdm-review--review-line r) r))
                              reviews)))
          (if (null (cdr table))
              (cdar table)
            (cdr (assoc (completing-read "Review: " table nil t) table)))))))

(defun rdm-review--set-state (review state)
  "Move REVIEW to STATE with `rdm review update --state'.
Returns the updated review's JSON."
  (let ((updated (rdm-call-sync 'review-update (plist-get review :id)
                                :state state :no-edit t)))
    (rdm-review--after-write updated (concat "→ " (plist-get updated :state)))
    updated))

(defun rdm-review-close (review)
  "Close REVIEW as addressed; return the updated review's JSON.
rdm refuses while any comment is still open, and says how many."
  (interactive (list (rdm-review-read-review)))
  (rdm-review--set-state review "addressed"))

(defun rdm-review-dismiss (review)
  "Dismiss REVIEW after confirming.
Return the updated review's JSON, or nil if declined."
  (interactive (list (rdm-review-read-review)))
  (when (y-or-n-p (format "Dismiss review %s? " (plist-get review :id)))
    (rdm-review--set-state review "dismissed")))

;;;; Showing a review

;;;###autoload
(defun rdm-review-show (review)
  "Display REVIEW, freshly read with `rdm review show', in a buffer.
Returns the buffer."
  (interactive (list (rdm-review-read-review)))
  (let* ((id (plist-get review :id))
         (full (rdm-call-sync 'review-show id))
         (buf (get-buffer-create (format "*rdm review %s*" id))))
    (with-current-buffer buf
      (special-mode)
      (let ((inhibit-read-only t))
        (erase-buffer)
        (insert (propertize (format "Review %s" id) 'font-lock-face 'rdm-view-title)
                "\n")
        (rdm-view--field "Target" (rdm-review--ref full))
        (rdm-view--field "Author" (plist-get full :author))
        (rdm-view--field "State" (plist-get full :state))
        (rdm-view--field "Verdict" (plist-get full :verdict))
        (rdm-view--field "Created commit" (plist-get full :created_commit))
        (when (rdm--nonempty (plist-get full :body))
          (insert "\n" (plist-get full :body) "\n"))
        (seq-doseq (c (plist-get full :comments))
          (let ((res (plist-get c :resolution)))
            (insert "\n"
                    (propertize (format "#%s  %s  %s" (plist-get c :id)
                                        (plist-get c :status)
                                        (if (plist-get c :anchor)
                                            (or (plist-get res :state) "?")
                                          "whole document"))
                                'font-lock-face 'rdm-view-label)
                    "\n")
            (when-let* ((quoted (plist-get (plist-get c :anchor) :quote)))
              (insert "> " quoted "\n"))
            (insert (or (plist-get c :body) "") "\n")
            (when (rdm--nonempty (plist-get c :reply))
              (insert "Reply: " (plist-get c :reply) "\n"))))
        (goto-char (point-min))))
    (pop-to-buffer buf)
    buf))

;;;; Requests queue

(defun rdm-review-request-candidates ()
  "Candidates for `rdm review requests', in rdm's order.
A plan or change target has no detail buffer; its row is offered but
opening it signals a `user-error'."
  (seq-map
   (lambda (r)
     (let* ((target (plist-get r :target))
            (ref (rdm-review--ref r))
            (item (or (rdm-review-target-item target)
                      (list :kind (intern (or (plist-get target :kind) "unknown"))
                            :identifier ref))))
       (rdm-item-register
        (format "%s  %s" ref (plist-get r :id))
        (append item
                (list :title (or (plist-get r :author) "")
                      :annotation (format "%s  %d open"
                                          (or (plist-get r :verdict) "")
                                          (rdm-review--open-count r))
                      :review-id (plist-get r :id))))))
   (rdm-call-sync 'review-requests)))

(defun rdm-review--goto-review (id)
  "Move point to the first overlaid comment of review ID, if any."
  (when-let* ((entry (seq-find (lambda (e)
                                 (and (plist-get e :overlay)
                                      (equal (plist-get (plist-get e :review) :id) id)))
                               rdm-review--entries)))
    (goto-char (overlay-start (plist-get entry :overlay)))))

;;;###autoload
(defun rdm-review-requests ()
  "Pick a review requesting changes and open its target with its reviews.
Returns the detail buffer."
  (interactive)
  (let* ((item (rdm-pick-read "Review request: " (rdm-review-request-candidates)
                              nil t))
         (buf (rdm-open-item item)))
    (with-current-buffer buf
      (rdm-review--goto-review (plist-get item :review-id)))
    buf))

;;;; Revision handoff

(defun rdm-review-revise-copy (request)
  "Copy REQUEST's `:command' to the kill ring and echo it; return it."
  (let ((command (plist-get request :command)))
    (kill-new command)
    (message "Copied %s" command)
    command))

(defun rdm-review-revise (review)
  "Hand REVIEW to the revision loop through `rdm-review-revise-function'.
No process is started here.  Returns the function's value."
  (interactive (list (rdm-review-read-review)))
  (funcall rdm-review-revise-function
           (list :review-id (plist-get review :id)
                 :target (rdm-review--ref review)
                 :project (plist-get (rdm-context) :project)
                 :command (format rdm-review-revise-command-format
                                  (plist-get review :id)))))

;;;; Authoring

(defvar-local rdm-review--draft nil
  "The own draft review this detail buffer is authoring, or nil.
A plist: `:id', `:commit' (its `created_commit') and `:previous-at'
\(the revision the buffer showed before it followed the draft).")

(defun rdm-review--require-view ()
  "Signal a `user-error' unless this is a detail buffer of a reviewable item."
  (unless (and (derived-mode-p 'rdm-view-mode) rdm-view--item)
    (user-error "rdm: Not in an rdm detail buffer"))
  (rdm-review-item-target rdm-view--item))

(defun rdm-review-own-draft (item)
  "Your newest draft review of ITEM, or nil.
Read with `rdm review list --on <target> --state draft' and `--mine',
or `--author' when `rdm-review-author' is set; rdm lists newest first,
so this is the first draft it returns."
  (let ((override (rdm--nonempty rdm-review-author)))
    (car (append (rdm-call-sync 'review-list
                                :on (rdm-review-item-target item)
                                :state "draft"
                                :author override
                                :mine (not override))
                 nil))))

(defun rdm-review--short (commit)
  "COMMIT abbreviated for display."
  (if (> (length commit) 12) (substring commit 0 12) commit))

(defun rdm-review--show-draft ()
  "Show in the header line which revision this buffer's draft anchors to."
  (setq header-line-format
        (when-let* ((draft rdm-review--draft))
          (let ((commit (plist-get draft :commit)))
            (format "Reviewing as of %s  (draft %s)"
                    (if (rdm--nonempty commit) (rdm-review--short commit)
                      "the current text")
                    (plist-get draft :id))))))

(defun rdm-review--adopt (review)
  "Make REVIEW, an own draft's JSON, this buffer's draft and show its text.
The buffer is re-rendered at REVIEW's `created_commit' -- the text rdm
locates quotes in -- unless it already shows that revision."
  (let ((commit (rdm--nonempty (plist-get review :created_commit)))
        (previous (if rdm-review--draft
                      (plist-get rdm-review--draft :previous-at)
                    rdm-view--at)))
    (setq rdm-review--draft (list :id (plist-get review :id) :commit commit
                                  :previous-at previous))
    (if (and commit (not (equal rdm-view--at commit)))
        (progn (setq rdm-view--at commit)
               (rdm-view--revert))
      (rdm-review--show-draft))))

(defun rdm-review--follow-draft ()
  "Re-render a current-revision render at its own draft's `created_commit'.
Returns non-nil when it re-rendered.  Runs from the render hook; the
re-render has a revision, so it does not look again."
  (let ((item rdm-view--item))
    (when (and rdm-review-in-views (not rdm-view--at)
               (memq (plist-get item :kind) '(roadmap phase task)))
      (let ((draft (condition-case err
                       (rdm-review-own-draft item)
                     (error (message "rdm: drafts of %s: %s"
                                     (rdm-item-identifier item)
                                     (error-message-string err))
                            nil))))
        (setq rdm-review--draft nil)
        (when draft
          (rdm-review--adopt draft)
          rdm-view--at)))))

(defun rdm-review--begin ()
  "Start a draft review of this buffer's item and adopt it; return its id."
  (let* ((item rdm-view--item)
         (review (rdm-call-sync 'review-start
                                :on (rdm-review-item-target item)
                                :author (rdm--nonempty rdm-review-author)
                                :no-edit t)))
    (rdm-review--adopt review)
    (rdm--after-write item (format "review %s started" (plist-get review :id)))
    (plist-get review :id)))

(defun rdm-review--ensure-draft ()
  "This buffer's draft id: its own draft, an existing one, else a new one.
Either way the buffer ends up showing the draft's `created_commit'."
  (cond
   (rdm-review--draft
    (rdm-review--adopt (list :id (plist-get rdm-review--draft :id)
                             :created_commit (plist-get rdm-review--draft :commit)))
    (plist-get rdm-review--draft :id))
   ((when-let* ((own (rdm-review-own-draft rdm-view--item)))
      (rdm-review--adopt own)
      (plist-get own :id)))
   (t (rdm-review--begin))))

(defun rdm-review-start ()
  "Start (or resume) your draft review of this detail buffer's item.
The buffer then shows the item as of the draft's `created_commit' and
says so in its header line.  Returns the draft's id."
  (interactive)
  (rdm-review--require-view)
  (let ((id (rdm-review--ensure-draft)))
    (message "rdm: reviewing %s in draft %s" (rdm-item-identifier rdm-view--item) id)
    id))

(defun rdm-review-occurrence-starts (body quoted)
  "The starts of the non-overlapping occurrences of QUOTED in BODY.
Counted as `rdm review comment --occurrence' documents: scanning from
the start, each match resumes after its own end."
  (let ((start 0) found pos)
    (unless (string-empty-p quoted)
      (while (setq pos (string-search quoted body start))
        (push pos found)
        (setq start (+ pos (length quoted)))))
    (nreverse found)))

(defun rdm-review-occurrence (body quoted offset)
  "The `--occurrence' of QUOTED at character OFFSET of BODY, or nil.
nil when QUOTED occurs once (no `--occurrence' is needed); else its
1-based index among the non-overlapping occurrences.  A `user-error'
when OFFSET is not one of them (it starts inside an overlapping repeat)."
  (let* ((starts (rdm-review-occurrence-starts body quoted))
         (index (seq-position starts offset)))
    (unless index
      (user-error "rdm: This selection starts inside a repeat of the same \
text, which rdm cannot anchor to; select from the start of the repeat"))
    (when (cdr starts) (1+ index))))

(defun rdm-review--authored (what)
  "Report WHAT was staged on this buffer's item and redraw its reviews."
  (rdm--after-write rdm-view--item what)
  (when rdm-view--at (rdm-review-refresh)))

(defun rdm-review--compose (id description &rest args)
  "Compose a comment on review ID; submitting runs `review comment'.
DESCRIPTION heads the compose buffer; ARGS are the anchor's `rdm-argv'
arguments (none for a whole-document comment)."
  (let ((view (current-buffer)))
    (rdm-compose
     ""
     (lambda (text)
       (when (string-blank-p text)
         (user-error "rdm: a review comment needs a body (or abort it)"))
       (prog1 (apply #'rdm-call-sync 'review-comment id
                     (append args (list :body text)))
         (when (buffer-live-p view)
           (with-current-buffer view
             (rdm-review--authored (format "comment on review %s" id))))))
     description)))

(defun rdm-review--region-args ()
  "The active region as a list (BEG END), or a `user-error'."
  (if (use-region-p)
      (list (region-beginning) (region-end))
    (user-error "rdm: Select the text to comment on (C comments on the \
whole document)")))

(defun rdm-review-comment-region (beg end)
  "Comment on the body text between BEG and END, in your draft review.
The text is the comment's `--quote'; when it occurs more than once in
the body, its `--occurrence' is passed too.  Without a draft one is
started (or yours adopted) and the buffer re-rendered as of its
`created_commit'; if that text differs from what the selection was
made on, the selection is discarded and you are asked to select again.
The comment itself is typed in a compose buffer.  Returns that buffer."
  (interactive (rdm-review--region-args))
  (rdm-review--require-view)
  (let ((region (rdm-view-body-region)))
    (unless (and region (< beg end) (<= (car region) beg) (<= end (cdr region)))
      (user-error "rdm: Select text inside the body to comment on it"))
    (let* ((quoted (buffer-substring-no-properties beg end))
           (shown (rdm-view-body))
           (occurrence (rdm-review-occurrence shown quoted (- beg (car region)))))
      (deactivate-mark)
      (let ((id (rdm-review--ensure-draft)))
        (unless (equal (rdm-view-body) shown)
          (user-error "rdm: Now showing the text as of %s, which review %s \
anchors to; your selection was made on other text, so select it again"
                      (rdm-review--short (or rdm-view--at "?")) id))
        (rdm-review--compose
         id
         (format "Comment in review %s on \"%s\""
                 id (truncate-string-to-width
                     (replace-regexp-in-string "\n" " " quoted) 40 nil nil t))
         :quote quoted :occurrence occurrence)))))

(defun rdm-review-comment-document ()
  "Comment on the whole document in your draft review (started if needed).
Returns the compose buffer."
  (interactive)
  (rdm-review--require-view)
  (let ((id (rdm-review--ensure-draft)))
    (rdm-review--compose id (format "Whole-document comment in review %s" id))))

(defun rdm-review--draft-id ()
  "This buffer's draft id, else your draft on its item, else a `user-error'."
  (rdm-review--require-view)
  (or (plist-get rdm-review--draft :id)
      (when-let* ((own (rdm-review-own-draft rdm-view--item)))
        (rdm-review--adopt own)
        (plist-get own :id))
      (user-error "rdm: No draft review of yours on %s; comment first"
                  (rdm-item-identifier rdm-view--item))))

(defun rdm-review-read-verdict ()
  "Read a verdict from those `rdm describe review' declares."
  (completing-read "Verdict: "
                   (or (rdm-enum-values 'review "verdict")
                       (user-error "rdm: `rdm describe review' declares no verdicts"))
                   nil t))

(defun rdm-review-submit (verdict)
  "Submit your draft review of this buffer's item with VERDICT.
Submission is terminal, so it is confirmed first; declining returns
nil.  Afterwards the buffer shows the revision it showed before the
draft, with the submitted review in its Reviews section.  Returns
rdm's output."
  (interactive (progn (rdm-review--draft-id) (list (rdm-review-read-verdict))))
  (let ((id (rdm-review--draft-id)))
    (when (y-or-n-p (format "Submit review %s of %s as %s? " id
                            (rdm-item-identifier rdm-view--item) verdict))
      (prog1 (rdm-call-sync 'review-submit id :verdict verdict)
        (setq rdm-view--at (plist-get rdm-review--draft :previous-at)
              rdm-review--draft nil)
        (rdm-review--show-draft)
        (let ((at rdm-view--at))
          (rdm--after-write rdm-view--item (format "review %s → %s" id verdict))
          (when at (revert-buffer)))))))

;;;; rdm-dispatch suffixes

(defun rdm-review-dispatch-show ()
  "Show a review of the `rdm-dispatch' item."
  (interactive)
  (rdm-review-show (rdm-review-read-review (rdm-dispatch--item))))

(defun rdm-review-dispatch-close ()
  "Close a review of the `rdm-dispatch' item as addressed."
  (interactive)
  (rdm-review-close (rdm-review-read-review (rdm-dispatch--item))))

(defun rdm-review-dispatch-dismiss ()
  "Dismiss a review of the `rdm-dispatch' item."
  (interactive)
  (rdm-review-dismiss (rdm-review-read-review (rdm-dispatch--item))))

(defun rdm-review-dispatch-revise ()
  "Hand a review of the `rdm-dispatch' item to the revision loop."
  (interactive)
  (rdm-review-revise (rdm-review-read-review (rdm-dispatch--item))))

(defun rdm-review--dispatch-view ()
  "The detail buffer of the `rdm-dispatch' item: this one, else opened."
  (let ((item (rdm-dispatch--item)))
    (if (rdm-review--shows-p item)
        (current-buffer)
      (rdm-open-item item))))

(defun rdm-review-dispatch-start ()
  "Start (or resume) your draft review of the `rdm-dispatch' item."
  (interactive)
  (with-current-buffer (rdm-review--dispatch-view)
    (rdm-review-start)))

(defun rdm-review-dispatch-comment ()
  "Comment on the whole `rdm-dispatch' item in your draft review."
  (interactive)
  (with-current-buffer (rdm-review--dispatch-view)
    (rdm-review-comment-document)))

(defun rdm-review-dispatch-submit ()
  "Submit your draft review of the `rdm-dispatch' item with a verdict."
  (interactive)
  (with-current-buffer (rdm-review--dispatch-view)
    (rdm-review--draft-id)
    (rdm-review-submit (rdm-review-read-verdict))))

;;;; Keys

(keymap-set rdm-view-mode-map "K" #'rdm-review-comment-at-point)
(keymap-set rdm-view-mode-map "M-n" #'rdm-review-next-comment)
(keymap-set rdm-view-mode-map "M-p" #'rdm-review-previous-comment)
(keymap-set rdm-view-mode-map "A" #'rdm-review-resolve)
(keymap-set rdm-view-mode-map "c" #'rdm-review-comment-region)
(keymap-set rdm-view-mode-map "C" #'rdm-review-comment-document)
(keymap-set rdm-view-mode-map "S" #'rdm-review-submit)

(provide 'rdm-review)
;;; rdm-review.el ends here
