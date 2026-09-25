;;; rdm-transient.el --- The rdm-dispatch transient: status, tags, staging  -*- lexical-binding: t; -*-

;; This file is not part of GNU Emacs.

;;; Commentary:

;; `rdm-dispatch' is the write surface of the rdm integration: a transient
;; scoped to one item (the item at point in a detail buffer, or the target
;; of an embark action) with three groups:
;;
;; - Status: one suffix per value `rdm describe <kind>' declares for the
;;   item's `status' field (see `rdm-status-values'); there is no status
;;   list here.  `blocked' asks for a reason; the statuses in
;;   `rdm-confirm-statuses' ask for confirmation.  A roadmap has no status.
;; - Tags: add and remove tags.  The write re-reads the item's tags and
;;   sends the whole list back, so tags the user did not touch (e.g.
;;   `needs-plan-review', `depends-unlanded') are kept.
;; - Plan repo: `rdm status', `rdm commit' and `rdm discard --force' over
;;   this Emacs's changeset (every rdm process the package runs carries
;;   `RDM_SESSION'; see `rdm-session').  This group works with no item.
;;
;; rdm stages every write; nothing is committed until `rdm-plan-commit'.
;; A write rdm refuses (e.g. the `gates.reviewed' gate) signals rdm's own
;; error message.  After a write, every detail buffer showing the item is
;; refreshed.
;;
;; The functions below (`rdm-set-status', `rdm-edit-tags',
;; `rdm-plan-status', `rdm-plan-commit', `rdm-plan-discard') are usable
;; without the transient.  Further suffixes attach with
;; `transient-append-suffix': the Tags suffix has key "T" and the Plan
;; repo group's last suffix key "D".

;;; Code:

(require 'cl-lib)
(require 'seq)
(require 'subr-x)
(require 'transient)
(require 'rdm-core)
(require 'rdm-view)

(defcustom rdm-confirm-statuses '("done" "wont-fix")
  "Status values `rdm-set-status' asks to confirm before writing."
  :type '(repeat string)
  :group 'rdm)

;;;; Item writes

(defun rdm--update-args (item)
  "The update action and positional arguments for ITEM."
  (pcase (plist-get item :kind)
    ('roadmap (list 'roadmap-update (plist-get item :roadmap)))
    ('phase (list 'phase-update (plist-get item :stem)
                  :roadmap (plist-get item :roadmap)))
    ('task (list 'task-update (plist-get item :slug)))
    (kind (user-error "rdm: cannot update a %s" kind))))

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
  "Refresh views of ITEM and report WHAT was staged."
  (rdm--refresh-views item)
  (message "rdm: %s %s staged — commit from rdm-dispatch (or `rdm commit')"
           (rdm-item-identifier item) what))

