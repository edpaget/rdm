;;; rdm-worktree.el --- Pick, add, remove and prune rdm worktrees  -*- lexical-binding: t; -*-

;; This file is not part of GNU Emacs.

;;; Commentary:

;; Commands over `rdm worktree', run from the code repo the current buffer
;; belongs to:
;;
;; - `rdm-worktree-list' picks one of the repo's rdm worktrees (annotated
;;   with branch, dirtiness and path) and opens it.
;; - `rdm-worktree-add' creates (or reuses) the worktree for a picked item
;;   (the item at point in a detail buffer, unless given a prefix
;;   argument) and opens it.
;; - `rdm-worktree-remove' removes a picked worktree after confirming;
;;   `rdm-worktree-prune' shows what `--dry-run' would remove, confirms,
;;   then removes every worktree whose item is done.  With a prefix
;;   argument both also delete the branch; with two, they also `--force'.
;;
;; Every worktree opens through `rdm-worktree-open', which calls
;; `rdm-worktree-open-function' (`project-switch-project' by default) on
;; its directory.  Setups that keep one tab per project dedupe there, so
;; opening a worktree that already has a tab switches to it.
;;
;; `rdm-worktree-id' converts an item plist to the item string `rdm
;; worktree' uses: `task/<slug>', `<roadmap>/<stem>' and a bare
;; `<roadmap>' (the worktree a roadmap's phases share);
;; `rdm-item-parse-worktree-id' (rdm-view.el) decodes one back.

;;; Code:

(require 'cl-lib)
(require 'seq)
(require 'subr-x)
(require 'project)
(require 'rdm-core)
(require 'rdm-view)
(require 'rdm-pick)

(defvar rdm-modeline-mode)
(declare-function rdm-modeline-refresh-root "rdm-modeline" (dir))
(declare-function rdm-modeline-forget "rdm-modeline" (dir))

(defcustom rdm-worktree-open-function #'project-switch-project
  "Function opening a worktree directory, called with the directory.
The default switches project with `project-switch-project', so project
and tab setups built on project.el apply.  `find-file' opens a Dired
buffer instead."
  :type 'function
  :group 'rdm)

;;;; Item strings

(defun rdm-worktree-id (item)
  "The `rdm worktree' item string for ITEM, an item plist.
A task is `task/<slug>'; a phase or roadmap is its `rdm-item-identifier'."
  (pcase (plist-get item :kind)
    ('task (concat "task/" (rdm-item-identifier item)))
    ((or 'phase 'roadmap) (rdm-item-identifier item))
    (kind (user-error "rdm: a %s has no worktree" kind))))

;;;; Opening

(defun rdm-worktree--key (dir)
  "DIR as a directory name with symbolic links resolved."
  (file-name-as-directory (file-truename (expand-file-name dir))))

(defun rdm-worktree-open (dir)
  "Open the worktree at DIR with `rdm-worktree-open-function'.
With `rdm-modeline-mode' on, the modeline cache for DIR is refreshed."
  (funcall rdm-worktree-open-function dir)
  (when (bound-and-true-p rdm-modeline-mode)
    (rdm-modeline-refresh-root dir))
  dir)

(defun rdm-worktree--forget (dir)
  "Drop the modeline cache entry for the removed worktree DIR."
  (when (fboundp 'rdm-modeline-forget)
    (rdm-modeline-forget dir)))

;;;; Picking a worktree

(defun rdm-worktree--dirty-p (row)
  "Non-nil when the `worktree list' ROW is dirty."
  (eq (plist-get row :dirty) t))

(defun rdm-worktree-read (prompt)
  "Read one of the repo's rdm worktrees with PROMPT; return its row.
A row is a plist with `:item', `:branch', `:path' and `:dirty'."
  (let* ((rows (append (rdm-call-sync 'worktree-list) nil))
         (cands (mapcar (lambda (row)
                          (cons (or (rdm--nonempty (plist-get row :item))
                                    (plist-get row :path))
                                row))
                        rows))
         (annotate (lambda (cand)
                     (when-let* ((row (cdr (assoc cand cands))))
                       (concat "  "
                               (propertize
                                (format "%s%s  %s"
                                        (or (plist-get row :branch) "")
                                        (if (rdm-worktree--dirty-p row) " (dirty)" "")
                                        (abbreviate-file-name (plist-get row :path)))
                                'face 'completions-annotations)))))
         (table (lambda (string pred action)
                  (if (eq action 'metadata)
                      `(metadata (category . rdm-worktree)
                                 (annotation-function . ,annotate))
                    (complete-with-action action cands string pred)))))
    (unless cands (user-error "rdm: no rdm worktrees in this repo"))
    (cdr (assoc (completing-read prompt table nil t) cands))))

