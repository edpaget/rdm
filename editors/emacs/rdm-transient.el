;;; rdm-transient.el --- The rdm-dispatch transient: status, tags, bodies, create, staging  -*- lexical-binding: t; -*-

;; This file is not part of GNU Emacs.

;;; Commentary:

;; `rdm-dispatch' is the write surface of the rdm integration: a transient
;; scoped to one item (the item at point in a detail buffer, or the target
;; of an embark action) with five groups:
;;
;; - Status: one suffix per value `rdm describe <kind>' declares for the
;;   item's `status' field (see `rdm-status-values'); there is no status
;;   list here.  A status rdm declares a note flag for (`value_flags',
;;   see `rdm-value-flag') asks for that note; the terminal statuses rdm
;;   declares (or `rdm-confirm-statuses', when set) ask for confirmation.
;;   A roadmap has no status.
;; - Edit: add and remove tags, and edit the body.  The tag write is one
;;   `update --add-tag/--remove-tag' call; rdm applies it to the current
;;   list, so tags the user did not touch (e.g. `needs-plan-review',
;;   `depends-unlanded') are kept by rdm, and rdm normalizes the text.
;;   The body opens in a compose buffer (rdm-compose.el) holding the
;;   current body; submitting writes it with `--body', or `--clear-body'
;;   when the buffer was emptied.
;; - Create: a roadmap, phase or task.  Slug, title, tags (and a phase's
;;   optional number) come from the minibuffer, the body from a compose
;;   buffer, piped to rdm on stdin.  A phase goes into the item's roadmap
;;   (a phase item's parent), else a picked one.  This group works with
;;   no item.
;; - Review: show one of the item's reviews, pick from the requests
;;   queue, close a review as addressed, dismiss one, hand one to the
;;   revision loop, and author your own: start a draft, comment on the
;;   whole document, submit with a verdict (rdm-review.el).  Offered
;;   only with an item.
;; - Plan repo: `rdm status', `rdm commit' and `rdm discard --force' over
;;   this Emacs's changeset (every rdm process the package runs carries
;;   `RDM_SESSION'; see `rdm-session'), each read as rdm's JSON reply and
;;   summarized from its data.  This group works with no item.
;;
;; rdm stages every write; nothing is committed until `rdm-plan-commit'.
;; A write rdm refuses (e.g. the `gates.reviewed' gate) signals rdm's own
;; error message.  After a write, every detail buffer showing the item is
;; refreshed and `rdm-after-write-hook' runs.
;;
;; The functions below (`rdm-set-status', `rdm-edit-tags',
;; `rdm-edit-body', `rdm-update-body', `rdm-create-roadmap',
;; `rdm-create-phase', `rdm-create-task', `rdm-create-item',
;; `rdm-plan-status', `rdm-plan-commit', `rdm-plan-discard') are usable
;; without the transient.  Further suffixes attach with
;; `transient-append-suffix': the Edit group's suffixes have keys "T"
;; and "B", the Create group's "+r", "+p" and "+t", the Review group's
;; "Rs", "Rq", "Ra", "Rd", "Rv", "Rn", "Rc" and "RS", and the Plan repo
;; group's last suffix
;; key "D".  A new fixed key belongs in
;; `rdm-dispatch--fixed-keys' so generated status keys avoid it.

;;; Code:

(require 'cl-lib)
(require 'seq)
(require 'subr-x)
(require 'transient)
(require 'rdm-core)
(require 'rdm-view)
(require 'rdm-compose)

;; rdm-pick requires this file, so it is loaded lazily where needed.
(declare-function rdm-pick-read-roadmap "rdm-pick" (&optional prompt))

;; rdm-review requires this file; the Review group's suffixes load it.
(autoload 'rdm-review-dispatch-show "rdm-review" nil t)
(autoload 'rdm-review-requests "rdm-review" nil t)
(autoload 'rdm-review-dispatch-close "rdm-review" nil t)
(autoload 'rdm-review-dispatch-dismiss "rdm-review" nil t)
(autoload 'rdm-review-dispatch-revise "rdm-review" nil t)
(autoload 'rdm-review-dispatch-start "rdm-review" nil t)
(autoload 'rdm-review-dispatch-comment "rdm-review" nil t)
(autoload 'rdm-review-dispatch-submit "rdm-review" nil t)

(defcustom rdm-confirm-statuses nil
  "Status values `rdm-set-status' asks to confirm before writing.
nil means the terminal statuses rdm declares for the item's kind
\(`rdm-terminal-values'); a list of status values overrides them."
  :type '(choice (const :tag "rdm's terminal statuses" nil)
                 (repeat string))
  :group 'rdm)

;;;; Item writes

(defun rdm--update-args (item)
  "The update action and positional arguments for ITEM."
  (pcase (plist-get item :kind)
    ('roadmap (list 'roadmap-update (plist-get item :roadmap)))
    ('phase (list 'phase-update (plist-get item :stem)
                  :roadmap (plist-get item :roadmap)))
    ('task (list 'task-update (plist-get item :slug)))
    (kind (user-error "rdm: Cannot update a %s" kind))))

(defun rdm--refresh-views (item)
  "Re-fetch every current-revision detail buffer showing ITEM.
A phase's roadmap view is refreshed too, since it lists the phase."
  (let ((id (rdm-item-identifier item))
        (kind (plist-get item :kind)))
    (dolist (buf (buffer-list))
      (with-current-buffer buf
        (when (and (derived-mode-p 'rdm-view-mode)
                   rdm-view--item
                   (not rdm-view--at)
                   (or (and (eq (plist-get rdm-view--item :kind) kind)
                            (equal (rdm-item-identifier rdm-view--item) id))
                       (and (eq kind 'phase)
                            (eq (plist-get rdm-view--item :kind) 'roadmap)
                            (equal (plist-get rdm-view--item :roadmap)
                                   (plist-get item :roadmap)))))
          (revert-buffer))))))

(defun rdm--after-write (item what)
  "Refresh views of ITEM, report WHAT was staged, run `rdm-after-write-hook'."
  (rdm--refresh-views item)
  (message "rdm: %s %s staged — commit from rdm-dispatch (or `rdm commit')"
           (rdm-item-identifier item) what)
  (run-hooks 'rdm-after-write-hook))

(defun rdm-set-status (item status &optional note)
  "Set the status of ITEM (a phase or task) to STATUS.
When rdm declares a note flag for STATUS (`rdm-value-flag', e.g. a
reason), NOTE is recorded with it; when nil it is read from the
minibuffer (empty means none).  A status in `rdm-confirm-statuses', or
when that is nil one of the terminal statuses rdm declares for ITEM's
kind, is confirmed first; declining writes nothing and returns nil.
Returns rdm's output."
  (let ((kind (plist-get item :kind)))
    (when (eq kind 'roadmap)
      (user-error "rdm: a roadmap has no status"))
    (let ((flag (rdm-value-flag kind "status" status)))
      (when (and flag (null note))
        (setq note (read-string (format "--%s for %s → %s (empty for none): "
                                        flag (rdm-item-identifier item) status))))
      (when (or (not (member status (or rdm-confirm-statuses
                                        (rdm-terminal-values kind "status"))))
                (y-or-n-p (format "Mark %s %s? " (rdm-item-identifier item) status)))
        (prog1 (apply #'rdm-call-sync
                      (append (rdm--update-args item)
                              (list :status status)
                              (when (and flag (rdm--nonempty note))
                                (list (intern (concat ":" flag)) note))))
          (rdm--after-write item (concat "→ " status)))))))

(defun rdm-item-tags (item)
  "ITEM's current tags, freshly read with `show'."
  (append (plist-get (apply #'rdm-call-sync (rdm-item-show-args item)) :tags)
          nil))

(defun rdm-edit-tags (item add remove)
  "Add the tags ADD to ITEM and remove the tags REMOVE, in one rdm call.
ADD and REMOVE are lists of strings passed to rdm as `--add-tag' and
`--remove-tag'; rdm applies them to the current list (removals first),
keeps every tag named in neither, and trims, drops empty and
de-duplicates the elements.  Removing the last tag leaves none.  When
ADD and REMOVE are both nil nothing is written and nil is returned;
otherwise returns rdm's output."
  (when (or add remove)
    (prog1 (apply #'rdm-call-sync
                  (append (rdm--update-args item)
                          (mapcan (lambda (tag) (list :add-tag tag)) add)
                          (mapcan (lambda (tag) (list :remove-tag tag)) remove)))
      (rdm--after-write item "tags"))))

;;;; Bodies

(defun rdm-update-body (item text)
  "Set ITEM's body to TEXT with `--body'; a blank TEXT clears the body.
Returns rdm's output."
  (prog1 (apply #'rdm-call-sync
                (append (rdm--update-args item)
                        (if (string-blank-p text)
                            (list :clear-body t)
                          (list :body text))))
    (rdm--after-write item "body")))

(defun rdm-edit-body (item)
  "Edit ITEM's body in a compose buffer; submitting writes it.
The buffer starts with the current body; emptying it clears the body.
Returns the compose buffer."
  (let ((body (or (plist-get (apply #'rdm-call-sync (rdm-item-show-args item))
                             :body)
                  "")))
    (rdm-compose body
                 (lambda (text) (rdm-update-body item text))
                 (format "Body of %s %s" (plist-get item :kind)
                         (rdm-item-identifier item)))))

;;;; Creating items

(cl-defun rdm-create-item (kind slug title body &key tags roadmap number)
  "Create a KIND (`roadmap', `phase' or `task') named SLUG titled TITLE.
BODY is piped to rdm on stdin (never `--body').  TAGS is a
comma-separated string passed to rdm as typed (rdm trims it, drops empty
tags and de-duplicates), or nil.  A phase needs ROADMAP; SLUG is its bare
slug (rdm prepends `phase-<n>-'), and NUMBER, when non-nil, its number
\(passed to rdm as typed).  The item plist is built from the created item
rdm reports: its `kind', `roadmap'/`stem'/`slug', `ref' and
`review_ref'.  Refreshes the views the new item affects and returns
that item plist."
  (let* ((action (pcase kind
                   ('roadmap 'roadmap-create)
                   ('phase 'phase-create)
                   ('task 'task-create)
                   (_ (user-error "rdm: Cannot create a %s" kind))))
         (json (apply #'rdm-call-sync action slug
                      :title title :tags (rdm--nonempty tags)
                      (append (when (eq kind 'phase)
                                (list :roadmap roadmap :number number))
                              (list :stdin body))))
         (created (intern (plist-get json :kind)))
         (item (rdm-item-with-refs
                (pcase created
                  ('roadmap (list :kind created :roadmap (plist-get json :slug)))
                  ('phase (list :kind created :roadmap (plist-get json :roadmap)
                                :stem (plist-get json :stem)))
                  (_ (list :kind created :slug (plist-get json :slug))))
                json)))
    (rdm--after-write item "created")
    item))

(defun rdm--read-required (prompt)
  "Read a non-empty, trimmed string with PROMPT."
  (or (rdm--nonempty (string-trim (read-string prompt)))
      (user-error "rdm: a value is required")))

(defun rdm--compose-create (kind slug title tags &rest keys)
  "Open a compose buffer for the body of a new KIND; submitting creates it.
SLUG, TITLE, TAGS and KEYS are passed to `rdm-create-item'."
  (rdm-compose ""
               (lambda (text)
                 (apply #'rdm-create-item kind slug title text :tags tags keys))
               (format "Body of new %s %s" kind slug)))

;;;###autoload
(defun rdm-create-roadmap (slug title tags)
  "Create a roadmap SLUG titled TITLE with TAGS, composing its body first.
Interactively, read SLUG, TITLE and comma-separated TAGS.  Returns the
compose buffer."
  (interactive (list (rdm--read-required "New roadmap slug: ")
                     (rdm--read-required "Title: ")
                     (read-string "Tags (comma-separated): ")))
  (rdm--compose-create 'roadmap slug title tags))

;;;###autoload
(defun rdm-create-task (slug title tags)
  "Create a task SLUG titled TITLE with TAGS, composing its body first.
Interactively, read SLUG, TITLE and comma-separated TAGS.  Returns the
compose buffer."
  (interactive (list (rdm--read-required "New task slug: ")
                     (rdm--read-required "Title: ")
                     (read-string "Tags (comma-separated): ")))
  (rdm--compose-create 'task slug title tags))

(defun rdm--context-roadmap (item)
  "The roadmap ITEM belongs to, else one read with the roadmap picker."
  (if (memq (plist-get item :kind) '(roadmap phase))
      (plist-get item :roadmap)
    (require 'rdm-pick)
    (rdm-pick-read-roadmap "New phase in roadmap: ")))

;;;###autoload
(defun rdm-create-phase (&optional context)
  "Create a phase, composing its body first; return the compose buffer.
Its roadmap is CONTEXT's (a roadmap item, or a phase item's roadmap),
else picked.  The bare slug, title, optional number and comma-separated
tags are read from the minibuffer.  Interactively, CONTEXT is the item
at point in a detail buffer."
  (interactive (list (rdm-dispatch--at-point)))
  (let* ((roadmap (rdm--context-roadmap context))
         (slug (rdm--read-required
                (format "New phase slug in %s (rdm adds phase-N-): " roadmap)))
         (title (rdm--read-required "Title: "))
         (number (rdm--nonempty
                  (string-trim (read-string "Number (empty for the next): "))))
         (tags (read-string "Tags (comma-separated): ")))
    (rdm--compose-create 'phase slug title tags :roadmap roadmap :number number)))

;;;; Plan repo

(defun rdm--other-changes (changes)
  "Lines counting the CHANGES (a `changes' reply) this Emacs did not stage."
  (delq nil
        (list (when (> (length (plist-get changes :others)) 0)
                (format "%d staged by other sessions"
                        (length (plist-get changes :others))))
              (when (> (length (plist-get changes :unattributed)) 0)
                (format "%d unattributed"
                        (length (plist-get changes :unattributed)))))))

(defun rdm--status-summary (json)
  "The text summarizing JSON, rdm's `status' reply."
  (let* ((changes (plist-get json :changes))
         (user (plist-get changes :user))
         (merge (plist-get json :merge))
         (sync (plist-get json :sync)))
    (string-join
     (append
      (list (if (> (length user) 0)
                "rdm: staged from this Emacs:"
              "rdm: nothing staged from this Emacs"))
      (mapcar (lambda (c) (format "  %s %s" (plist-get c :change) (plist-get c :path)))
              user)
      (rdm--other-changes changes)
      (when merge
        (list (format "merge in progress (%d unmerged)" (plist-get merge :unmerged))))
      (when sync
        (list (format "%s vs %s: %d ahead, %d behind"
                      (plist-get sync :branch) (plist-get sync :remote)
                      (plist-get sync :ahead) (plist-get sync :behind)))))
     "\n")))

(defun rdm--uncommitted-paths (changes)
  "Lines naming the CHANGES (a `changes' reply) left uncommitted.
Each group lists its paths and the rdm commands that recover them."
  (let ((others (append (plist-get changes :others) nil))
        (unattributed (append (plist-get changes :unattributed) nil))
        (paths (lambda (cs) (mapcar (lambda (c) (format "  %s" (plist-get c :path)))
                                    cs))))
    (append
     (when others
       (append (list (format "%d uncommitted path(s) belong to another changeset:"
                             (length others)))
               (funcall paths others)
               (list "recover with `rdm session list' and `rdm commit --changeset <id>',\
 or `rdm commit --all'")))
     (when unattributed
       (append (list (format "%d uncommitted path(s) are not attributed to any changeset:"
                             (length unattributed)))
               (funcall paths unattributed)
               (list "recover with `rdm commit --all'"))))))

(defun rdm--commit-summary (json)
  "The text summarizing JSON, rdm's `commit' reply."
  (let ((sha (plist-get json :commit))
        (changes (plist-get json :changes))
        (skipped (append (plist-get json :skipped) nil))
        (advisory (plist-get json :advisory)))
    (string-join
     (append
      (if sha
          (cons (format "rdm: committed %s" (substring sha 0 (min 12 (length sha))))
                (rdm--other-changes changes))
        (cons "rdm: nothing committed" (rdm--uncommitted-paths changes)))
      (when skipped
        (list (format "skipped: %s" (string-join skipped ", "))))
      (when advisory (list advisory)))
     "\n")))

(defun rdm--discard-summary (json)
  "The text summarizing JSON, rdm's `discard' reply."
  (let ((restored (length (plist-get json :restored)))
        (skipped (length (plist-get json :skipped))))
    (concat (format "rdm: discarded %d path%s" restored (if (= restored 1) "" "s"))
            (if (> skipped 0)
                (format ", skipped %d changed by another session" skipped)
              "")
            (if (eq (plist-get json :merge_aborted) t) " (merge aborted)" ""))))

;;;###autoload
(defun rdm-plan-status ()
  "Show this Emacs's staged plan-repo changes (`rdm status').
Returns rdm's JSON reply as a plist."
  (interactive)
  (let ((json (rdm-call-sync 'status)))
    (message "%s" (rdm--status-summary json))
    json))

;;;###autoload
(defun rdm-plan-commit (message)
  "Commit this Emacs's staged plan-repo changes with MESSAGE.
Returns rdm's JSON reply as a plist."
  (interactive (list (read-string "Commit message: ")))
  (unless (rdm--nonempty (string-trim message))
    (user-error "rdm: a commit needs a message"))
  (let ((json (rdm-call-sync 'commit :message message)))
    (message "%s" (rdm--commit-summary json))
    json))

;;;###autoload
(defun rdm-plan-discard ()
  "Discard this Emacs's staged plan-repo changes after confirming.
Returns rdm's JSON reply as a plist, or nil when declined."
  (interactive)
  (when (y-or-n-p "Discard every change staged from this Emacs? ")
    (let ((json (rdm-call-sync 'discard :force t)))
      (message "%s" (rdm--discard-summary json))
      json)))

;;;; The transient

(defun rdm-dispatch--item ()
  "The item `rdm-dispatch' acts on, or a `user-error'."
  (or (transient-scope) (user-error "rdm: No item for this action")))

(defconst rdm-dispatch--fixed-keys '("T" "B" "+" "R" "S" "C" "D")
  "The keys (and key prefixes) `rdm-dispatch' binds to fixed suffixes.
Generated status keys never use one of them.")

(defun rdm-dispatch--status-keys (values)
  "Distinct single-character keys for the status VALUES, in order.
Each value prefers its words' initials, then its other letters."
  (let ((used (copy-sequence rdm-dispatch--fixed-keys)) keys)
    (dolist (v values)
      (let* ((words (split-string v "[^[:alnum:]]+" t))
             (cands (append (mapcar (lambda (w) (substring w 0 1)) words)
                            (mapcar #'char-to-string (string-to-list v))
                            (mapcar #'number-to-string (number-sequence 1 9))))
             (key (seq-find (lambda (k) (and (string-match-p "\\`[[:alnum:]]\\'" k)
                                             (not (member k used))))
                            cands)))
        (push key used)
        (push key keys)))
    (nreverse keys)))

(defun rdm-dispatch--status-specs (item)
  "The Status suffix specs for ITEM: one per value core declares."
  (unless (eq (plist-get item :kind) 'roadmap)
    (let ((values (rdm-status-values (plist-get item :kind))))
      (cl-mapcar (lambda (key value)
                   (list key value
                         (lambda ()
                           (interactive)
                           (rdm-set-status (rdm-dispatch--item) value))))
                 (rdm-dispatch--status-keys values) values))))

(defun rdm-dispatch--status-children (_children)
  "The Status group's suffixes for the transient's item."
  (when-let* ((item (transient-scope)))
    (transient-parse-suffixes 'rdm-dispatch
                              (rdm-dispatch--status-specs item))))

(defun rdm-dispatch--has-status-p ()
  "Non-nil when the transient's item has a status."
  (let ((item (transient-scope)))
    (and item (not (eq (plist-get item :kind) 'roadmap)))))

(defun rdm-dispatch--description ()
  "The heading of `rdm-dispatch': its item and session."
  (let ((item (transient-scope)))
    (format "rdm %s  (session %s)"
            (if item
                (format "%s %s" (plist-get item :kind) (rdm-item-identifier item))
              "plan repo")
            (rdm-session-id))))

(defun rdm-dispatch-edit-tags (item)
  "Read tags to remove and add for ITEM, then `rdm-edit-tags'."
  (interactive (list (rdm-dispatch--item)))
  (let* ((current (rdm-item-tags item))
         (remove (and current
                      (completing-read-multiple
                       (format "Remove tags from %s (comma-separated): "
                               (rdm-item-identifier item))
                       current nil t)))
         (add (rdm--nonempty (read-string "Add tags (comma-separated): "))))
    (rdm-edit-tags item (and add (list add)) remove)))

(defun rdm-dispatch-edit-body (item)
  "Edit ITEM's body in a compose buffer (`rdm-edit-body')."
  (interactive (list (rdm-dispatch--item)))
  (rdm-edit-body item))

(defun rdm-dispatch-create-phase ()
  "Create a phase in the transient's roadmap (or its phase's), else a picked one."
  (interactive)
  (rdm-create-phase (transient-scope)))

(defun rdm-dispatch--at-point ()
  "The item at point in a detail buffer, or nil."
  (and (derived-mode-p 'rdm-view-mode) (rdm-view-item-at-point)))

;;;###autoload (autoload 'rdm-dispatch "rdm-transient" nil t)
(transient-define-prefix rdm-dispatch (&optional item)
  "Act on the rdm ITEM: status, tags, body, reviews; create; commit.
Interactively, ITEM is the item at point in a detail buffer; elsewhere
only the Create and Plan repo groups are offered."
  [:description rdm-dispatch--description
   ["Status"
    :class transient-column
    :if rdm-dispatch--has-status-p
    :setup-children rdm-dispatch--status-children]
   ["Edit"
    :if transient-scope
    ("T" "Edit tags" rdm-dispatch-edit-tags)
    ("B" "Edit body" rdm-dispatch-edit-body)]
   ["Review"
    :if transient-scope
    ("Rs" "Show review" rdm-review-dispatch-show)
    ("Rq" "Requests queue" rdm-review-requests)
    ("Ra" "Close as addressed" rdm-review-dispatch-close)
    ("Rd" "Dismiss" rdm-review-dispatch-dismiss)
    ("Rv" "Revise (copy command)" rdm-review-dispatch-revise)
    ("Rn" "Start draft review" rdm-review-dispatch-start)
    ("Rc" "Comment on document" rdm-review-dispatch-comment)
    ("RS" "Submit with verdict" rdm-review-dispatch-submit)]]
  [["Create"
    ("+r" "Roadmap" rdm-create-roadmap)
    ("+p" "Phase" rdm-dispatch-create-phase)
    ("+t" "Task" rdm-create-task)]
   ["Plan repo"
    ("S" "Status" rdm-plan-status)
    ("C" "Commit" rdm-plan-commit)
    ("D" "Discard" rdm-plan-discard)]]
  (interactive (list (rdm-dispatch--at-point)))
  (transient-setup 'rdm-dispatch nil nil :scope item))

(defun rdm-dispatch-item (item)
  "Open the `rdm-dispatch' transient on ITEM, an rdm item plist.
`rdm-dispatch' is for interactive use only; this is how code opens it."
  (transient-setup 'rdm-dispatch nil nil :scope item))

(keymap-set rdm-view-mode-map "x" #'rdm-dispatch)

(provide 'rdm-transient)
;;; rdm-transient.el ends here
