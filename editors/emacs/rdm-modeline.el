;;; rdm-modeline.el --- The current rdm worktree's item and status in the mode line  -*- lexical-binding: t; -*-

;; This file is not part of GNU Emacs.

;;; Commentary:

;; `rdm-modeline-segment' returns "ITEM · STATUS" for the rdm worktree the
;; current buffer is in: a task's or phase's status, or a roadmap's
;; done/total phase count for the worktree its phases share.  Outside an
;; rdm worktree it returns "".  `rdm-modeline-nano-segment' is the same
;; text for nano-modeline (or any mode line built from zero-argument
;; functions); nano-modeline is not required.
;;
;; `rdm-modeline-mode', a global minor mode, adds the segment to
;; `mode-line-misc-info' and installs the refresh hooks.  Loading this
;; file changes nothing.
;;
;; The mode line is redrawn constantly, so the segment never runs rdm: it
;; only looks up a cache keyed by the worktree root (the directory
;; holding `.git', found without running a process).  A root not yet in
;; the cache renders "" and schedules one fill on an idle timer.  A fill
;; makes two asynchronous reads: `rdm worktree current' for the item,
;; then the item's own `show --format json' for its status.  A checkout
;; that is not an rdm worktree is cached too, as "".  Every read runs
;; with `default-directory' bound to the entry's root, in a buffer that
;; belongs to it, so a refresh started from any buffer fills each entry
;; from its own worktree.
;;
;; An entry is refreshed (both reads again) by `after-save-hook' and
;; `magit-post-refresh-hook' for the current buffer's root, and by
;; `rdm-after-write-hook' for every cached rdm worktree, since a write
;; may come from a buffer outside the worktree.  A newer fill always wins
;; over an older one still in flight.

;;; Code:

(require 'cl-lib)
(require 'seq)
(require 'subr-x)
(require 'rdm-core)
(require 'rdm-view)
(require 'rdm-worktree)

(defvar magit-post-refresh-hook)

(defcustom rdm-modeline-idle-delay 0.5
  "Idle seconds before an uncached worktree's modeline entry is filled."
  :type 'number
  :group 'rdm)

(defface rdm-modeline '((t :inherit mode-line-emphasis))
  "Face for the rdm modeline segment."
  :group 'rdm)

;;;; The cache

(cl-defstruct (rdm-modeline--entry (:constructor rdm-modeline--entry-create)
                                   (:copier nil))
  "One worktree root's modeline state."
  (label nil :documentation "The rendered text, \"\" for none, nil until filled.")
  (item nil :documentation "The worktree's item string, or nil.")
  (generation 0 :documentation "Bumped by each fill; replies of older fills are dropped."))

(defvar rdm-modeline--cache (make-hash-table :test #'equal)
  "Modeline entries, keyed by worktree root (see `rdm-worktree--key').")

(defvar-local rdm-modeline--root-memo nil
  "(DIRECTORY . ROOT): the root `rdm-modeline--root' found for DIRECTORY.")

(defun rdm-modeline--root ()
  "The cache key for the current buffer's checkout, or nil.
The directory holding `.git' above `default-directory', memoized per
buffer.  Runs no process; remote directories have none."
  (let ((dir default-directory))
    (if (equal (car rdm-modeline--root-memo) dir)
        (cdr rdm-modeline--root-memo)
      (let ((root (and dir (not (file-remote-p dir))
                       (when-let* ((found (locate-dominating-file dir ".git")))
                         (rdm-worktree--key found)))))
        (setq rdm-modeline--root-memo (cons dir root))
        root))))

(defun rdm-modeline--ensure (root)
  "The cache entry for ROOT, created empty when missing."
  (or (gethash root rdm-modeline--cache)
      (puthash root (rdm-modeline--entry-create) rdm-modeline--cache)))

;;;; The segment

;;;###autoload
(defun rdm-modeline-segment ()
  "\"ITEM · STATUS\" for the current buffer's rdm worktree, or \"\".
Never runs rdm: an uncached worktree renders \"\" and is filled once on
an idle timer."
  (if-let* ((root (rdm-modeline--root)))
      (let ((entry (gethash root rdm-modeline--cache)))
        (unless entry
          (setq entry (rdm-modeline--ensure root))
          (run-with-idle-timer rdm-modeline-idle-delay nil
                               #'rdm-modeline--fill-cold root))
        (or (rdm-modeline--entry-label entry) ""))
    ""))

;;;###autoload
(defun rdm-modeline-nano-segment ()
  "The rdm segment for nano-modeline: a leading space and the text, or \"\"."
  (let ((text (rdm-modeline-segment)))
    (if (string-empty-p text) "" (concat " " text))))

(defun rdm-modeline--mode-line ()
  "The rdm segment for `mode-line-misc-info', padded, or \"\"."
  (let ((text (rdm-modeline-segment)))
    (if (string-empty-p text) "" (concat " " text " "))))

(defconst rdm-modeline--mode-line-entry '(:eval (rdm-modeline--mode-line))
  "The `mode-line-misc-info' element `rdm-modeline-mode' adds.")

;;;; Filling

(defun rdm-modeline--label (id item data)
  "The segment text for worktree item ID (ITEM as a plist) shown as DATA."
  (let ((state (if (eq (plist-get item :kind) 'roadmap)
                   (let ((phases (append (plist-get data :phases) nil)))
                     (format "%d/%d"
                             (seq-count (lambda (p) (equal (plist-get p :status) "done"))
                                        phases)
                             (length phases)))
                 (or (plist-get data :status) "?"))))
    (propertize (format "%s · %s" id state)
                'face 'rdm-modeline
                'help-echo (format "rdm worktree for %s %s" (plist-get item :kind) id))))

(defun rdm-modeline--store (root entry generation item label)
  "Store ITEM and LABEL in ROOT's ENTRY unless a newer fill than GENERATION ran."
  (when (and (eq (gethash root rdm-modeline--cache) entry)
             (= generation (rdm-modeline--entry-generation entry)))
    (setf (rdm-modeline--entry-item entry) item
          (rdm-modeline--entry-label entry) label)
    (force-mode-line-update t)))

(defun rdm-modeline--buffer (root)
  "A live buffer whose checkout is ROOT, or nil."
  (seq-find (lambda (b)
              (equal (cdr (buffer-local-value 'rdm-modeline--root-memo b)) root))
            (buffer-list)))

(defun rdm-modeline--in-root (root fn)
  "Call FN with `default-directory' bound to ROOT, in a buffer of ROOT.
The buffer supplies the rdm context (its dir-locals and environment);
without one, a temporary buffer holding the global values is used."
  (if-let* ((buf (rdm-modeline--buffer root)))
      (with-current-buffer buf
        (let ((default-directory root))
          (funcall fn)))
    (with-temp-buffer
      (setq default-directory root)
      (funcall fn))))

(defun rdm-modeline--read (root entry generation action args callback)
  "Run ACTION with ARGS for ROOT, passing the result to CALLBACK.
Any error stores an empty label for ROOT's ENTRY at GENERATION."
  (let ((fail (lambda (_err) (rdm-modeline--store root entry generation nil ""))))
    (condition-case err
        (rdm-modeline--in-root root (lambda () (rdm-call action args callback fail)))
      (error (funcall fail err)))))

(defun rdm-modeline--fill (root)
  "Re-read ROOT's item and status into its cache entry, asynchronously."
  (let* ((entry (rdm-modeline--ensure root))
         (generation (cl-incf (rdm-modeline--entry-generation entry))))
    (rdm-modeline--read
     root entry generation 'worktree-current nil
     (lambda (current)
       (let* ((id (plist-get current :item))
              (item (rdm-item-parse-worktree-id id)))
         (if (null item)
             (rdm-modeline--store root entry generation nil "")
           (let ((show (rdm-item-show-args item)))
             (rdm-modeline--read
              root entry generation (car show) (append (cdr show) '(:no-body t))
              (lambda (data)
                (rdm-modeline--store root entry generation id
                                     (rdm-modeline--label id item data)))))))))))

(defun rdm-modeline--fill-cold (root)
  "Fill ROOT's entry unless a fill has already started."
  (when-let* ((entry (gethash root rdm-modeline--cache)))
    (when (zerop (rdm-modeline--entry-generation entry))
      (rdm-modeline--fill root))))

;;;; Refreshing

(defun rdm-modeline-refresh ()
  "Refresh the cached entry of the current buffer's checkout, if any.
On `after-save-hook' and `magit-post-refresh-hook'."
  (when-let* ((root (rdm-modeline--root)))
    (when (gethash root rdm-modeline--cache)
      (rdm-modeline--fill root))))

(defun rdm-modeline-refresh-all ()
  "Refresh every cached rdm worktree entry.  On `rdm-after-write-hook'."
  (let (roots)
    (maphash (lambda (root entry)
               (when (rdm-modeline--entry-item entry) (push root roots)))
             rdm-modeline--cache)
    (mapc #'rdm-modeline--fill roots)))

(defun rdm-modeline-refresh-root (dir)
  "Fill (or refresh) the entry of the worktree at DIR."
  (rdm-modeline--fill (rdm-worktree--key dir)))

(defun rdm-modeline-forget (dir)
  "Drop the cached entry of the worktree at DIR."
  (remhash (rdm-worktree--key dir) rdm-modeline--cache)
  (force-mode-line-update t))

(defun rdm-modeline--magit-hook (fn)
  "Apply FN (`add-hook' or `remove-hook') to `magit-post-refresh-hook'."
  (funcall fn 'magit-post-refresh-hook #'rdm-modeline-refresh))

(defun rdm-modeline--magit-loaded ()
  "Subscribe to magit's refresh hook once magit is loaded, if still enabled."
  (when (bound-and-true-p rdm-modeline-mode)
    (rdm-modeline--magit-hook #'add-hook)))

;;;###autoload
(define-minor-mode rdm-modeline-mode
  "Show the current rdm worktree's item and status in the mode line.
Adds `rdm-modeline-segment' to `mode-line-misc-info' and refreshes it on
`after-save-hook', `magit-post-refresh-hook' and `rdm-after-write-hook'."
  :global t
  :group 'rdm
  (clrhash rdm-modeline--cache)
  (if rdm-modeline-mode
      (progn
        (add-hook 'after-save-hook #'rdm-modeline-refresh)
        (add-hook 'rdm-after-write-hook #'rdm-modeline-refresh-all)
        (if (boundp 'magit-post-refresh-hook)
            (rdm-modeline--magit-hook #'add-hook)
          (with-eval-after-load 'magit-mode (rdm-modeline--magit-loaded)))
        (add-to-list 'mode-line-misc-info rdm-modeline--mode-line-entry))
    (remove-hook 'after-save-hook #'rdm-modeline-refresh)
    (remove-hook 'rdm-after-write-hook #'rdm-modeline-refresh-all)
    (when (boundp 'magit-post-refresh-hook)
      (rdm-modeline--magit-hook #'remove-hook))
    (setq mode-line-misc-info (delete rdm-modeline--mode-line-entry
                                      mode-line-misc-info)))
  (force-mode-line-update t))

(provide 'rdm-modeline)
;;; rdm-modeline.el ends here