(defun rdm-set-status (item status &optional reason)
  "Set the status of ITEM (a phase or task) to STATUS.
For `blocked', REASON is recorded; when nil it is read from the
minibuffer (empty means none).  A status in `rdm-confirm-statuses' is
confirmed first; declining writes nothing and returns nil.  Returns
rdm's output."
  (when (eq (plist-get item :kind) 'roadmap)
    (user-error "rdm: a roadmap has no status"))
  (when (and (equal status "blocked") (null reason))
    (setq reason (read-string (format "Reason %s is blocked: "
                                      (rdm-item-identifier item)))))
  (when (or (not (member status rdm-confirm-statuses))
            (y-or-n-p (format "Mark %s %s? " (rdm-item-identifier item) status)))
    (prog1 (apply #'rdm-call-sync
                  (append (rdm--update-args item)
                          (list :status status
                                :reason (and (equal status "blocked")
                                             (rdm--nonempty reason)))))
      (rdm--after-write item (concat "→ " status)))))

(defun rdm-item-tags (item)
  "ITEM's current tags, freshly read with `show'."
  (append (plist-get (apply #'rdm-call-sync (rdm-item-show-args item)) :tags)
          nil))

(defun rdm-edit-tags (item add remove)
  "Add the tags ADD to ITEM and remove the tags REMOVE.
The current tags are re-read and the whole list written back, so every
tag not in REMOVE survives.  An edit leaving no tags is refused, since
rdm cannot clear a tag list; one changing nothing writes nothing.
Returns the new tag list."
  (let* ((current (rdm-item-tags item))
         (tags (seq-uniq (append (seq-remove (lambda (tag) (member tag remove))
                                             current)
                                 (seq-remove #'string-empty-p
                                             (mapcar #'string-trim add))))))
    (cond
     ((equal tags current)
      (message "rdm: tags of %s unchanged" (rdm-item-identifier item)))
     ((null tags)
      (user-error "rdm: cannot clear a tag list; keep at least one tag on %s"
                  (rdm-item-identifier item)))
     (t
      (apply #'rdm-call-sync
             (append (rdm--update-args item)
                     (list :tags (string-join tags ","))))
      (rdm--after-write item "tags")))
    tags))

;;;; Plan repo

(defun rdm--show-output (text)
  "Echo TEXT, rdm's output, trimmed; return it."
  (message "%s" (string-trim text))
  text)

(defun rdm-plan-status ()
  "Show this Emacs's staged plan-repo changes (`rdm status'); return the text."
  (interactive)
  (rdm--show-output (rdm-call-sync 'status)))

(defun rdm-plan-commit (message)
  "Commit this Emacs's staged plan-repo changes with MESSAGE."
  (interactive (list (read-string "Commit message: ")))
  (unless (rdm--nonempty (string-trim message))
    (user-error "rdm: a commit needs a message"))
  (rdm--show-output (rdm-call-sync 'commit :message message)))

(defun rdm-plan-discard ()
  "Discard this Emacs's staged plan-repo changes after confirming.
Returns rdm's output, or nil when declined."
  (interactive)
  (when (y-or-n-p "Discard every change staged from this Emacs? ")
    (rdm--show-output (rdm-call-sync 'discard :force t))))

;;;; The transient

(defun rdm-dispatch--item ()
  "The item `rdm-dispatch' acts on, or a `user-error'."
  (or (transient-scope) (user-error "rdm: no item for this action")))

(defun rdm-dispatch--status-keys (values)
  "Distinct single-character keys for the status VALUES, in order.
Each value prefers its words' initials, then its other letters."
  (let ((used '("T" "S" "C" "D")) keys)
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
         (add (split-string (read-string "Add tags (comma-separated): ")
                            "," t "[[:space:]]+")))
    (rdm-edit-tags item add remove)))

(defun rdm-dispatch--at-point ()
  "The item at point in a detail buffer, or nil."
  (and (derived-mode-p 'rdm-view-mode) (rdm-view-item-at-point)))

;;;###autoload (autoload 'rdm-dispatch "rdm-transient" nil t)
(transient-define-prefix rdm-dispatch (&optional item)
  "Act on the rdm ITEM: set its status, edit its tags, commit the plan repo.
Interactively, ITEM is the item at point in a detail buffer; elsewhere
only the Plan repo group is offered."
  [:description rdm-dispatch--description
   ["Status"
    :class transient-column
    :if rdm-dispatch--has-status-p
    :setup-children rdm-dispatch--status-children]
   ["Tags"
    :if transient-scope
    ("T" "Edit tags" rdm-dispatch-edit-tags)]]
  ["Plan repo"
   ("S" "Status" rdm-plan-status)
   ("C" "Commit" rdm-plan-commit)
   ("D" "Discard" rdm-plan-discard)]
  (interactive (list (rdm-dispatch--at-point)))
  (transient-setup 'rdm-dispatch nil nil :scope item))

(defun rdm-dispatch-item (item)
  "Open the `rdm-dispatch' transient on ITEM, an rdm item plist.
`rdm-dispatch' is for interactive use only; this is how code opens it."
  (transient-setup 'rdm-dispatch nil nil :scope item))

(keymap-set rdm-view-mode-map "x" #'rdm-dispatch)

(provide 'rdm-transient)
;;; rdm-transient.el ends here
