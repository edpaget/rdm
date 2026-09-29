;;; rdm-git.el --- Done: trailers in commit messages  -*- lexical-binding: t; -*-

;; This file is not part of GNU Emacs.

;;; Commentary:

;; `rdm-insert-done-line' picks an active phase or task (rdm's `--active'
;; group, from `rdm tree --active') and inserts `rdm hook done-line''s
;; output, verbatim, at the trailer position of a commit message: the end
;; of the message, before any `#' comment lines and the scissors line,
;; appended to an existing trailer block.  The current rdm worktree's item
;; (from `rdm worktree current') is the picker's default.  It works in any
;; buffer.
;;
;; In magit commit buffers `rdm-git-commit-mode' offers it: a message
;; names the key, `C-c C-x d' (or `gd' in evil's normal state, see
;; rdm-evil.el).  Nothing is inserted automatically.
;;
;; When the current branch is not the project's default branch, the line
;; is still inserted, with a warning: the post-commit hook only acts on
;; the default branch, the post-merge hook when the branch is merged, and
;; an item landed through rdm-land is marked done without a trailer.

;;; Code:

(require 'seq)
(require 'subr-x)
(require 'rdm-core)
(require 'rdm-view)
(require 'rdm-pick)

(defvar evil-local-mode)
(declare-function evil-normalize-keymaps "ext:evil-core" (&optional state))
(declare-function magit-toplevel "ext:magit-git" (&optional directory))

(defconst rdm-git-commit-key "C-c C-x d"
  "The key `rdm-git-commit-mode' binds to `rdm-insert-done-line'.")

(defconst rdm-git-commit-evil-key "gd"
  "The normal-state key rdm-evil.el binds in `rdm-git-commit-mode'.")

;;;; Candidates

(defun rdm-git--candidate (node item)
  "The candidate for tree NODE, an active phase or task, as ITEM.
Its text is the node's `ref', the shape `rdm worktree current' reports."
  (let ((status (plist-get node :status)))
    (rdm-pick--candidate (plist-get node :ref)
                         (rdm-item-with-refs (append item (list :status status))
                                             node)
                         status)))

(defun rdm-git-candidates ()
  "Candidates for every active phase and task, from one `rdm tree --active'.
rdm decides which items are active; every phase and task it returns is
offered."
  (let (out)
    (seq-doseq (node (plist-get (rdm-call-sync 'tree :active t) :children))
      (pcase (plist-get node :kind)
        ("roadmap"
         (let ((roadmap (plist-get node :name)))
           (seq-doseq (p (plist-get node :children))
             (push (rdm-git--candidate
                    p (list :kind 'phase :roadmap roadmap :stem (plist-get p :name)))
                   out))))
        ("task"
         (push (rdm-git--candidate node (list :kind 'task :slug (plist-get node :name)))
               out))))
    (nreverse out)))

(defun rdm-git-preselect (candidates current)
  "Order CANDIDATES for CURRENT, a `worktree current' reply or nil.
Return (CANDIDATES . DEFAULT).  A phase or task item whose `:ref' is a
candidate becomes DEFAULT.  A roadmap item moves the candidates whose
item names that roadmap first; when there is exactly one it becomes
DEFAULT."
  (let ((ref (plist-get current :ref)))
    (cond
     ((null ref) (cons candidates nil))
     ((member ref candidates)
      (cons candidates (car (member ref candidates))))
     ((equal (plist-get current :kind) "roadmap")
      (let* ((mine-p (lambda (c)
                       (equal (plist-get (rdm-item-from c) :roadmap)
                              (plist-get current :roadmap))))
             (mine (seq-filter mine-p candidates)))
        (cons (append mine (seq-remove mine-p candidates))
              (and (= (length mine) 1) (car mine)))))
     (t (cons candidates nil)))))

;;;; Inserting

(defconst rdm-git--trailer-re "\\`[A-Za-z0-9][A-Za-z0-9-]*: "
  "Matches a trailer line.")

(defun rdm-git--message-end ()
  "The end of the commit message: the first comment or scissors line."
  (save-excursion
    (goto-char (point-min))
    (let ((comment (string-trim (or comment-start "#"))))
      (if (and (not (string-empty-p comment))
               (re-search-forward (concat "^" (regexp-quote comment)) nil t))
          (match-beginning 0)
        (point-max)))))

(defun rdm-git--trailer-block-p (start end)
  "Non-nil when the last paragraph between START and END is a trailer block.
The first paragraph (the subject) never is."
  (save-excursion
    (goto-char end)
    (when (search-backward "\n\n" start t)
      (seq-every-p (lambda (l) (string-match-p rdm-git--trailer-re l))
                   (split-string (buffer-substring (+ (point) 2) end) "\n")))))

(defun rdm-git-insert-trailer (line)
  "Insert LINE (no trailing newline) at the commit message's trailer position.
An empty message keeps line 1 empty for the subject: LINE follows a
blank line, and point stays on line 1."
  (save-excursion
    (let* ((limit (rdm-git--message-end))
           (end (progn (goto-char limit)
                       (skip-chars-backward " \t\n")
                       (point)))
           (sep (cond ((= end (point-min)) "\n\n")
                      ((rdm-git--trailer-block-p (point-min) end) "\n")
                      (t "\n\n"))))
      (delete-region end limit)
      (goto-char end)
      (insert sep line "\n")
      (when (< (point) (point-max)) (insert "\n")))))

;;;; Branch check

(defun rdm-git--toplevel ()
  "The working tree to run rdm and git from."
  (or (and (fboundp 'magit-toplevel) (ignore-errors (magit-toplevel)))
      default-directory))

(defun rdm-git-current-branch ()
  "The current git branch, or nil when detached or not in a repository."
  (ignore-errors
    (car (process-lines "git" "symbolic-ref" "--short" "HEAD"))))

(defun rdm-git-check-branch (branch default)
  "Warn, without blocking, when BRANCH is not DEFAULT.  Return non-nil if so."
  (when (and branch default (not (equal branch default)))
    (display-warning
     'rdm
     (format "Committing on %s, not %s: the post-commit hook acts on Done: \
lines only on %s; the post-merge hook acts when %s is merged.  An item \
landed through rdm-land is marked done without a trailer."
             branch default default branch)
     :warning)
    t))

;;;; Command

;;;###autoload
(defun rdm-insert-done-line ()
  "Pick an active phase or task and insert its `Done:' line as a trailer.
The line is `rdm hook done-line''s output.  The current rdm worktree's
item is the default.  Warns when not on the default branch."
  (interactive)
  (let* ((buf (current-buffer))
         (default-directory (rdm-git--toplevel))
         (picked (rdm-git-preselect
                  (rdm-git-candidates)
                  (ignore-errors (rdm-worktree-item))))
         (item (rdm-pick-read "Done: " (car picked) (cdr picked) t))
         (line (string-trim-right
                (apply #'rdm-call-sync 'done-line (rdm-item-done-line-args item))
                "\n+"))
         (branch (rdm-git-current-branch))
         (default (plist-get (rdm-context) :default-branch)))
    (with-current-buffer buf
      (rdm-git-insert-trailer line))
    (rdm-git-check-branch branch default)
    line))

;;;; Commit-buffer offer

(defvar-keymap rdm-git-commit-mode-map
  :doc "Keymap of `rdm-git-commit-mode'."
  rdm-git-commit-key #'rdm-insert-done-line)

(define-minor-mode rdm-git-commit-mode
  "Offer `rdm-insert-done-line' in a commit message buffer."
  :lighter nil
  :keymap rdm-git-commit-mode-map)

(defun rdm-git-commit-offer-key ()
  "The key that inserts a Done: line in the current buffer.
The evil key only when it actually resolves to `rdm-insert-done-line'."
  (if (and (bound-and-true-p evil-local-mode)
           (eq (key-binding (kbd rdm-git-commit-evil-key))
               #'rdm-insert-done-line))
      rdm-git-commit-evil-key
    rdm-git-commit-key))

;;;###autoload
(defun rdm-git-commit-setup ()
  "Offer a Done: trailer in this commit buffer; insert nothing."
  ;; The evil bindings (rdm-evil.el requires this file, so not at top level).
  (when (featurep 'evil) (require 'rdm-evil))
  (rdm-git-commit-mode 1)
  (when (fboundp 'evil-normalize-keymaps) (evil-normalize-keymaps))
  (message "rdm: %s inserts a Done: line" (rdm-git-commit-offer-key)))

;;;###autoload
(with-eval-after-load 'git-commit
  (add-hook 'git-commit-setup-hook #'rdm-git-commit-setup))

(provide 'rdm-git)
;;; rdm-git.el ends here