;;;###autoload
(defun rdm-worktree-list ()
  "Pick one of this repo's rdm worktrees and open it."
  (interactive)
  (rdm-worktree-open (plist-get (rdm-worktree-read "Worktree: ") :path)))

;;;; Adding

(defun rdm-worktree--read-item ()
  "Read a task, phase or roadmap to create a worktree for; return its item."
  (pcase (completing-read "Worktree for: " '("task" "phase" "roadmap") nil t)
    ("task" (rdm-pick-read "Task: " (rdm-pick-task-candidates)))
    ("phase" (let ((roadmap (rdm-pick-read-roadmap)))
               (rdm-pick-read (format "Phase of %s: " roadmap)
                              (rdm-pick-phase-candidates roadmap))))
    (_ (list :kind 'roadmap :roadmap (rdm-pick-read-roadmap)))))

;;;###autoload
(defun rdm-worktree-add (item)
  "Create (or reuse) the worktree for ITEM and open it; return its path.
Interactively, ITEM is the item at point in a detail buffer, else (or
with a prefix argument) a picked task, phase or roadmap.  rdm runs from
the current buffer's code repo."
  (interactive
   (list (or (and (not current-prefix-arg)
                  (derived-mode-p 'rdm-view-mode)
                  (rdm-view-item-at-point))
             (rdm-worktree--read-item))))
  (let* ((reply (rdm-call-sync 'worktree-add (rdm-worktree-id item)))
         (path (plist-get reply :path)))
    (message "rdm: worktree for %s %s at %s" (plist-get reply :item)
             (if (eq (plist-get reply :created) t) "created" "reused")
             (abbreviate-file-name path))
    (rdm-worktree-open path)))

;;;; Removing

(defun rdm-worktree--flags (arg)
  "The `:delete-branch' and `:force' arguments for prefix ARG."
  (list :delete-branch (and arg t)
        :force (and (consp arg) (>= (prefix-numeric-value arg) 16))))

;;;###autoload
(defun rdm-worktree-remove (row &optional arg)
  "Remove the worktree ROW (a `worktree list' row) after confirming.
With prefix ARG, also delete its branch; with two, also `--force'.
Returns rdm's output, or nil when declined."
  (interactive (list (rdm-worktree-read "Remove worktree: ") current-prefix-arg))
  (let ((path (plist-get row :path)))
    (when (y-or-n-p (format "Remove worktree %s%s? "
                            (abbreviate-file-name path)
                            (if (rdm-worktree--dirty-p row)
                                " (it has uncommitted changes)"
                              "")))
      (prog1 (apply #'rdm-call-sync 'worktree-remove path
                    (rdm-worktree--flags arg))
        (rdm-worktree--forget path)
        (message "rdm: removed worktree %s" (abbreviate-file-name path))))))

;;;###autoload
(defun rdm-worktree-prune (&optional arg)
  "Remove every worktree whose item is done, after a dry run and confirmation.
With prefix ARG, also delete their branches; with two, also `--force'
\(dirty worktrees included).  Returns rdm's reply, or nil when there is
nothing to prune or it was declined."
  (interactive "P")
  (let* ((flags (rdm-worktree--flags arg))
         (dry (apply #'rdm-call-sync 'worktree-prune :dry-run t flags))
         (doomed (seq-filter (lambda (r) (equal (plist-get r :action) "would-remove"))
                             (plist-get dry :results))))
    (cond
     ((null doomed)
      (message "rdm: no done worktrees to prune")
      nil)
     ((y-or-n-p (format "Prune %d worktree(s): %s? " (length doomed)
                        (mapconcat (lambda (r) (plist-get r :item)) doomed ", ")))
      (let ((reply (apply #'rdm-call-sync 'worktree-prune flags)))
        (seq-doseq (r (plist-get reply :results))
          (when (member (plist-get r :action) '("removed" "removed-branch-kept"))
            (rdm-worktree--forget (plist-get r :path))))
        (message "rdm: pruned %s worktree(s)" (plist-get reply :removed))
        reply)))))

(provide 'rdm-worktree)
;;; rdm-worktree.el ends here
